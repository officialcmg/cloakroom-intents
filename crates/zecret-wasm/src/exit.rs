//! The exit from NEAR Intents, in steps the worker can schedule and persist between.
//!
//! **1Click** (mainnet): [`oneclick_quote`] gets a firm quote and checks it against what
//! the user approved; [`oneclick_submit`] asks 1Click for the payload, checks it, signs it
//! and submits it; [`oneclick_status`] reads the delivery status. Nothing is signed unless
//! every check of `zecret_intents::check` passes (docs/INTENTS.md §3).
//!
//! **Direct** (local and testnet, where 1Click cannot see our stand-in Verifier):
//! [`direct_prepare`] fixes the deadline and nonce of one `ft_withdraw` intent that moves
//! the taker's whole balance of the swap token to a NEAR account the user named;
//! [`direct_relay`] signs it (the signature is deterministic, so a resubmission after a
//! reload sends the same bytes) and hands it to the maker's relay, which sends it with
//! `execute_intents` and pays the gas. The taker's own account cannot pay for it (see
//! [`DIRECT_EXIT_GAS`]); [`direct_submit_with_gas`] is that path, kept for the
//! measurement. A nonce works once, so the intent executes at most once.
//!
//! Errors carry the website's error codes (web/ENGINE_API.md).

use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use zecret_core::{
    config::RpcUrls,
    near::{tx::Action, At, NearKey, NearRpc, Signer, TGAS},
    net::SharedTransport,
};
use zecret_intents::{
    check::{check_generated_intent, check_quote, Policy, Refusal},
    client::VerifierRpc,
    direct::{execute_intents_args, IntentDraft},
    oneclick::{self, ExitQuoteParams, OneClick, QuoteRequest, QuoteResponse, SwapStatus},
    time::{self, Time},
    ApiErrorKind, ApprovedExit, Error, HttpTransport, IntentSigner, Nonce, Standard,
};

use crate::transport::CorePost;

/// An exit error with the website's code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExitError {
    pub code: &'static str,
    pub message: String,
    pub transient: bool,
    /// For `EXIT_BELOW_MINIMUM`: the firm quote the user may accept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offered: Option<Offered>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offered {
    pub amount_out: String,
    pub min_amount_out: String,
}

impl ExitError {
    fn new(code: &'static str, message: impl Into<String>, transient: bool) -> Self {
        Self { code, message: message.into(), transient, offered: None }
    }
}

/// Maps an error of `zecret-intents` to the website's codes.
pub fn classify(e: &Error) -> ExitError {
    match e {
        Error::Transport(m) => ExitError::new("NETWORK", format!("NEAR Intents could not be reached: {m}"), true),
        Error::Api(a) => match &a.kind {
            ApiErrorKind::NoQuote => ExitError::new("EXIT_NO_QUOTE", "NEAR Intents has no price for this pair right now. The engine asks again in 20 seconds.", true),
            ApiErrorKind::AmountTooLow { .. } => ExitError::new("EXIT_AMOUNT_TOO_LOW", format!("NEAR Intents: {}", a.message), false),
            ApiErrorKind::Server | ApiErrorKind::RateLimited => ExitError::new("NETWORK", format!("NEAR Intents answered {}: {}", a.status, a.message), true),
            _ => ExitError::new("EXIT_FAILED", format!("NEAR Intents answered {}: {}", a.status, a.message), false),
        },
        Error::Refused(r) => refused(r),
        Error::Rpc(m) => ExitError::new("NETWORK", format!("NEAR RPC: {m}"), true),
        Error::Parse(m) | Error::Invalid(m) => ExitError::new("EXIT_REFUSED", format!("Nothing was signed: {m}"), false),
    }
}

fn refused(r: &Refusal) -> ExitError {
    ExitError::new("EXIT_REFUSED", format!("The safety check refused the exit, so nothing was signed: {r}"), false)
}

