//! The direct path: build and sign Verifier intents without 1Click, for anyone to
//! submit with `execute_intents`.
//!
//! Use it as the fallback when 1Click is unavailable (move the USDC to a NEAR account
//! with `ft_withdraw`, or to another Verifier account with `transfer`) and for local
//! tests against the Verifier code in a sandbox. It cannot swap to another chain by
//! itself: that needs a solver, which only 1Click (or the solver bus) provides.
//!
//! Nonce and deadline rules (see [`crate::nonce`]): the deadline must be in the future
//! when the transaction runs; with a versioned nonce the nonce deadline must not be
//! earlier than the intent deadline and the salt must be valid. Each nonce works once
//! for each signer.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::http::{HttpRequest, HttpResponse, parse_json};
use crate::nonce::{Nonce, Salt};
use crate::payload::{DecodedPayload, FtWithdraw, Intent, SignedPayload, Standard, Transfer, UnsignedPayload};
use crate::signer::IntentSigner;
use crate::time::Time;

/// The gas the sandbox tests attach to `execute_intents` (one `ft_withdraw` burnt
/// about 11 Tgas there). The Verifier needs no deposit for it.
pub const EXECUTE_INTENTS_GAS: u64 = 300_000_000_000_000;

/// An intent list that the signer will sign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentDraft {
    pub signer_id: String,
    pub verifying_contract: String,
    pub deadline: Time,
    pub nonce: Nonce,
    pub intents: Vec<Intent>,
}

impl IntentDraft {
    pub fn new(
        signer_id: impl Into<String>,
        verifying_contract: impl Into<String>,
        deadline: Time,
        nonce: Nonce,
    ) -> Self {
        Self {
            signer_id: signer_id.into(),
            verifying_contract: verifying_contract.into(),
            deadline,
            nonce,
            intents: Vec::new(),
        }
    }

    /// Adds a `transfer` of one token (id with prefix, e.g. `nep141:usdc.near`).
    pub fn transfer(mut self, receiver_id: &str, token_id: &str, amount: u128) -> Self {
        self.intents.push(Intent::Transfer(Transfer {
            receiver_id: receiver_id.into(),
            tokens: [(token_id.to_owned(), amount.to_string())].into(),
            memo: None,
        }));
        self
    }

    /// Adds an `ft_withdraw` of a NEP-141 token (contract id **without** prefix) to a
    /// NEAR account, with `ft_transfer` (refunded if it fails).
    pub fn ft_withdraw(mut self, token: &str, receiver_id: &str, amount: u128) -> Self {
        self.intents.push(Intent::FtWithdraw(FtWithdraw {
            token: token.into(),
            receiver_id: receiver_id.into(),
            amount: amount.to_string(),
            memo: None,
        }));
        self
    }

    /// Adds an `ft_withdraw` with a memo. Bridge tokens (`*.omft.near`) read the memo
    /// `WITHDRAW_TO:<address>` to release on their own chain when `receiver_id` is the
    /// token contract; this crate has not tested that on mainnet.
    pub fn ft_withdraw_with_memo(
        mut self,
        token: &str,
        receiver_id: &str,
        amount: u128,
        memo: &str,
    ) -> Self {
        self.intents.push(Intent::FtWithdraw(FtWithdraw {
            token: token.into(),
            receiver_id: receiver_id.into(),
            amount: amount.to_string(),
            memo: Some(memo.into()),
        }));
        self
    }

