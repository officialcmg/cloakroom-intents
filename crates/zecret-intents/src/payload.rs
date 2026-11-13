//! Verifier payloads: the intents, the message a key signs, and the signed
//! `MultiPayload` that `execute_intents` and `/v0/submit-intent` take.
//!
//! Formats follow `contracts/defuse/core/src/payload/` and `crates/signatures/nep413`
//! in `near/intents` (commit fce0936):
//!
//! - `raw_ed25519`: the payload is a JSON string with `signer_id`,
//!   `verifying_contract`, `deadline`, `nonce` and `intents`. The key signs the UTF-8
//!   bytes of that string. The intent hash is SHA-256 of the same bytes.
//! - `nep413`: the payload is `{message, nonce, recipient, callbackUrl?}`, where
//!   `message` is a JSON string with `signer_id`, `deadline` and `intents`, and
//!   `recipient` is the verifying contract. The key signs SHA-256 of the borsh
//!   encoding of `(2^31 + 413 as u32, message, nonce, recipient, callback_url)`. The
//!   intent hash is that same SHA-256.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::nonce::Nonce;
use crate::time::{self, Time};

/// A signing standard this crate signs with. Both use an ed25519 key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Standard {
    RawEd25519,
    Nep413,
}

impl Standard {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RawEd25519 => "raw_ed25519",
            Self::Nep413 => "nep413",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "raw_ed25519" => Ok(Self::RawEd25519),
            "nep413" => Ok(Self::Nep413),
            other => Err(Error::Invalid(format!(
                "signing standard {other:?} is not supported (raw_ed25519, nep413)"
            ))),
        }
    }
}

/// A `transfer` intent: move tokens to another account inside the Verifier. Token ids
/// carry their standard prefix (`nep141:usdc.near`). Amounts are decimal strings in
/// the token's smallest unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub receiver_id: String,
    pub tokens: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

/// An `ft_withdraw` intent: withdraw a NEP-141 token from the Verifier to a NEAR
/// account with `ft_transfer`. The token id has **no** prefix (`usdc.near`).
///
/// `msg` is not offered: with `msg` the Verifier uses `ft_transfer_call`, and a failed
/// call is not refunded (Verifier docs, "Refunds on failed withdrawals").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FtWithdraw {
    pub token: String,
    pub receiver_id: String,
    pub amount: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

/// The intents this crate builds and accepts. Serializes as the Verifier's
/// internally tagged form: `{"intent": "transfer", ...}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    Transfer(Transfer),
    FtWithdraw(FtWithdraw),
}

impl Intent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Transfer(_) => "transfer",
            Self::FtWithdraw(_) => "ft_withdraw",
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Parses one intent strictly: only `transfer` and `ft_withdraw`, and no fields
    /// that this crate does not know (a `transfer` with `msg` would call a contract).
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let mut obj = v
            .as_object()
            .cloned()
            .ok_or_else(|| "intent is not an object".to_string())?;
        let tag = obj
            .remove("intent")
            .and_then(|t| t.as_str().map(str::to_owned))
            .ok_or_else(|| "intent has no `intent` tag".to_string())?;
        let body = Value::Object(obj);
        match tag.as_str() {
            "transfer" => serde_json::from_value(body)
                .map(Self::Transfer)
                .map_err(|e| format!("transfer: {e}")),
            "ft_withdraw" => serde_json::from_value(body)
                .map(Self::FtWithdraw)
                .map_err(|e| format!("ft_withdraw: {e}")),
            other => Err(format!("intent type {other:?} is not accepted")),
        }
    }
}

/// The NEP-413 payload object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Nep413Payload {
    pub message: String,
    /// Base64 of 32 bytes.
    pub nonce: String,
    pub recipient: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

const NEP413_TAG: u32 = (1u32 << 31) + 413;

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