fn time_ms(ms: i64) -> Result<Time, ExitError> {
    time::from_unix_millis(ms).map_err(|e| ExitError::new("INTERNAL", e.to_string(), false))
}

/// What the user approved, as the worker stores it (amounts as decimal strings).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub signer_id: String,
    pub verifying_contract: String,
    pub origin_asset: String,
    pub amount_in: String,
    pub destination_asset: String,
    pub recipient: String,
    pub min_amount_out: String,
}

impl Approval {
    pub fn to_approved(&self) -> Result<ApprovedExit, ExitError> {
        let n = |s: &str, what: &str| s.parse::<u128>().map_err(|_| ExitError::new("INVALID_PARAMS", format!("{what} must be a whole number"), false));
        Ok(ApprovedExit {
            signer_id: self.signer_id.clone(),
            verifying_contract: self.verifying_contract.clone(),
            origin_asset: self.origin_asset.clone(),
            amount_in: n(&self.amount_in, "amountIn")?,
            destination_asset: self.destination_asset.clone(),
            recipient: self.recipient.clone(),
            min_amount_out: n(&self.min_amount_out, "minAmountOut")?,
        })
    }
}

/// The parts of a firm quote the page shows, and the raw answer for the next step.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmQuote {
    pub amount_in: String,
    pub amount_out: String,
    pub min_amount_out: String,
    pub deposit_address: String,
    pub time_estimate_s: Option<f64>,
    pub fee_bps: u32,
    pub raw: Value,
}

fn summary(q: &QuoteResponse) -> FirmQuote {
    FirmQuote {
        amount_in: q.quote.amount_in.clone(),
        amount_out: q.quote.amount_out.clone(),
        min_amount_out: q.quote.min_amount_out.clone(),
        deposit_address: q.quote.deposit_address.clone().unwrap_or_default(),
        time_estimate_s: q.quote.time_estimate,
        fee_bps: q.quote_request.app_fee_bps(),
        raw: q.raw.clone(),
    }
}

/// Step 1 of the 1Click exit: a firm quote (100 bp slippage, request deadline in 30
/// minutes, refunds to the taker's Verifier account), checked against the approval. A
/// firm minimum below the approved one is `EXIT_BELOW_MINIMUM` with the offer.
pub fn oneclick_quote<T: HttpTransport>(t: &T, api: &OneClick, approval: &Approval, now_ms: i64) -> Result<FirmQuote, ExitError> {
    let approved = approval.to_approved()?;
    let now = time_ms(now_ms)?;
    let params = ExitQuoteParams {
        signer_id: approved.signer_id.clone(),
        origin_asset: approved.origin_asset.clone(),
        amount: approved.amount_in,
        destination_asset: approved.destination_asset.clone(),
        recipient: approved.recipient.clone(),
        slippage_bps: 100,
        deadline: now + Duration::minutes(30),
        dry: false,
        referral: None,
        quote_waiting_time_ms: None,
    };
    let req = api.quote_request(&QuoteRequest::exit(&params));
    let r = t.send(&req).map_err(|m| classify(&Error::Transport(m)))?;
    let q = oneclick::parse_quote(&r).map_err(|e| classify(&e))?;
    match check_quote(&approved, &q, &Policy::default(), now) {
        Ok(()) => Ok(summary(&q)),
        Err(Refusal::MinimumTooLow { approved: min, quoted }) => Err(ExitError {
            code: "EXIT_BELOW_MINIMUM",
            message: format!("The firm price for the delivery guarantees {quoted}, below the approved minimum {min}. Nothing was signed."),
            transient: false,
            offered: Some(Offered { amount_out: q.quote.amount_out.clone(), min_amount_out: q.quote.min_amount_out.clone() }),
        }),
        Err(r) => Err(refused(&r)),
    }
}

/// The result of a submitted 1Click exit.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submitted {
    pub intent_hash: String,
    pub deposit_address: String,
}

