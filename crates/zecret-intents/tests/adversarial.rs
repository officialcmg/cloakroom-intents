//! A hostile 1Click: quotes that carry a valid signature by the quote key and still try to
//! move the taker's USDC somewhere else, or more of it, or for less.
//!
//! The recorded tests (`recorded.rs`) change answers after 1Click signed them, so most of
//! their refusals are `QuoteSignature`. Here each changed quote is signed again with a test
//! key, and the policy pins that test key. So these tests reach the field checks behind
//! the signature: they show what the check refuses if the quote key itself is used against
//! the user (stolen, or 1Click compromised). With the real key pinned, every one of these
//! quotes is refused already at the signature.

use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::{Value, json};
use zecret_intents::check::{Policy, Refusal, valid_account_id};
use zecret_intents::oneclick::{GeneratedIntent, StatusResponse, USDC_NEAR_ASSET};
use zecret_intents::quote_sig::quote_message;
use zecret_intents::{
    ApprovedExit, QuoteResponse, Standard, check_generated_intent, check_status, time,
};

const USDC_BASE: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
const RECIPIENT: &str = "0x1111111111111111111111111111111111111111";
const SIGNER: &str = "0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0";
const ATTACKER: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("json")
}

fn hostile_key() -> SigningKey {
    SigningKey::from_bytes(&[0x42; 32])
}

fn hostile_policy() -> Policy {
    Policy {
        quote_key: hostile_key().verifying_key().to_bytes(),
        ..Policy::default()
    }
}

/// Signs a quote answer the way 1Click does, with the hostile key.
fn sign(raw: &mut Value) {
    let m = quote_message(raw).expect("quote message");
    let sig = hostile_key().sign(m.as_bytes()).to_bytes();
    raw["signature"] = json!(format!("ed25519:{}", bs58::encode(sig).into_string()));
}

struct Case {
    raw_quote: Value,
    generated: GeneratedIntent,
    approved: ApprovedExit,
    now: time::Time,
    standard: Standard,
}

fn case(name: &str, standard: Standard) -> Case {
    let f = fixture(name);
    let ex = f["exchanges"].as_array().unwrap();
    let mut raw_quote = ex[1]["response"]["body"].clone();
    sign(&mut raw_quote);
    let g = &ex[2]["response"]["body"]["intent"];
    let min: u128 = f["notes"]["approved_min_amount_out"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    Case {
        raw_quote,
        generated: GeneratedIntent {
            standard: g["standard"].as_str().unwrap().into(),
            payload: g["payload"].clone(),
            correlation_id: None,
        },
        approved: ApprovedExit::mainnet(
            SIGNER,
            USDC_NEAR_ASSET,
            5_000_000,
            USDC_BASE,
            RECIPIENT,
            min,
        ),
        now: time::parse(f["recorded_at"].as_str().unwrap()).unwrap(),
        standard,
    }
}

fn both() -> [Case; 2] {
    [
        case("exit_raw_ed25519", Standard::RawEd25519),
        case("exit_nep413", Standard::Nep413),
    ]
}

impl Case {
    fn quote(&self) -> QuoteResponse {
        QuoteResponse::from_value(self.raw_quote.clone()).unwrap()
    }

    /// Changes the quote and signs it again with the hostile key.
    fn edit_quote(&mut self, f: impl FnOnce(&mut Value)) {
        f(&mut self.raw_quote);
        sign(&mut self.raw_quote);
    }

    fn edit_payload(&mut self, f: impl FnOnce(&mut Value)) {
        let text = match self.standard {
            Standard::RawEd25519 => self.generated.payload.as_str().unwrap().to_owned(),
            Standard::Nep413 => self.generated.payload["message"]
                .as_str()
                .unwrap()
                .to_owned(),
        };
        let mut v: Value = serde_json::from_str(&text).unwrap();
        f(&mut v);
        match self.standard {
            Standard::RawEd25519 => self.generated.payload = Value::String(v.to_string()),
            Standard::Nep413 => self.generated.payload["message"] = Value::String(v.to_string()),
        }
    }

    fn check_with(&self, policy: &Policy) -> Result<zecret_intents::CheckedIntent, Refusal> {
        check_generated_intent(
            &self.approved,
            &self.quote(),
            &self.generated,
            self.standard,
            policy,
            self.now,
        )
    }

    fn check(&self) -> Result<zecret_intents::CheckedIntent, Refusal> {
        self.check_with(&hostile_policy())
    }
}

#[test]
fn a_correctly_signed_hostile_quote_is_refused_by_the_pinned_key() {
    for c in both() {
        // The harness signs correctly: with the hostile key pinned, the unchanged answer passes.
        c.check()
            .expect("the re-signed recorded answer passes with its key pinned");
        // With 1Click's key pinned (the default policy) it is refused.
        assert_eq!(
            c.check_with(&Policy::default()),
            Err(Refusal::QuoteSignature)
        );
    }
}

#[test]
fn a_signed_quote_for_another_recipient_or_asset_is_refused() {
    for mut c in both() {
        c.edit_quote(|q| {
            q["quoteRequest"]["recipient"] = json!("0x2222222222222222222222222222222222222222")
        });
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "recipient",
                ..
            })
        ));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["destinationAsset"] = json!("nep141:sol.omft.near"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "destinationAsset",
                ..
            })
        ));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["originAsset"] = json!("nep141:wrap.near"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "originAsset",
                ..
            })
        ));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["recipientType"] = json!("INTENTS"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "recipientType",
                ..
            })
        ));
    }
}