impl Nep413Payload {
    /// SHA-256 of `borsh((NEP413_TAG, payload))`: the bytes an ed25519 key signs.
    pub fn prehash(&self) -> Result<[u8; 32]> {
        let nonce = Nonce::from_base64(&self.nonce)?;
        let mut buf = Vec::with_capacity(64 + self.message.len() + self.recipient.len());
        buf.extend_from_slice(&NEP413_TAG.to_le_bytes());
        borsh_string(&mut buf, &self.message);
        buf.extend_from_slice(&nonce.0);
        borsh_string(&mut buf, &self.recipient);
        match &self.callback_url {
            None => buf.push(0),
            Some(u) => {
                buf.push(1);
                borsh_string(&mut buf, u);
            }
        }
        Ok(Sha256::digest(&buf).into())
    }
}

/// The content of a payload, the same for every standard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedPayload {
    pub signer_id: String,
    pub verifying_contract: String,
    pub deadline: Time,
    pub nonce: Nonce,
    pub intents: Vec<Intent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPayloadJson {
    signer_id: String,
    verifying_contract: String,
    deadline: String,
    nonce: String,
    #[serde(default)]
    intents: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nep413MessageJson {
    signer_id: String,
    deadline: String,
    #[serde(default)]
    intents: Vec<Value>,
}

fn decode_intents(list: &[Value]) -> Result<Vec<Intent>> {
    list.iter()
        .map(|v| Intent::from_json(v).map_err(Error::Parse))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Unsigned {
    Raw(String),
    Nep413(Nep413Payload),
}

/// A payload ready to sign. It can only be made by this crate: by
/// [`crate::direct::IntentDraft`] (our own intents) or by
/// [`crate::check::check_generated_intent`] (a 1Click intent that passed the check).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsignedPayload(Unsigned);

impl UnsignedPayload {
    pub(crate) fn raw(payload: String) -> Self {
        Self(Unsigned::Raw(payload))
    }

    pub(crate) fn nep413(payload: Nep413Payload) -> Self {
        Self(Unsigned::Nep413(payload))
    }

    pub(crate) fn from_parts(d: &DecodedPayload, standard: Standard) -> Self {
        // Structs, not `json!`, so that the field order is fixed whatever features
        // serde_json is built with.
        #[derive(Serialize)]
        struct RawOut<'a> {
            signer_id: &'a str,
            verifying_contract: &'a str,
            deadline: String,
            nonce: String,
            intents: &'a [Intent],
        }
        #[derive(Serialize)]
        struct Nep413MessageOut<'a> {
            signer_id: &'a str,
            deadline: String,
            intents: &'a [Intent],
        }
        match standard {
            Standard::RawEd25519 => Self::raw(
                serde_json::to_string(&RawOut {
                    signer_id: &d.signer_id,
                    verifying_contract: &d.verifying_contract,
                    deadline: time::format(d.deadline),
                    nonce: d.nonce.to_base64(),
                    intents: &d.intents,
                })
                .unwrap_or_default(),
            ),
            Standard::Nep413 => Self::nep413(Nep413Payload {
                message: serde_json::to_string(&Nep413MessageOut {
                    signer_id: &d.signer_id,
                    deadline: time::format(d.deadline),
                    intents: &d.intents,
                })
                .unwrap_or_default(),
                nonce: d.nonce.to_base64(),
                recipient: d.verifying_contract.clone(),
                callback_url: None,
            }),
        }
    }

    pub fn standard(&self) -> Standard {
        match &self.0 {
            Unsigned::Raw(_) => Standard::RawEd25519,
            Unsigned::Nep413(_) => Standard::Nep413,
        }
    }

    /// The exact bytes the ed25519 key signs.
    pub fn bytes_to_sign(&self) -> Result<Vec<u8>> {
        match &self.0 {
            Unsigned::Raw(p) => Ok(p.as_bytes().to_vec()),
            Unsigned::Nep413(p) => Ok(p.prehash()?.to_vec()),
        }
    }

    /// The Verifier's intent hash (the `intentHash` 1Click returns, in base58).
    pub fn intent_hash(&self) -> Result<[u8; 32]> {
        match &self.0 {
            Unsigned::Raw(p) => Ok(Sha256::digest(p.as_bytes()).into()),
            Unsigned::Nep413(p) => p.prehash(),
        }
    }

    pub fn intent_hash_base58(&self) -> Result<String> {
        Ok(bs58::encode(self.intent_hash()?).into_string())
    }

    /// The payload as it goes into the `MultiPayload` (a string for `raw_ed25519`, an
    /// object for `nep413`).
    pub fn payload_json(&self) -> Value {
        match &self.0 {
            Unsigned::Raw(p) => Value::String(p.clone()),
            Unsigned::Nep413(p) => serde_json::to_value(p).unwrap_or(Value::Null),
        }
    }

    /// Reads what the payload says. Fails on any field or intent type this crate does
    /// not know.
    pub fn decode(&self) -> Result<DecodedPayload> {
        match &self.0 {
            Unsigned::Raw(p) => {
                let j: RawPayloadJson = serde_json::from_str(p)
                    .map_err(|e| Error::Parse(format!("raw_ed25519 payload: {e}")))?;
                Ok(DecodedPayload {
                    signer_id: j.signer_id,
                    verifying_contract: j.verifying_contract,
                    deadline: time::parse(&j.deadline)?,
                    nonce: Nonce::from_base64(&j.nonce)?,
                    intents: decode_intents(&j.intents)?,
                })
            }
            Unsigned::Nep413(p) => {
                if p.callback_url.is_some() {
                    return Err(Error::Parse("nep413 payload has a callbackUrl".into()));
                }
                let j: Nep413MessageJson = serde_json::from_str(&p.message)
                    .map_err(|e| Error::Parse(format!("nep413 message: {e}")))?;
                Ok(DecodedPayload {
                    signer_id: j.signer_id,
                    verifying_contract: p.recipient.clone(),
                    deadline: time::parse(&j.deadline)?,
                    nonce: Nonce::from_base64(&p.nonce)?,
                    intents: decode_intents(&j.intents)?,
                })
            }
        }
    }
}