/// Step 2 of the 1Click exit: generate the payload for the quote of step 1, check it,
/// sign it with the taker key and submit it. The quote is checked again first.
pub fn oneclick_submit<T: HttpTransport>(t: &T, api: &OneClick, seed: &[u8; 32], approval: &Approval, quote_raw: &Value, now_ms: i64) -> Result<Submitted, ExitError> {
    let approved = approval.to_approved()?;
    let now = time_ms(now_ms)?;
    let signer = IntentSigner::from_seed(seed);
    if signer.account_id() != approved.signer_id {
        return Err(ExitError::new("INTERNAL", "the stored NEAR key does not own the payout account", false));
    }
    let q = QuoteResponse::from_value(quote_raw.clone()).map_err(|e| classify(&e))?;
    check_quote(&approved, &q, &Policy::default(), now).map_err(|r| refused(&r))?;
    let deposit = q.quote.deposit_address.clone().unwrap_or_default();
    let send = |req| t.send(&req).map_err(|m| classify(&Error::Transport(m)));
    let r = send(api.generate_intent_request(&deposit, &approved.signer_id, Standard::RawEd25519))?;
    let generated = oneclick::parse_generated_intent(&r).map_err(|e| classify(&e))?;
    let checked = check_generated_intent(&approved, &q, &generated, Standard::RawEd25519, &Policy::default(), now).map_err(|r| refused(&r))?;
    let signed = signer.sign(&checked.payload).map_err(|e| classify(&e))?;
    let r = send(api.submit_intent_request(&signed))?;
    let submitted = oneclick::parse_submit_intent(&r).map_err(|e| classify(&e))?;
    if submitted.intent_hash != checked.intent_hash {
        return Err(ExitError::new("EXIT_FAILED", format!("submit-intent returned hash {}, the signed intent has {}", submitted.intent_hash, checked.intent_hash), false));
    }
    Ok(Submitted { intent_hash: checked.intent_hash, deposit_address: deposit })
}

/// One `/v0/status` reading.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `SUCCESS`, `REFUNDED`, `FAILED`, `PROCESSING`, ...
    pub status: String,
    pub is_final: bool,
    pub amount_out: Option<String>,
    pub destination_txs: Vec<Value>,
    pub refunded_amount: Option<String>,
}

pub fn oneclick_status<T: HttpTransport>(t: &T, api: &OneClick, deposit_address: &str) -> Result<Status, ExitError> {
    let r = t.send(&api.status_request(deposit_address, None)).map_err(|m| classify(&Error::Transport(m)))?;
    let s = oneclick::parse_status(&r).map_err(|e| classify(&e))?;
    let status = match &s.status {
        SwapStatus::KnownDepositTx => "KNOWN_DEPOSIT_TX".to_string(),
        SwapStatus::PendingDeposit => "PENDING_DEPOSIT".to_string(),
        SwapStatus::IncompleteDeposit => "INCOMPLETE_DEPOSIT".to_string(),
        SwapStatus::Processing => "PROCESSING".to_string(),
        SwapStatus::Success => "SUCCESS".to_string(),
        SwapStatus::Refunded => "REFUNDED".to_string(),
        SwapStatus::Failed => "FAILED".to_string(),
        SwapStatus::Unknown(x) => x.clone(),
    };
    Ok(Status {
        is_final: s.status.is_final(),
        status,
        amount_out: s.swap_details.amount_out.clone(),
        destination_txs: s.swap_details.destination_chain_tx_hashes.iter().map(|d| json!({"hash": d.hash, "url": d.explorer_url})).collect(),
        refunded_amount: s.swap_details.refunded_amount.clone(),
    })
}

// ---------------------------------------------------------------- the direct exit

/// Gas `execute_intents` with one `ft_withdraw` needs attached. Measured with the live
/// `intents.near` code and `wrap.near` in the sandbox (`tests/sandbox.rs`, `gas_sweep`): it
/// fails with "Exceeded the prepaid gas" at 35 Tgas and works at 40 Tgas, costing about
/// 0.001 NEAR. A signer must hold the attached gas at the pessimistic price (about
/// 0.031 NEAR for 30 Tgas, measured), so the taker's 0.0281 NEAR left of the stipend
/// cannot send it: the maker's relay does ([`direct_relay`]).
pub const DIRECT_EXIT_GAS: u64 = 40 * TGAS;