    /// Checks the draft before signing: at least one intent, a deadline after `now`,
    /// and a versioned nonce whose deadline is not before the intent deadline.
    pub fn validate(&self, now: Time) -> Result<()> {
        if self.intents.is_empty() {
            return Err(Error::Invalid("no intents".into()));
        }
        if self.deadline <= now {
            return Err(Error::Invalid("deadline is not in the future".into()));
        }
        if let Some(v) = self.nonce.as_versioned()
            && v.deadline < self.deadline
        {
            return Err(Error::Invalid(
                "the nonce deadline is before the intent deadline".into(),
            ));
        }
        for i in &self.intents {
            match i {
                Intent::Transfer(t) => {
                    if t.receiver_id == self.signer_id {
                        return Err(Error::Invalid("transfer to the signer itself".into()));
                    }
                    if t.tokens.is_empty() {
                        return Err(Error::Invalid("transfer of no tokens".into()));
                    }
                }
                Intent::FtWithdraw(w) => {
                    if w.token.contains(':') {
                        return Err(Error::Invalid(
                            "ft_withdraw takes the token contract id without a prefix".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// The payload to sign, in the given standard.
    pub fn to_unsigned(&self, standard: Standard, now: Time) -> Result<UnsignedPayload> {
        self.validate(now)?;
        let d = DecodedPayload {
            signer_id: self.signer_id.clone(),
            verifying_contract: self.verifying_contract.clone(),
            deadline: self.deadline,
            nonce: self.nonce,
            intents: self.intents.clone(),
        };
        Ok(UnsignedPayload::from_parts(&d, standard))
    }

    /// Validates and signs. The signer must be the draft's signer.
    pub fn sign(
        &self,
        signer: &IntentSigner,
        standard: Standard,
        now: Time,
    ) -> Result<SignedPayload> {
        if signer.account_id() != self.signer_id {
            return Err(Error::Invalid(format!(
                "draft signer {} is not the key's account {}",
                self.signer_id,
                signer.account_id()
            )));
        }
        signer.sign(&self.to_unsigned(standard, now)?)
    }
}

/// Arguments of `execute_intents` (and `simulate_intents`): `{"signed": [...]}`.
pub fn execute_intents_args(signed: &[SignedPayload]) -> Value {
    json!({ "signed": signed.iter().map(SignedPayload::to_json).collect::<Vec<_>>() })
}

/// A NEAR JSON-RPC view call (`query` / `call_function` at final finality).
pub fn view_call_request(rpc_url: &str, contract: &str, method: &str, args: &Value) -> HttpRequest {
    let body = json!({
        "jsonrpc": "2.0",
        "id": "zecret-intents",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": contract,
            "method_name": method,
            "args_base64": STANDARD.encode(args.to_string()),
        }
    });
    HttpRequest::post_json(rpc_url, &body)
}

/// Parses the JSON result of a view call.
pub fn parse_view_result<T: DeserializeOwned>(r: &HttpResponse) -> Result<T> {
    if !(200..300).contains(&r.status) {
        return Err(Error::Rpc(format!(
            "HTTP {}: {}",
            r.status,
            String::from_utf8_lossy(&r.body[..r.body.len().min(300)])
        )));
    }
    let v: Value = parse_json(&r.body, "NEAR RPC")?;
    if let Some(e) = v.get("error") {
        return Err(Error::Rpc(e.to_string()));
    }
    let result = v
        .get("result")
        .ok_or_else(|| Error::Parse("NEAR RPC answer has no result".into()))?;
    if let Some(e) = result.get("error") {
        return Err(Error::Rpc(e.to_string()));
    }
    let bytes: Vec<u8> = result
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Parse("view result has no bytes".into()))?
        .iter()
        .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
        .collect::<Option<_>>()
        .ok_or_else(|| Error::Parse("view result bytes are not bytes".into()))?;
    parse_json(&bytes, "view result")
}

/// `current_salt` on the Verifier: the salt for new versioned nonces.
pub fn current_salt_request(rpc_url: &str, verifier: &str) -> HttpRequest {
    view_call_request(rpc_url, verifier, "current_salt", &json!({}))
}

pub fn parse_current_salt(r: &HttpResponse) -> Result<Salt> {
    let s: String = parse_view_result(r)?;
    Salt::from_hex(&s)
}

/// `mt_balance_of(account_id, token_id)`: a balance inside the Verifier.
pub fn balance_request(rpc_url: &str, verifier: &str, account_id: &str, token_id: &str) -> HttpRequest {
    view_call_request(
        rpc_url,
        verifier,
        "mt_balance_of",
        &json!({ "account_id": account_id, "token_id": token_id }),
    )
}

pub fn parse_balance(r: &HttpResponse) -> Result<u128> {
    let s: String = parse_view_result(r)?;
    crate::oneclick::amount(&s, "mt_balance_of")
}

/// `is_nonce_used(account_id, nonce)`.
pub fn nonce_used_request(rpc_url: &str, verifier: &str, account_id: &str, nonce: &Nonce) -> HttpRequest {
    view_call_request(
        rpc_url,
        verifier,
        "is_nonce_used",
        &json!({ "account_id": account_id, "nonce": nonce.to_base64() }),
    )
}

pub fn parse_nonce_used(r: &HttpResponse) -> Result<bool> {
    parse_view_result(r)
}

/// `simulate_intents(signed)`: runs the intents without changing state. Fails (as an
/// RPC error that carries the Verifier's panic message) if they would fail.
pub fn simulate_request(rpc_url: &str, verifier: &str, signed: &[SignedPayload]) -> HttpRequest {
    view_call_request(rpc_url, verifier, "simulate_intents", &execute_intents_args(signed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time;

    fn signer() -> IntentSigner {
        IntentSigner::from_seed(&[7u8; 32])
    }

    fn now() -> Time {
        time::parse("2026-11-13T03:00:00.000Z").unwrap()
    }

    fn draft() -> IntentDraft {
        let deadline = time::parse("2026-11-13T04:00:00.000Z").unwrap();
        let nonce = Nonce::versioned(Salt::from_hex("252812b3").unwrap(), deadline, [3u8; 15]).unwrap();
        IntentDraft::new(signer().account_id(), "intents.near", deadline, nonce)
    }

    #[test]
    fn raw_ft_withdraw_payload_is_exact() {
        let s = signer();
        let signed = draft()
            .ft_withdraw("usdc.near", "bob.near", 1_000_000)
            .sign(&s, Standard::RawEd25519, now())
            .unwrap();
        let j = signed.to_json();
        assert_eq!(j["standard"], "raw_ed25519");
        let expected = format!(
            r#"{{"signer_id":"{}","verifying_contract":"intents.near","deadline":"2026-11-13T04:00:00.000Z","nonce":"{}","intents":[{{"intent":"ft_withdraw","token":"usdc.near","receiver_id":"bob.near","amount":"1000000"}}]}}"#,
            s.account_id(),
            draft().nonce.to_base64()
        );
        assert_eq!(j["payload"].as_str().unwrap(), expected);
        assert_eq!(j["public_key"], s.public_key_str());
        crate::signer::verify(&signed).unwrap();
        let d = signed.unsigned().decode().unwrap();
        assert_eq!(d.signer_id, s.account_id());
        assert_eq!(d.intents.len(), 1);
    }

    #[test]
    fn nep413_transfer_payload_shape() {
        let s = signer();
        let signed = draft()
            .transfer("bob.near", "nep141:usdc.near", 5)
            .sign(&s, Standard::Nep413, now())
            .unwrap();
        let j = signed.to_json();
        assert_eq!(j["standard"], "nep413");
        assert_eq!(j["payload"]["recipient"], "intents.near");
        assert_eq!(j["payload"]["nonce"], draft().nonce.to_base64());
        let msg: Value = serde_json::from_str(j["payload"]["message"].as_str().unwrap()).unwrap();
        assert_eq!(
            msg,
            json!({
                "signer_id": s.account_id(),
                "deadline": "2026-11-13T04:00:00.000Z",
                "intents": [{"intent":"transfer","receiver_id":"bob.near","tokens":{"nep141:usdc.near":"5"}}],
            })
        );
        assert!(j["payload"].get("callbackUrl").is_none());
        crate::signer::verify(&signed).unwrap();
        // The NEP-413 intent hash is the signed prehash.
        let hash = signed.unsigned().intent_hash().unwrap();
        assert_eq!(signed.unsigned().bytes_to_sign().unwrap(), hash.to_vec());
    }

    #[test]
    fn validation_refusals() {
        let s = signer();
        let n = now();
        assert!(draft().sign(&s, Standard::RawEd25519, n).is_err(), "no intents");
        let late = time::parse("2026-11-13T05:00:00.000Z").unwrap();
        assert!(
            draft()
                .transfer("bob.near", "nep141:t", 1)
                .sign(&s, Standard::RawEd25519, late)
                .is_err(),
            "expired"
        );
        let mut d = draft().transfer("bob.near", "nep141:t", 1);
        d.deadline = late;
        assert!(d.sign(&s, Standard::RawEd25519, n).is_err(), "nonce deadline earlier");
        assert!(
            draft()
                .transfer(&s.account_id(), "nep141:t", 1)
                .sign(&s, Standard::RawEd25519, n)
                .is_err(),
            "self transfer"
        );
        assert!(
            draft()
                .ft_withdraw("nep141:usdc.near", "bob.near", 1)
                .sign(&s, Standard::RawEd25519, n)
                .is_err(),
            "prefixed withdraw token"
        );
        let other = IntentSigner::from_seed(&[8u8; 32]);
        assert!(
            draft()
                .transfer("bob.near", "nep141:t", 1)
                .sign(&other, Standard::RawEd25519, n)
                .is_err(),
            "wrong key"
        );
        // A legacy nonce is allowed.
        let mut d = draft().transfer("bob.near", "nep141:t", 1);
        d.nonce = Nonce::legacy([9u8; 32]).unwrap();
        d.sign(&s, Standard::RawEd25519, n).unwrap();
    }

    #[test]
    fn execute_args_and_rpc_requests() {
        let s = signer();
        let signed = draft()
            .ft_withdraw("usdc.near", "bob.near", 1)
            .sign(&s, Standard::RawEd25519, now())
            .unwrap();
        let args = execute_intents_args(std::slice::from_ref(&signed));
        assert_eq!(args["signed"][0], signed.to_json());

        let r = balance_request("https://rpc.example", "intents.near", "abc", "nep141:usdc.near");
        let b = r.json_body().unwrap();
        assert_eq!(b["method"], "query");
        assert_eq!(b["params"]["method_name"], "mt_balance_of");
        let decoded = STANDARD
            .decode(b["params"]["args_base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&decoded).unwrap(),
            json!({"account_id":"abc","token_id":"nep141:usdc.near"})
        );
        let sim = simulate_request("https://rpc.example", "intents.near", &[signed]);
        assert_eq!(sim.json_body().unwrap()["params"]["method_name"], "simulate_intents");
        let n = nonce_used_request("https://rpc.example", "intents.near", "abc", &draft().nonce);
        assert_eq!(n.json_body().unwrap()["params"]["method_name"], "is_nonce_used");
    }

    #[test]
    fn view_result_parsing() {
        // The live answer of current_salt on intents.near, 2026-11-13.
        let ok = HttpResponse::new(
            200,
            r#"{"jsonrpc":"2.0","result":{"block_hash":"x","block_height":1,"logs":[],"result":[34,50,53,50,56,49,50,98,51,34]},"id":"zecret-intents"}"#,
        );
        assert_eq!(parse_current_salt(&ok).unwrap().to_hex(), "252812b3");
        let bal = HttpResponse::new(
            200,
            r#"{"jsonrpc":"2.0","result":{"logs":[],"result":[34,53,48,48,48,48,48,48,34]},"id":1}"#,
        );
        assert_eq!(parse_balance(&bal).unwrap(), 5_000_000);
        let used = HttpResponse::new(200, r#"{"result":{"result":[102,97,108,115,101]}}"#);
        assert!(!parse_nonce_used(&used).unwrap());
        let call_err = HttpResponse::new(
            200,
            r#"{"jsonrpc":"2.0","result":{"error":"wasm execution failed with error: Smart contract panicked: insufficient balance","logs":[]},"id":1}"#,
        );
        assert!(matches!(parse_balance(&call_err), Err(Error::Rpc(m)) if m.contains("insufficient")));
        let rpc_err = HttpResponse::new(200, r#"{"jsonrpc":"2.0","error":{"code":-32000},"id":1}"#);
        assert!(matches!(parse_balance(&rpc_err), Err(Error::Rpc(_))));
        assert!(matches!(
            parse_balance(&HttpResponse::new(502, "bad gateway")),
            Err(Error::Rpc(_))
        ));
    }
}
