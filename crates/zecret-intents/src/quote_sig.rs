//! 1Click quote signatures.
//!
//! 1Click signs every quote (dry and firm) with one ed25519 key. The algorithm is taken
//! from `verifyQuoteSignature` in `@defuse-protocol/one-click-sdk-typescript` 0.1.26
//! (`dist/index.mjs`):
//!
//! 1. Take a fixed set of `quoteRequest` fields and a fixed set of `quote` fields
//!    (fewer for dry quotes). Optional fields that are "falsy" in JavaScript are left
//!    out.
//! 2. Merge `{...request, ...quote, timestamp}` (so `quote.deadline` replaces
//!    `quoteRequest.deadline` on firm quotes).
//! 3. Serialize with sorted keys and no spaces (`json-stable-stringify`).
//! 4. SHA-256, then base58. The key signs the UTF-8 bytes of the base58 string.
//!
//! A valid signature binds the deposit address to the recipient, destination asset,
//! amounts, refund address and deadlines that 1Click promised for it.

use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// `ONE_CLICK_MANAGER_PUB_KEY` in the official SDK:
/// `ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc`.
pub const ONE_CLICK_QUOTE_KEY_B58: &str = "reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";

/// The same key as bytes.
pub const ONE_CLICK_QUOTE_KEY: [u8; 32] = [
    0x0c, 0xb7, 0xe9, 0x49, 0xe3, 0x87, 0x1e, 0xcf, 0x28, 0x4a, 0x50, 0xb8, 0xe7, 0x74, 0xab, 0x29,
    0xb3, 0xf7, 0xb2, 0x36, 0x3e, 0x39, 0x8c, 0xef, 0x1d, 0xf2, 0xe9, 0x44, 0x57, 0xb7, 0xe9, 0x43,
];

const REQUIRED_REQUEST_FIELDS: [&str; 12] = [
    "dry",
    "swapType",
    "slippageTolerance",
    "originAsset",
    "depositType",
    "destinationAsset",
    "amount",
    "refundTo",
    "refundType",
    "recipient",
    "recipientType",
    "deadline",
];

const OPTIONAL_REQUEST_FIELDS: [&str; 5] = [
    "quoteWaitingTimeMs",
    "referral",
    "virtualChainRecipient",
    "virtualChainRefundRecipient",
    "customRecipientMsg",
];

const DRY_QUOTE_FIELDS: [&str; 8] = [
    "amountIn",
    "amountInFormatted",
    "amountInUsd",
    "minAmountIn",
    "amountOut",
    "amountOutFormatted",
    "amountOutUsd",
    "minAmountOut",
];

const FIRM_OPTIONAL_QUOTE_FIELDS: [&str; 7] = [
    "depositAddress",
    "depositMemo",
    "deadline",
    "timeWhenInactive",
    "timeEstimate",
    "refundFee",
    "withdrawFee",
];

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JavaScript `{...a, k: v}` where `v` may be `undefined`: an undefined value
/// overrides an earlier key and is then dropped by `JSON.stringify`.
fn set(map: &mut Map<String, Value>, key: &str, v: Option<&Value>) {
    match v {
        Some(v) => {
            map.insert(key.into(), v.clone());
        }
        None => {
            map.remove(key);
        }
    }
}

/// The base58 SHA-256 message 1Click signs for a quote answer (the whole response
/// object: `{quoteRequest, quote, timestamp, signature, …}`; also works for the
/// `quoteResponse` inside a status answer).
pub fn quote_message(response: &Value) -> Option<String> {
    let req = response.get("quoteRequest")?.as_object()?;
    let quote = response.get("quote")?.as_object()?;
    let timestamp = response.get("timestamp")?;
    let dry = req.get("dry").is_some_and(truthy);

    let mut m = Map::new();
    for k in REQUIRED_REQUEST_FIELDS {
        set(&mut m, k, req.get(k));
    }
    for k in OPTIONAL_REQUEST_FIELDS {
        set(&mut m, k, req.get(k).filter(|v| truthy(v)));
    }
    for k in DRY_QUOTE_FIELDS {
        set(&mut m, k, quote.get(k));
    }
    if !dry {
        for k in FIRM_OPTIONAL_QUOTE_FIELDS {
            set(&mut m, k, quote.get(k).filter(|v| truthy(v)));
        }
    }
    m.insert("timestamp".into(), timestamp.clone());
    let text = stable_stringify(&Value::Object(m));
    Some(bs58::encode(Sha256::digest(text.as_bytes())).into_string())
}

/// `json-stable-stringify`: objects with keys sorted, no spaces. Sorting is explicit,
/// so the result does not depend on serde_json's `preserve_order` feature. Keys are
/// compared by UTF-16 code units as in JavaScript.
fn stable_stringify(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let body: Vec<String> = entries
                .into_iter()
                .map(|(k, v)| format!("{}:{}", Value::String(k.clone()), stable_stringify(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(items) => {
            let body: Vec<String> = items.iter().map(stable_stringify).collect();
            format!("[{}]", body.join(","))
        }
        scalar => scalar.to_string(),
    }
}

/// Checks a quote answer's `signature` against `key`.
pub fn verify(response: &Value, key: &[u8; 32]) -> bool {
    let Some(message) = quote_message(response) else {
        return false;
    };
    let Some(sig) = response
        .get("signature")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix("ed25519:"))
        .and_then(|s| bs58::decode(s).into_vec().ok())
        .and_then(|v| <[u8; 64]>::try_from(v).ok())
    else {
        return false;
    };
    let Ok(pk) = VerifyingKey::from_bytes(key) else {
        return false;
    };
    pk.verify_strict(message.as_bytes(), &Signature::from_bytes(&sig))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_constant_matches_base58() {
        assert_eq!(
            bs58::decode(ONE_CLICK_QUOTE_KEY_B58).into_vec().unwrap(),
            ONE_CLICK_QUOTE_KEY.to_vec()
        );
    }

    #[test]
    fn stable_stringify_sorts_nested_keys() {
        let v = serde_json::json!({"b": 1, "a": {"d": [true, null], "c": "x"}});
        assert_eq!(stable_stringify(&v), r#"{"a":{"c":"x","d":[true,null]},"b":1}"#);
    }

    #[test]
    fn javascript_truthiness() {
        assert!(!truthy(&Value::Null));
        assert!(!truthy(&serde_json::json!(0)));
        assert!(!truthy(&serde_json::json!("")));
        assert!(!truthy(&serde_json::json!(false)));
        assert!(truthy(&serde_json::json!(12)));
        assert!(truthy(&serde_json::json!("0")));
    }
}