/// One planned `ft_withdraw` from the taker's Verifier account to a NEAR account. Stored
/// by the worker before anything is signed or sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectPlan {
    pub signer_id: String,
    pub verifier: String,
    /// The token contract id, without the `nep141:` prefix.
    pub token: String,
    pub receiver: String,
    pub amount: String,
    pub deadline_ms: i64,
    /// Base64 of the versioned nonce (its deadline equals `deadline_ms`).
    pub nonce: String,
    /// Base58 hash of the signed payload.
    pub intent_hash: String,
}

impl DirectPlan {
    fn draft(&self) -> Result<IntentDraft, ExitError> {
        let amount: u128 = self.amount.parse().map_err(|_| ExitError::new("INTERNAL", "plan amount is not a number", false))?;
        let nonce = Nonce::from_base64(&self.nonce).map_err(|e| classify(&e))?;
        Ok(IntentDraft::new(&self.signer_id, &self.verifier, time_ms(self.deadline_ms)?, nonce).ft_withdraw(&self.token, &self.receiver, amount))
    }
}

/// The Verifier's views go to the first NEAR endpoint only (the `zecret-intents` client
/// takes one URL); an exit that fails there is tried again by the page.
fn verifier_rpc(t: &SharedTransport, rpc_url: &str, verifier: &str) -> VerifierRpc<CorePost> {
    VerifierRpc { transport: CorePost(t.clone()), rpc_url: rpc_urls(rpc_url).primary().into(), verifier: verifier.into() }
}

/// The page passes the NEAR endpoints as one URL or as a JSON array of URLs.
pub fn rpc_urls(s: &str) -> RpcUrls {
    let s = s.trim();
    if s.starts_with('[')
        && let Ok(u) = serde_json::from_str::<RpcUrls>(s)
    {
        return u;
    }
    RpcUrls::from(s)
}

fn near_rpc(t: &SharedTransport, rpc_url: &str) -> NearRpc {
    NearRpc::with_urls(t.clone(), &rpc_urls(rpc_url))
}

/// A NEAR account id as the protocol accepts it (2 to 64 characters of lower-case
/// letters, digits and `-_.`, no leading, trailing or double separators).
pub fn valid_near_account(id: &str) -> bool {
    let b = id.as_bytes();
    if b.len() < 2 || b.len() > 64 {
        return false;
    }
    let sep = |c: u8| matches!(c, b'-' | b'_' | b'.');
    if sep(b[0]) || sep(b[b.len() - 1]) {
        return false;
    }
    b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || sep(*c)) && !b.windows(2).any(|w| sep(w[0]) && sep(w[1]))
}