#[test]
fn a_signed_quote_that_refunds_elsewhere_is_refused() {
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["refundTo"] = json!(ATTACKER));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "refundTo",
                ..
            })
        ));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["refundType"] = json!("ORIGIN_CHAIN"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "refundType",
                ..
            })
        ));
    }
    for (k, v) in [
        ("customRecipientMsg", json!("{\"receiver_id\":\"x\"}")),
        (
            "virtualChainRecipient",
            json!("0x2222222222222222222222222222222222222222"),
        ),
        (
            "virtualChainRefundRecipient",
            json!("0x2222222222222222222222222222222222222222"),
        ),
    ] {
        for mut c in both() {
            c.edit_quote(|q| q["quoteRequest"][k] = v.clone());
            assert!(
                matches!(c.check(), Err(Refusal::QuoteHasUnexpectedField(f)) if f == k),
                "{k}"
            );
        }
    }
}

#[test]
fn a_signed_quote_below_the_approved_minimum_is_refused() {
    for mut c in both() {
        let min = c.approved.min_amount_out;
        c.edit_quote(|q| q["quote"]["minAmountOut"] = json!((min - 1).to_string()));
        assert_eq!(
            c.check(),
            Err(Refusal::MinimumTooLow {
                approved: min,
                quoted: min - 1
            })
        );
    }
}

#[test]
fn a_signed_quote_for_another_amount_is_refused() {
    for mut c in both() {
        c.edit_quote(|q| q["quote"]["amountIn"] = json!("5000001"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "quote.amountIn",
                ..
            })
        ));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["amount"] = json!("50000000"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "amount",
                ..
            })
        ));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["swapType"] = json!("FLEX_INPUT"));
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "swapType",
                ..
            })
        ));
    }
}

#[test]
fn a_deposit_address_that_is_not_an_intents_account_is_refused() {
    for bad in [
        "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM", // a Solana address (upper case)
        "0X2222222222222222222222222222222222222222",   // an EVM address in upper case
        SIGNER,                                         // the taker itself
        "intents.near",                                 // the Verifier
        "a",
        ".near",
    ] {
        for mut c in both() {
            c.edit_quote(|q| q["quote"]["depositAddress"] = json!(bad));
            c.edit_payload(|p| p["intents"][0]["receiver_id"] = json!(bad));
            assert_eq!(
                c.check(),
                Err(Refusal::BadDepositAddress(bad.into())),
                "{bad}"
            );
        }
    }
    for mut c in both() {
        c.edit_quote(|q| {
            q["quote"].as_object_mut().unwrap().remove("depositAddress");
        });
        assert_eq!(c.check(), Err(Refusal::NoDepositAddress));
    }
    // A lower-case EVM address is a valid Verifier account (an account of that EVM key), so
    // only the signature can tell whether 1Click chose it.
    assert!(valid_account_id(ATTACKER) && valid_account_id("alice.near"));
    assert!(valid_account_id(
        "0x2222222222222222222222222222222222222222"
    ));
}