/// A signed `MultiPayload`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedPayload {
    pub(crate) unsigned: UnsignedPayload,
    pub(crate) public_key: [u8; 32],
    pub(crate) signature: [u8; 64],
}

impl SignedPayload {
    pub fn unsigned(&self) -> &UnsignedPayload {
        &self.unsigned
    }

    pub fn public_key_str(&self) -> String {
        format!("ed25519:{}", bs58::encode(self.public_key).into_string())
    }

    pub fn signature_str(&self) -> String {
        format!("ed25519:{}", bs58::encode(self.signature).into_string())
    }

    pub fn intent_hash_base58(&self) -> Result<String> {
        self.unsigned.intent_hash_base58()
    }

    /// The `MultiPayload` JSON: `{standard, payload, public_key, signature}`.
    pub fn to_json(&self) -> Value {
        json!({
            "standard": self.unsigned.standard().as_str(),
            "payload": self.unsigned.payload_json(),
            "public_key": self.public_key_str(),
            "signature": self.signature_str(),
        })
    }
}

impl Serialize for SignedPayload {
    fn serialize<S: serde::Serializer>(&self, s: S) -> core::result::Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, VerifyingKey};

    #[test]
    fn nep413_prehash_matches_the_verifier_test_vector() {
        // From crates/signatures/nep413/src/lib.rs in near/intents.
        let pk = VerifyingKey::from_bytes(
            &hex::decode("e2e9cb7ac57cb46d4da1ce1d1cc2c33bdfe17407c517916b522724a8ea2c6c50")
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let sig = Signature::from_bytes(
            &hex::decode(
                "e2ff6254871a3fec1853c167b42f0f14248c4cf7fef5452dc24d8dbdc5c4bf18\
                 3ab707322b4d782d5f5a05571bae476c5f7ee41c473f3002e600865e46b75d0f",
            )
            .unwrap()
            .try_into()
            .unwrap(),
        );
        let p = Nep413Payload {
            message: "Hello, world!".into(),
            nonce: Nonce([0u8; 32]).to_base64(),
            recipient: "intents.near".into(),
            callback_url: None,
        };
        pk.verify_strict(&p.prehash().unwrap(), &sig).unwrap();
    }

    #[test]
    fn raw_intent_hash_matches_the_verifier_test_vector() {
        // From contracts/defuse/core/src/payload/multi.rs in near/intents.
        let payload = r#"{"signer_id":"74affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d53751","verifying_contract":"intents.near","deadline":{"timestamp":1732035219},"nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"token_diff","diff":{"nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near":"-1000","nep141:eth-0xdac17f958d2ee523a2206206994597c13d831ec7.omft.near":"998"}}]}"#;
        let u = UnsignedPayload::raw(payload.into());
        assert_eq!(
            u.intent_hash_base58().unwrap(),
            "8LKE47o44ybZQR9ozLyDnvMDTh4Ao5ipy2mJWsYByG5Q"
        );
        // The payload has a token_diff intent and an old deadline format: decode refuses.
        assert!(u.decode().is_err());
    }

    #[test]
    fn intent_json_roundtrip_and_strictness() {
        let t = Intent::Transfer(Transfer {
            receiver_id: "bob.near".into(),
            tokens: [("nep141:usdc.near".to_string(), "10".to_string())].into(),
            memo: None,
        });
        let v = t.to_json();
        assert_eq!(
            v,
            json!({"intent":"transfer","receiver_id":"bob.near","tokens":{"nep141:usdc.near":"10"}})
        );
        assert_eq!(Intent::from_json(&v).unwrap(), t);

        let w = Intent::FtWithdraw(FtWithdraw {
            token: "usdc.near".into(),
            receiver_id: "bob.near".into(),
            amount: "10".into(),
            memo: Some("m".into()),
        });
        assert_eq!(Intent::from_json(&w.to_json()).unwrap(), w);

        // A transfer with a notification `msg` would call the receiver: refused.
        let with_msg = json!({"intent":"transfer","receiver_id":"bob.near","tokens":{"a":"1"},"msg":"x"});
        assert!(Intent::from_json(&with_msg).is_err());
        let withdraw_call = json!({"intent":"ft_withdraw","token":"a","receiver_id":"b","amount":"1","msg":""});
        assert!(Intent::from_json(&withdraw_call).is_err());
        let diff = json!({"intent":"token_diff","diff":{"a":"-1","b":"1"}});
        assert!(Intent::from_json(&diff).is_err());
        assert!(Intent::from_json(&json!("transfer")).is_err());
        assert!(Intent::from_json(&json!({"receiver_id":"b"})).is_err());
    }

    #[test]
    fn standards_parse() {
        assert_eq!(Standard::parse("nep413").unwrap(), Standard::Nep413);
        assert_eq!(
            Standard::parse("raw_ed25519").unwrap().as_str(),
            "raw_ed25519"
        );
        assert!(Standard::parse("erc191").is_err());
    }

    #[test]
    fn nep413_with_callback_is_hashed_and_refused_on_decode() {
        let mut p = Nep413Payload {
            message: r#"{"signer_id":"a","deadline":"2026-11-16T03:33:00.000Z","intents":[]}"#
                .into(),
            nonce: Nonce([1u8; 32]).to_base64(),
            recipient: "intents.near".into(),
            callback_url: None,
        };
        let plain = p.prehash().unwrap();
        p.callback_url = Some("https://example.com".into());
        assert_ne!(p.prehash().unwrap(), plain);
        assert!(UnsignedPayload::nep413(p).decode().is_err());
    }
}