/// Plans the direct exit of the taker's whole balance of `token` to `receiver`. Reads the
/// balance and the Verifier's current nonce salt; checks that the receiver exists and can
/// hold the token (a NEP-141 token pays only registered accounts). Signs nothing.
pub fn direct_prepare(t: &SharedTransport, rpc_url: &str, verifier: &str, token: &str, seed: &[u8; 32], receiver: &str, now_ms: i64) -> Result<DirectPlan, ExitError> {
    if !valid_near_account(receiver) {
        return Err(ExitError::new("INVALID_ADDRESS", format!("{receiver:?} is not a NEAR account id"), false));
    }
    let signer = IntentSigner::from_seed(seed);
    let signer_id = signer.account_id();
    let v = verifier_rpc(t, rpc_url, verifier);
    let balance = v.balance(&signer_id, &format!("nep141:{token}")).map_err(|e| classify(&e))?;
    if balance == 0 {
        return Err(ExitError::new("EXIT_FAILED", format!("the taker's Verifier account holds no {token}"), false));
    }
    let near = near_rpc(t, rpc_url);
    let nerr = |e: anyhow::Error| ExitError::new("NETWORK", format!("NEAR RPC: {e:#}"), true);
    if near.account(receiver, &At::Final).map_err(nerr)?.is_none() {
        return Err(ExitError::new("EXIT_REFUSED", format!("the NEAR account {receiver} does not exist; nothing was signed"), false));
    }
    let reg = near.view(token, "storage_balance_of", &json!({"account_id": receiver}), &At::Final).map_err(nerr)?;
    if reg.is_null() {
        return Err(ExitError::new("EXIT_REFUSED", format!("the NEAR account {receiver} is not registered with the token {token} (storage_deposit), so it cannot receive it; nothing was signed"), false));
    }
    let salt = v.current_salt().map_err(|e| classify(&e))?;
    let deadline = time_ms(now_ms)? + Duration::hours(1);
    // Millisecond precision, so that the stored plan rebuilds the same payload.
    let deadline_ms = deadline.timestamp_millis();
    let deadline = time_ms(deadline_ms)?;
    let nonce = Nonce::versioned_random(salt, deadline).map_err(|e| classify(&e))?;
    let mut plan = DirectPlan {
        signer_id,
        verifier: verifier.into(),
        token: token.into(),
        receiver: receiver.into(),
        amount: balance.to_string(),
        deadline_ms,
        nonce: nonce.to_base64(),
        intent_hash: String::new(),
    };
    let signed = plan.draft()?.sign(&signer, Standard::RawEd25519, time_ms(now_ms)?).map_err(|e| classify(&e))?;
    plan.intent_hash = signed.intent_hash_base58().map_err(|e| classify(&e))?;
    Ok(plan)
}

/// What a direct exit did.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectResult {
    /// The `execute_intents` transaction, or `None` if the nonce was already used (an
    /// earlier submission of the same plan executed).
    pub tx_hash: Option<String>,
    pub delivered: String,
    /// yoctoNEAR the taker's NEAR account held before and after.
    pub near_before: String,
    pub near_after: String,
}

/// Signs the planned intent and sends it with `execute_intents` from the taker's own NEAR
/// account with `gas` attached. Not used by the engine: the rest of the stipend cannot pay
/// for the gas `ft_withdraw` needs (see [`DIRECT_EXIT_GAS`]); kept to measure that.
/// Simulates first, so a payload the Verifier would refuse costs no gas.
pub fn direct_submit_with_gas(t: &SharedTransport, rpc_url: &str, seed: &[u8; 32], plan: &DirectPlan, now_ms: i64, gas: u64) -> Result<DirectResult, ExitError> {
    let signer = IntentSigner::from_seed(seed);
    if signer.account_id() != plan.signer_id {
        return Err(ExitError::new("INTERNAL", "the stored NEAR key does not own the payout account", false));
    }
    let near = near_rpc(t, rpc_url);
    let nerr = |e: anyhow::Error| ExitError::new("NETWORK", format!("NEAR RPC: {e:#}"), true);
    let before = near.account(&plan.signer_id, &At::Final).map_err(nerr)?.map(|a| a.amount).unwrap_or(0);
    let v = verifier_rpc(t, rpc_url, &plan.verifier);
    let nonce = Nonce::from_base64(&plan.nonce).map_err(|e| classify(&e))?;
    if v.nonce_used(&plan.signer_id, &nonce).map_err(|e| classify(&e))? {
        return Ok(DirectResult { tx_hash: None, delivered: plan.amount.clone(), near_before: before.to_string(), near_after: before.to_string() });
    }
    if now_ms >= plan.deadline_ms {
        return Err(ExitError::new("EXIT_FAILED", "the signed exit expired before it was sent; a new one is needed", false));
    }
    let signed = plan.draft()?.sign(&signer, Standard::RawEd25519, time_ms(now_ms)?).map_err(|e| classify(&e))?;
    if signed.intent_hash_base58().map_err(|e| classify(&e))? != plan.intent_hash {
        return Err(ExitError::new("INTERNAL", "the stored plan does not rebuild the same intent", false));
    }
    v.simulate(std::slice::from_ref(&signed)).map_err(|e| match e {
        Error::Transport(_) => classify(&e),
        other => ExitError::new("EXIT_FAILED", format!("the Verifier would refuse the exit: {other}"), false),
    })?;
    let key = NearKey::from_seed(*seed);
    let who = Signer::new(&plan.signer_id, key);
    let action = Action::call("execute_intents", &execute_intents_args(&[signed]), gas, 0);
    let out = near.transact(&who, &plan.verifier, vec![action]).map_err(|e| {
        let m = format!("{e:#}");
        if m.contains("NotEnoughBalance") || m.contains("LackBalanceForState") {
            ExitError::new("EXIT_FAILED", format!("the taker's NEAR account cannot pay for execute_intents: {m}"), false)
        } else {
            nerr(e)
        }
    })?;
    if let Some(f) = out.failure() {
        return Err(ExitError::new("EXIT_FAILED", format!("execute_intents failed (tx {}): {f}", out.hash), false));
    }
    if out.result.is_none() {
        return Err(ExitError::new("NETWORK", format!("execute_intents tx {} is not executed yet", out.hash), true));
    }
    let after = near.account(&plan.signer_id, &At::Optimistic).map_err(nerr)?.map(|a| a.amount).unwrap_or(0);
    Ok(DirectResult { tx_hash: Some(out.hash), delivered: plan.amount.clone(), near_before: before.to_string(), near_after: after.to_string() })
}