#[test]
fn a_signed_quote_that_needs_a_memo_is_refused() {
    for mut c in both() {
        c.edit_quote(|q| {
            q["quote"]["depositMemo"] = json!("12345");
            q["quoteRequest"]["depositMode"] = json!("MEMO");
        });
        assert_eq!(
            c.check(),
            Err(Refusal::QuoteHasUnexpectedField("depositMemo"))
        );
    }
}

#[test]
fn expired_old_or_dry_signed_quotes_are_refused() {
    for mut c in both() {
        let past = time::format(c.now - chrono::Duration::seconds(1));
        c.edit_quote(|q| q["quote"]["deadline"] = json!(past));
        assert!(matches!(c.check(), Err(Refusal::QuoteExpired { .. })));
    }
    for mut c in both() {
        let old = time::format(c.now - chrono::Duration::minutes(6));
        c.edit_quote(|q| q["timestamp"] = json!(old));
        assert!(matches!(c.check(), Err(Refusal::QuoteTooOld { .. })));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quoteRequest"]["dry"] = json!(true));
        assert_eq!(c.check(), Err(Refusal::QuoteIsDry));
    }
}

#[test]
fn a_quote_without_a_signature_is_refused() {
    for mut c in both() {
        c.raw_quote.as_object_mut().unwrap().remove("signature");
        assert!(
            QuoteResponse::from_value(c.raw_quote.clone()).is_err(),
            "no signature field: not a quote"
        );
        c.raw_quote["signature"] = json!("");
        assert_eq!(c.check(), Err(Refusal::QuoteSignature));
    }
}

#[test]
fn a_payload_that_moves_more_or_elsewhere_than_the_signed_quote_is_refused() {
    for mut c in both() {
        c.edit_payload(|p| p["intents"][0]["receiver_id"] = json!(ATTACKER));
        assert!(matches!(c.check(), Err(Refusal::WrongReceiver { .. })));
    }
    for mut c in both() {
        c.edit_payload(|p| p["intents"][0]["tokens"][USDC_NEAR_ASSET] = json!("5000001"));
        assert!(matches!(c.check(), Err(Refusal::WrongAmount { .. })));
    }
    for mut c in both() {
        c.edit_payload(|p| {
            let extra = json!({"intent":"ft_withdraw","token":"17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1","receiver_id":"attacker.near","amount":"1"});
            p["intents"].as_array_mut().unwrap().push(extra);
        });
        assert_eq!(c.check(), Err(Refusal::IntentCount(2)));
    }
    for mut c in both() {
        c.edit_payload(|p| p["intents"][0]["msg"] = json!("{}"));
        assert!(matches!(c.check(), Err(Refusal::Malformed(_))));
    }
    for mut c in both() {
        c.edit_payload(|p| p["signer_id"] = json!(ATTACKER));
        assert!(matches!(c.check(), Err(Refusal::WrongSigner { .. })));
    }
}

#[test]
fn a_status_answer_must_be_signed_and_name_the_deposit_address() {
    let f = fixture("exit_raw_ed25519");
    let ex = f["exchanges"].as_array().unwrap();
    let deposit = ex[1]["response"]["body"]["quote"]["depositAddress"]
        .as_str()
        .unwrap()
        .to_owned();
    let status_body = ex
        .iter()
        .find(|e| e["request"]["url"].as_str().unwrap().contains("/v0/status"))
        .unwrap()["response"]["body"]
        .clone();
    let parse = |v: &Value| serde_json::from_value::<StatusResponse>(v.clone()).unwrap();
    check_status(&parse(&status_body), &deposit, &Policy::default())
        .expect("the recorded status is signed");
    assert!(matches!(
        check_status(&parse(&status_body), ATTACKER, &Policy::default()),
        Err(Refusal::QuoteMismatch {
            field: "quote.depositAddress",
            ..
        })
    ));
    let mut forged = status_body.clone();
    forged["quoteResponse"]["quote"]["depositAddress"] = json!(ATTACKER);
    assert_eq!(
        check_status(&parse(&forged), ATTACKER, &Policy::default()),
        Err(Refusal::QuoteSignature)
    );
    let mut bare = status_body;
    bare.as_object_mut().unwrap().remove("quoteResponse");
    assert!(matches!(
        check_status(&parse(&bare), &deposit, &Policy::default()),
        Err(Refusal::Malformed(_))
    ));
}