/// Signs the planned intent and hands it to the maker's relay (`zecret maker --relay`,
/// `crates/zecret-node/src/relay.rs`), which sends it with `execute_intents` and pays the
/// gas. The relay can only send what the taker key signed. If the nonce is already used,
/// an earlier submission of this plan executed and nothing is sent.
pub fn direct_relay(t: &SharedTransport, rpc_url: &str, relay_url: &str, seed: &[u8; 32], plan: &DirectPlan, now_ms: i64) -> Result<DirectResult, ExitError> {
    let signer = IntentSigner::from_seed(seed);
    if signer.account_id() != plan.signer_id {
        return Err(ExitError::new("INTERNAL", "the stored NEAR key does not own the payout account", false));
    }
    let near = near_rpc(t, rpc_url);
    let nerr = |e: anyhow::Error| ExitError::new("NETWORK", format!("NEAR RPC: {e:#}"), true);
    let before = near.account(&plan.signer_id, &At::Final).map_err(nerr)?.map(|a| a.amount).unwrap_or(0);
    let v = verifier_rpc(t, rpc_url, &plan.verifier);
    let nonce = Nonce::from_base64(&plan.nonce).map_err(|e| classify(&e))?;
    if v.nonce_used(&plan.signer_id, &nonce).map_err(|e| classify(&e))? {
        return Ok(DirectResult { tx_hash: None, delivered: plan.amount.clone(), near_before: before.to_string(), near_after: before.to_string() });
    }
    if now_ms >= plan.deadline_ms {
        return Err(ExitError::new("EXIT_FAILED", "the signed exit expired before it was sent; a new one is needed", false));
    }
    let signed = plan.draft()?.sign(&signer, Standard::RawEd25519, time_ms(now_ms)?).map_err(|e| classify(&e))?;
    if signed.intent_hash_base58().map_err(|e| classify(&e))? != plan.intent_hash {
        return Err(ExitError::new("INTERNAL", "the stored plan does not rebuild the same intent", false));
    }
    let body = execute_intents_args(&[signed]).to_string();
    let r = t
        .post(relay_url, &[("content-type", "application/json")], body.as_bytes())
        .map_err(|e| ExitError::new("NETWORK", format!("the maker's relay did not answer: {e:#}"), true))?;
    let a: Value = serde_json::from_slice(&r.body).unwrap_or_else(|_| json!({"ok": false, "error": String::from_utf8_lossy(&r.body)}));
    let hash = a["hash"].as_str().map(str::to_string);
    if a["ok"].as_bool() != Some(true) {
        let msg = a["error"].as_str().unwrap_or("no reason given").to_string();
        if a["pending"].as_bool() == Some(true) || r.status >= 500 {
            return Err(ExitError::new("NETWORK", format!("the relay's execute_intents is not final yet: {msg}"), true));
        }
        return Err(ExitError::new("EXIT_FAILED", format!("the maker's relay did not execute the exit{}: {msg}", hash.map(|h| format!(" (tx {h})")).unwrap_or_default()), false));
    }
    let after = near.account(&plan.signer_id, &At::Optimistic).map_err(nerr)?.map(|a| a.amount).unwrap_or(0);
    Ok(DirectResult { tx_hash: hash, delivered: plan.amount.clone(), near_before: before.to_string(), near_after: after.to_string() })
}

/// The time format 1Click and the Verifier use, for the worker's logs.
pub fn format_time(ms: i64) -> String {
    time_ms(ms).map(time::format).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zecret_intents::ApiError;

    #[test]
    fn near_account_ids() {
        for ok in ["alice.near", "a1", "user_1.test.near", "0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0"] {
            assert!(valid_near_account(ok), "{ok}");
        }
        for bad in ["A.near", "a", ".alice", "alice.", "al..ice", "al ice", ""] {
            assert!(!valid_near_account(bad), "{bad}");
        }
    }

    #[test]
    fn errors_map_to_web_codes() {
        let api = |status, msg: &str| Error::Api(ApiError::from_response(status, format!("{{\"message\":\"{msg}\"}}").as_bytes()));
        let c = classify(&api(400, "Quoting for this pair is not available"));
        assert_eq!((c.code, c.transient), ("EXIT_NO_QUOTE", true));
        assert_eq!(classify(&api(400, "Amount is too low for bridge, try at least 5909617")).code, "EXIT_AMOUNT_TOO_LOW");
        assert_eq!(classify(&api(503, "down")).code, "NETWORK");
        assert_eq!(classify(&Error::Transport("timeout".into())).code, "NETWORK");
        assert_eq!(classify(&Error::Refused(Refusal::QuoteSignature)).code, "EXIT_REFUSED");
        assert_eq!(classify(&api(400, "recipient is not valid")).code, "EXIT_FAILED");
    }

    #[test]
    fn a_stored_plan_rebuilds_the_same_signed_intent() {
        let seed = [7u8; 32];
        let signer = IntentSigner::from_seed(&seed);
        let deadline_ms = 1_794_600_000_123;
        let nonce = Nonce::versioned_random(zecret_intents::Salt([1, 2, 3, 4]), time_ms(deadline_ms).unwrap()).unwrap();
        let mut plan = DirectPlan {
            signer_id: signer.account_id(),
            verifier: "intents.test.near".into(),
            token: "wrap.test.near".into(),
            receiver: "alice.test.near".into(),
            amount: "30000000000000000000000000".into(),
            deadline_ms,
            nonce: nonce.to_base64(),
            intent_hash: String::new(),
        };
        let now = time_ms(deadline_ms - 60_000).unwrap();
        let a = plan.draft().unwrap().sign(&signer, Standard::RawEd25519, now).unwrap();
        plan.intent_hash = a.intent_hash_base58().unwrap();
        let json = serde_json::to_string(&plan).unwrap();
        let again: DirectPlan = serde_json::from_str(&json).unwrap();
        let b = again.draft().unwrap().sign(&signer, Standard::RawEd25519, now).unwrap();
        assert_eq!(a.to_json(), b.to_json());
        assert_eq!(b.intent_hash_base58().unwrap(), plan.intent_hash);
        // NearKey and IntentSigner from the same seed are the same key and account.
        assert_eq!(NearKey::from_seed(seed).implicit_account(), signer.account_id());
    }
}
