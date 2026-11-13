//! Tests against responses recorded from the live 1Click API on 2026-11-13
//! (`examples/record_live.rs` and `tests/fixtures/manual_captures.json`).

use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::{Value, json};
use zecret_intents::check::{Policy, Refusal};
use zecret_intents::oneclick::{GeneratedIntent, USDC_NEAR_ASSET, parse_tokens};
use zecret_intents::{
    ApiErrorKind, ApprovedExit, Error, ExitClient, ExitQuoteParams, HttpRequest, HttpResponse,
    HttpTransport, IntentSigner, Nonce, OneClick, QuoteResponse, Standard, SwapStatus,
    check_generated_intent, check_quote, time,
};

const USDC_BASE: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
const RECIPIENT: &str = "0x1111111111111111111111111111111111111111";
const SIGNER: &str = "0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0";

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("json")
}

/// Replays recorded exchanges in order. Each request must equal the recorded one:
/// same method, URL and JSON body.
struct Replay {
    queue: RefCell<VecDeque<Value>>,
}

impl Replay {
    fn new(f: &Value) -> Self {
        Self {
            queue: RefCell::new(
                f["exchanges"]
                    .as_array()
                    .expect("exchanges")
                    .iter()
                    .cloned()
                    .collect(),
            ),
        }
    }

    fn remaining(&self) -> usize {
        self.queue.borrow().len()
    }

    fn patch_next_response(&self, index: usize, f: impl FnOnce(&mut Value)) {
        f(&mut self.queue.borrow_mut()[index]["response"]["body"]);
    }
}

impl HttpTransport for Replay {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let e = self
            .queue
            .borrow_mut()
            .pop_front()
            .ok_or("no recorded exchange left")?;
        assert_eq!(e["request"]["method"], r.method.as_str());
        assert_eq!(e["request"]["url"], r.url.as_str());
        assert_eq!(e["request"]["body"], r.json_body().unwrap_or(Value::Null));
        let body = &e["response"]["body"];
        let bytes = match body {
            Value::String(s) => s.clone().into_bytes(),
            other => other.to_string().into_bytes(),
        };
        Ok(HttpResponse::new(
            e["response"]["status"].as_u64().expect("status") as u16,
            bytes,
        ))
    }
}

fn signer() -> IntentSigner {
    IntentSigner::from_seed(&[0x5a; 32])
}

fn replay_exit(name: &str, standard: Standard) {
    let f = fixture(name);
    let now = time::parse(f["recorded_at"].as_str().unwrap()).unwrap();
    let deadline = time::parse(f["notes"]["quote_deadline"].as_str().unwrap()).unwrap();
    let min: u128 = f["notes"]["approved_min_amount_out"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let t = Replay::new(&f);
    let client = ExitClient::new(&t, OneClick::default());
    let s = signer();
    assert_eq!(s.account_id(), SIGNER);

    let dry = client
        .quote(&ExitQuoteParams {
            signer_id: SIGNER.into(),
            origin_asset: USDC_NEAR_ASSET.into(),
            amount: 5_000_000,
            destination_asset: USDC_BASE.into(),
            recipient: RECIPIENT.into(),
            slippage_bps: 100,
            deadline,
            dry: true,
            referral: None,
            quote_waiting_time_ms: None,
        })
        .unwrap();
    assert!(dry.quote.deposit_address.is_none());
    assert!(dry.quote.amount_out().unwrap() > 4_900_000);

    let approved = ApprovedExit::mainnet(
        SIGNER,
        USDC_NEAR_ASSET,
        5_000_000,
        USDC_BASE,
        RECIPIENT,
        min,
    );
    let submitted = client
        .execute_exit(&s, &approved, 100, deadline, standard, now)
        .unwrap();
    // The submit request body matched the recording, so our signature is byte for byte
    // the one 1Click accepted, and the hash it returned is the one we computed.
    assert_eq!(submitted.intent_hash, submitted.checked.intent_hash);
    assert_eq!(submitted.signed.unsigned().standard(), standard);
    let nonce = submitted.signed.unsigned().decode().unwrap().nonce;
    let v = nonce.as_versioned().expect("1Click uses versioned nonces");
    assert_eq!(v.salt.to_hex(), "252812b3");
    assert_eq!(v.deadline, submitted.checked.deadline);
    assert_eq!(
        Some(submitted.checked.deadline),
        submitted.quote.quote.deadline().unwrap()
    );
    // The deposit address deadline is the request deadline plus 72 hours.
    assert_eq!(
        submitted.checked.deadline - deadline,
        chrono::Duration::hours(72)
    );

    let mut last = None;
    while t.remaining() > 0 {
        let st = client.status(submitted.deposit_address()).unwrap();
        assert!(st.quote_response.as_ref().is_some_and(|q| {
            zecret_intents::quote_sig::verify(q, &zecret_intents::quote_sig::ONE_CLICK_QUOTE_KEY)
        }));
        last = Some(st.status);
    }
    // The key holds nothing, so 1Click marks the swap failed.
    assert_eq!(last, Some(SwapStatus::Failed));
}

#[test]
fn replays_the_raw_ed25519_exit() {
    replay_exit("exit_raw_ed25519", Standard::RawEd25519);
}

#[test]
fn replays_the_nep413_exit() {
    replay_exit("exit_nep413", Standard::Nep413);
}

#[test]
fn a_submit_hash_that_differs_is_an_error() {
    let f = fixture("exit_raw_ed25519");
    let now = time::parse(f["recorded_at"].as_str().unwrap()).unwrap();
    let deadline = time::parse(f["notes"]["quote_deadline"].as_str().unwrap()).unwrap();
    let mut f2 = f.clone();
    // Drop the dry quote exchange; execute_exit starts with the firm quote.
    f2["exchanges"].as_array_mut().unwrap().remove(0);
    let t = Replay::new(&f2);
    t.patch_next_response(2, |b| {
        b["intentHash"] = json!("11111111111111111111111111111111")
    });
    let client = ExitClient::new(&t, OneClick::default());
    let approved =
        ApprovedExit::mainnet(SIGNER, USDC_NEAR_ASSET, 5_000_000, USDC_BASE, RECIPIENT, 1);
    let e = client
        .execute_exit(
            &signer(),
            &approved,
            100,
            deadline,
            Standard::RawEd25519,
            now,
        )
        .unwrap_err();
    assert!(matches!(e, Error::Parse(m) if m.contains("returned hash")));
}

#[test]
fn a_tampered_generate_answer_is_not_signed_or_submitted() {
    let f = fixture("exit_raw_ed25519");
    let now = time::parse(f["recorded_at"].as_str().unwrap()).unwrap();
    let deadline = time::parse(f["notes"]["quote_deadline"].as_str().unwrap()).unwrap();
    let mut f2 = f.clone();
    f2["exchanges"].as_array_mut().unwrap().remove(0);
    let t = Replay::new(&f2);
    t.patch_next_response(1, |b| {
        let p = b["intent"]["payload"]
            .as_str()
            .unwrap()
            .replace("\"receiver_id\":\"", "\"receiver_id\":\"evil");
        b["intent"]["payload"] = json!(p);
    });
    let client = ExitClient::new(&t, OneClick::default());
    let approved =
        ApprovedExit::mainnet(SIGNER, USDC_NEAR_ASSET, 5_000_000, USDC_BASE, RECIPIENT, 1);
    let e = client
        .execute_exit(
            &signer(),
            &approved,
            100,
            deadline,
            Standard::RawEd25519,
            now,
        )
        .unwrap_err();
    assert!(
        matches!(e, Error::Refused(Refusal::WrongReceiver { .. })),
        "{e}"
    );
    // submit-intent and status were never called.
    assert_eq!(t.remaining(), 2);
}

#[test]
fn parses_the_token_list() {
    let f = fixture("tokens");
    let e = &f["exchanges"][0];
    let r = HttpResponse::new(200, e["response"]["body"].to_string());
    let tokens = parse_tokens(&r).unwrap();
    assert!(tokens.len() > 100, "{}", tokens.len());
    let usdc = tokens
        .iter()
        .find(|t| t.asset_id == USDC_NEAR_ASSET)
        .unwrap();
    assert_eq!(
        (
            usdc.symbol.as_str(),
            usdc.decimals,
            usdc.blockchain.as_str()
        ),
        ("USDC", 6, "near")
    );
    let base = tokens.iter().find(|t| t.asset_id == USDC_BASE).unwrap();
    assert_eq!(base.blockchain, "base");
    // Asset ids come in several forms; all are kept as opaque strings.
    assert!(tokens.iter().any(|t| t.asset_id.starts_with("nep245:")));
    assert!(tokens.iter().any(|t| t.asset_id.starts_with("1cs_v1:")));
    assert!(
        tokens
            .iter()
            .all(|t| !t.asset_id.is_empty() && !t.symbol.is_empty())
    );
}

#[test]
fn classifies_recorded_errors() {
    let f = fixture("errors");
    let kinds: Vec<ApiErrorKind> = f["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let r = HttpResponse::new(
                e["response"]["status"].as_u64().unwrap() as u16,
                e["response"]["body"].to_string(),
            );
            match r.ok_body() {
                Err(Error::Api(a)) => a.kind,
                other => panic!("expected an API error, got {other:?}"),
            }
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ApiErrorKind::DeadlineInvalid,
            ApiErrorKind::RecipientInvalid,
            ApiErrorKind::AmountTooLow {
                min: Some(5_867_027)
            },
            ApiErrorKind::NotFound,
            ApiErrorKind::Unauthorized,
        ]
    );
}

#[test]
fn classifies_manual_error_captures() {
    let m = fixture("manual_captures");
    let c = &m["captures"];
    let kind = |k: &str| {
        let r = &c[k]["response"];
        let resp = HttpResponse::new(r["status"].as_u64().unwrap() as u16, r["body"].to_string());
        match resp.ok_body() {
            Err(Error::Api(a)) => a.kind,
            other => panic!("{k}: {other:?}"),
        }
    };
    assert_eq!(
        kind("submit_intent_unparseable"),
        ApiErrorKind::BadSignedIntent
    );
    assert_eq!(
        kind("generate_intent_quote_failed"),
        ApiErrorKind::QuoteNotPending
    );
    assert_eq!(
        kind("generate_intent_unknown_deposit"),
        ApiErrorKind::Server
    );
}

// ---------------------------------------------------------------------------
// The pre-signing check, on real quotes and real generated payloads.

struct Case {
    quote: QuoteResponse,
    generated: GeneratedIntent,
    approved: ApprovedExit,
    now: zecret_intents::time::Time,
    standard: Standard,
}

fn case(name: &str, standard: Standard) -> Case {
    let f = fixture(name);
    let ex = f["exchanges"].as_array().unwrap();
    let quote = QuoteResponse::from_value(ex[1]["response"]["body"].clone()).unwrap();
    let g = &ex[2]["response"]["body"]["intent"];
    let generated = GeneratedIntent {
        standard: g["standard"].as_str().unwrap().into(),
        payload: g["payload"].clone(),
        correlation_id: None,
    };
    let min: u128 = f["notes"]["approved_min_amount_out"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    Case {
        quote,
        generated,
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

impl Case {
    fn check(&self) -> Result<zecret_intents::CheckedIntent, Refusal> {
        check_generated_intent(
            &self.approved,
            &self.quote,
            &self.generated,
            self.standard,
            &Policy::default(),
            self.now,
        )
    }

    /// Edits the decoded payload JSON (the raw string, or the NEP-413 message).
    fn edit_payload(&mut self, f: impl FnOnce(&mut Value)) {
        match self.standard {
            Standard::RawEd25519 => {
                let mut v: Value =
                    serde_json::from_str(self.generated.payload.as_str().unwrap()).unwrap();
                f(&mut v);
                self.generated.payload = Value::String(v.to_string());
            }
            Standard::Nep413 => {
                let mut v: Value =
                    serde_json::from_str(self.generated.payload["message"].as_str().unwrap())
                        .unwrap();
                f(&mut v);
                self.generated.payload["message"] = Value::String(v.to_string());
            }
        }
    }

    fn edit_quote(&mut self, f: impl FnOnce(&mut Value)) {
        let mut raw = self.quote.raw.clone();
        f(&mut raw);
        self.quote = QuoteResponse::from_value(raw).unwrap();
    }
}

fn both() -> [Case; 2] {
    [
        case("exit_raw_ed25519", Standard::RawEd25519),
        case("exit_nep413", Standard::Nep413),
    ]
}

#[test]
fn accepts_the_real_payloads() {
    for c in both() {
        let ok = c.check().unwrap();
        assert_eq!(
            Some(ok.deposit_address.clone()),
            c.quote.quote.deposit_address
        );
        let signed = signer().sign(&ok.payload).unwrap();
        zecret_intents::signer::verify(&signed).unwrap();
    }
}

#[test]
fn refuses_a_wrong_receiver() {
    for mut c in both() {
        c.edit_payload(|v| v["intents"][0]["receiver_id"] = json!("attacker.near"));
        assert!(
            matches!(c.check(), Err(Refusal::WrongReceiver { .. })),
            "{:?}",
            c.check()
        );
    }
}

#[test]
fn refuses_a_wrong_amount() {
    for mut c in both() {
        c.edit_payload(|v| v["intents"][0]["tokens"][USDC_NEAR_ASSET] = json!("5000001"));
        assert!(matches!(c.check(), Err(Refusal::WrongAmount { .. })));
    }
    for mut c in both() {
        c.edit_payload(|v| v["intents"][0]["tokens"][USDC_NEAR_ASSET] = json!("05000000"));
        assert!(matches!(c.check(), Err(Refusal::WrongAmount { .. })));
    }
    // The user approved another amount than the quote carries.
    for mut c in both() {
        c.approved.amount_in = 4_000_000;
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "amount",
                ..
            })
        ));
    }
}

#[test]
fn refuses_a_wrong_token() {
    for mut c in both() {
        c.edit_payload(|v| {
            v["intents"][0]["tokens"] = json!({"nep141:wrap.near": "5000000"});
        });
        assert!(matches!(c.check(), Err(Refusal::WrongToken { .. })));
    }
    for mut c in both() {
        c.edit_payload(|v| {
            v["intents"][0]["tokens"]["nep141:wrap.near"] = json!("1");
        });
        assert!(matches!(c.check(), Err(Refusal::WrongToken { .. })));
    }
}

#[test]
fn refuses_a_wrong_signer() {
    for mut c in both() {
        c.edit_payload(|v| v["signer_id"] = json!("alice.near"));
        assert!(matches!(c.check(), Err(Refusal::WrongSigner { .. })));
    }
    // The real API returned this payload for signerId alice.near; our key's check
    // refuses it.
    let m = fixture("manual_captures");
    let quote =
        QuoteResponse::from_value(m["captures"]["second_firm_quote"]["response"]["body"].clone())
            .unwrap();
    let g = &m["captures"]["generate_intent_other_signer"]["response"]["body"]["intent"];
    let generated = GeneratedIntent {
        standard: "raw_ed25519".into(),
        payload: g["payload"].clone(),
        correlation_id: None,
    };
    let now = time::parse("2026-11-13T03:04:30Z").unwrap();
    let approved =
        ApprovedExit::mainnet(SIGNER, USDC_NEAR_ASSET, 5_000_000, USDC_BASE, RECIPIENT, 1);
    let r = check_generated_intent(
        &approved,
        &quote,
        &generated,
        Standard::RawEd25519,
        &Policy::default(),
        now,
    );
    assert!(
        matches!(r, Err(Refusal::WrongSigner { ref found, .. }) if found == "alice.near"),
        "{r:?}"
    );
}

#[test]
fn refuses_a_wrong_contract_standard_and_shape() {
    for mut c in both() {
        c.edit_payload(|v| {
            if v.get("verifying_contract").is_some() {
                v["verifying_contract"] = json!("intents.evil.near");
            }
        });
        if c.standard == Standard::Nep413 {
            c.generated.payload["recipient"] = json!("intents.evil.near");
        }
        assert!(matches!(c.check(), Err(Refusal::WrongContract { .. })));
    }
    for mut c in both() {
        c.standard = match c.standard {
            Standard::RawEd25519 => Standard::Nep413,
            Standard::Nep413 => Standard::RawEd25519,
        };
        assert!(matches!(c.check(), Err(Refusal::WrongStandard { .. })));
    }
    for mut c in both() {
        c.edit_payload(|v| {
            let first = v["intents"][0].clone();
            v["intents"].as_array_mut().unwrap().push(first);
        });
        assert!(matches!(c.check(), Err(Refusal::IntentCount(2))));
    }
    for mut c in both() {
        c.edit_payload(|v| v["intents"] = json!([]));
        assert!(matches!(c.check(), Err(Refusal::IntentCount(0))));
    }
    for mut c in both() {
        c.edit_payload(|v| {
            v["intents"][0] = json!({"intent":"ft_withdraw","token":"usdc.near","receiver_id":"x.near","amount":"5000000"});
        });
        assert!(matches!(c.check(), Err(Refusal::UnexpectedIntent(_))));
    }
    for mut c in both() {
        c.edit_payload(|v| {
            v["intents"][0] = json!({"intent":"token_diff","diff":{"a":"-1","b":"1"}});
        });
        assert!(matches!(c.check(), Err(Refusal::Malformed(_))));
    }
    for mut c in both() {
        c.edit_payload(|v| v["intents"][0]["msg"] = json!("call me"));
        assert!(matches!(c.check(), Err(Refusal::Malformed(_))));
    }
    for mut c in both() {
        c.edit_payload(|v| v["intents"][0]["memo"] = json!("note"));
        assert!(matches!(c.check(), Err(Refusal::Memo)));
    }
    for mut c in both() {
        c.edit_payload(|v| v["extra"] = json!(1));
        assert!(matches!(c.check(), Err(Refusal::Malformed(_))));
    }
    let mut c = case("exit_nep413", Standard::Nep413);
    c.generated.payload["callbackUrl"] = json!("https://example.com");
    assert!(matches!(c.check(), Err(Refusal::Malformed(_))));
    let mut c = case("exit_raw_ed25519", Standard::RawEd25519);
    c.generated.payload = json!({"not": "a string"});
    assert!(matches!(c.check(), Err(Refusal::Malformed(_))));
}

#[test]
fn refuses_expired_and_far_deadlines() {
    for mut c in both() {
        c.now = c.quote.quote.deadline().unwrap().unwrap();
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteExpired { .. }) | Err(Refusal::IntentExpired { .. })
        ));
    }
    // The quote's request deadline (30 min) passed: refunds have begun.
    for mut c in both() {
        c.now += chrono::Duration::minutes(31);
        assert!(matches!(c.check(), Err(Refusal::QuoteExpired { .. })));
    }
    for mut c in both() {
        c.edit_payload(|v| v["deadline"] = json!("2027-01-01T00:00:00.000Z"));
        assert!(matches!(c.check(), Err(Refusal::DeadlineTooFar { .. })));
    }
    for c in both() {
        let policy = Policy {
            max_intent_validity: chrono::Duration::hours(1),
            ..Policy::default()
        };
        let r = check_generated_intent(
            &c.approved,
            &c.quote,
            &c.generated,
            c.standard,
            &policy,
            c.now,
        );
        assert!(matches!(r, Err(Refusal::DeadlineTooFar { .. })));
    }
    for mut c in both() {
        c.edit_payload(|v| v["deadline"] = json!("2026-11-13T00:00:00.000Z"));
        assert!(matches!(c.check(), Err(Refusal::IntentExpired { .. })));
    }
}

#[test]
fn refuses_quotes_that_differ_from_the_approval() {
    for mut c in both() {
        c.approved.recipient = "0x2222222222222222222222222222222222222222".into();
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "recipient",
                ..
            })
        ));
    }
    for mut c in both() {
        c.approved.destination_asset = "nep141:sol.omft.near".into();
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "destinationAsset",
                ..
            })
        ));
    }
    for mut c in both() {
        c.approved.signer_id = "ab".repeat(32);
        assert!(matches!(
            c.check(),
            Err(Refusal::QuoteMismatch {
                field: "refundTo",
                ..
            })
        ));
    }
    for mut c in both() {
        c.approved.min_amount_out = c.quote.quote.min_amount_out().unwrap() + 1;
        assert!(matches!(c.check(), Err(Refusal::MinimumTooLow { .. })));
    }
}

#[test]
fn refuses_a_tampered_or_unsigned_quote() {
    // Changing any signed field breaks 1Click's signature.
    for mut c in both() {
        c.edit_quote(|q| {
            q["quoteRequest"]["recipient"] = json!("0x2222222222222222222222222222222222222222")
        });
        c.approved.recipient = "0x2222222222222222222222222222222222222222".into();
        assert!(matches!(c.check(), Err(Refusal::QuoteSignature)));
    }
    for mut c in both() {
        c.edit_quote(|q| q["quote"]["minAmountOut"] = json!("9999999"));
        assert!(matches!(c.check(), Err(Refusal::QuoteSignature)));
    }
    for mut c in both() {
        let real = c.quote.quote.deposit_address.clone().unwrap();
        let fake = "ff".repeat(32);
        c.edit_quote(|q| q["quote"]["depositAddress"] = json!(fake));
        c.edit_payload(|v| v["intents"][0]["receiver_id"] = json!(fake));
        assert_ne!(real, fake);
        assert!(matches!(c.check(), Err(Refusal::QuoteSignature)));
    }
    for mut c in both() {
        c.edit_quote(|q| q["signature"] = json!("ed25519:1111"));
        assert!(matches!(c.check(), Err(Refusal::QuoteSignature)));
    }
    // Another key than 1Click's.
    for c in both() {
        let policy = Policy {
            quote_key: signer().public_key(),
            ..Policy::default()
        };
        assert!(matches!(
            check_quote(&c.approved, &c.quote, &policy, c.now),
            Err(Refusal::QuoteSignature)
        ));
    }
}

#[test]
fn refuses_a_dry_quote() {
    let f = fixture("exit_raw_ed25519");
    let dry = QuoteResponse::from_value(f["exchanges"][0]["response"]["body"].clone()).unwrap();
    assert!(dry.signature_is_valid());
    let c = case("exit_raw_ed25519", Standard::RawEd25519);
    assert!(matches!(
        check_quote(&c.approved, &dry, &Policy::default(), c.now),
        Err(Refusal::QuoteIsDry)
    ));
}

#[test]
fn manual_captures_have_valid_quote_signatures_and_nonces() {
    let m = fixture("manual_captures");
    for k in ["firm_quote", "second_firm_quote"] {
        let q = QuoteResponse::from_value(m["captures"][k]["response"]["body"].clone()).unwrap();
        assert!(q.signature_is_valid(), "{k}");
        assert_eq!(
            q.quote_request.app_fee_bps(),
            1,
            "unauthenticated stablecoin app fee"
        );
    }
    let g = &m["captures"]["generate_intent_raw_ed25519"]["response"]["body"]["intent"]["payload"];
    let p: Value = serde_json::from_str(g.as_str().unwrap()).unwrap();
    let n = Nonce::from_base64(p["nonce"].as_str().unwrap()).unwrap();
    assert_eq!(n.as_versioned().unwrap().salt.to_hex(), "252812b3");
    // 1Click's intent hash is SHA-256 of the raw payload.
    let submitted = &m["captures"]["submit_intent_signed"];
    let payload = submitted["request"]["signedData"]["payload"]
        .as_str()
        .unwrap();
    use sha2::Digest as _;
    let hash = bs58::encode(sha2::Sha256::digest(payload.as_bytes())).into_string();
    assert_eq!(submitted["response"]["body"]["intentHash"], json!(hash));
    // The capture was signed with Python's ed25519 by the same seed; ed25519 is
    // deterministic, so our key gives the same public key and signature.
    let s = signer();
    assert_eq!(
        submitted["request"]["signedData"]["public_key"],
        json!(s.public_key_str())
    );
    let sig = {
        use ed25519_dalek::Signer as _;
        ed25519_dalek::SigningKey::from_bytes(&[0x5a; 32]).sign(payload.as_bytes())
    };
    assert_eq!(
        submitted["request"]["signedData"]["signature"],
        json!(format!(
            "ed25519:{}",
            bs58::encode(sig.to_bytes()).into_string()
        ))
    );
}

// ---------------------------------------------------------------------------
// Review-1 F-02: quote freshness from the signed `timestamp`. The recorded quotes are
// old, so every test passes a "now" consistent with the fixture (`Case::now` is the
// fixture's `recorded_at`); the crate never reads the clock.

/// Proof of concept from `docs/security/poc/f02_request_deadline_unsigned.rs`, kept as
/// a regression test.
#[test]
fn poc_f02_expired_request_deadline_cannot_be_hidden() {
    for mut c in both() {
        // 31 minutes after the recording: the 30-minute request deadline has passed.
        c.now += chrono::Duration::minutes(31);
        assert!(matches!(c.check(), Err(Refusal::QuoteExpired { .. })));
        // The attacker moves the unsigned request deadline into the future.
        c.edit_quote(|q| q["quoteRequest"]["deadline"] = json!("2026-12-31T00:00:00.000Z"));
        assert!(
            c.quote.signature_is_valid(),
            "the edit does not break the signature"
        );
        assert!(
            c.check().is_err(),
            "F-02: an exit was accepted for a quote whose request deadline had passed"
        );
    }
}

fn signed_timestamp(c: &Case) -> zecret_intents::time::Time {
    time::parse(c.quote.raw["timestamp"].as_str().unwrap()).unwrap()
}

#[test]
fn quote_freshness_comes_from_the_signed_timestamp() {
    for mut c in both() {
        let ts = signed_timestamp(&c);
        // The fixture's "now" is within a few seconds of the quote.
        assert!(c.check().is_ok());
        // Just under the default 5 minutes: accepted.
        c.now = ts + chrono::Duration::minutes(5) - chrono::Duration::milliseconds(1);
        assert!(c.check().is_ok(), "{:?}", c.check());
        // 5 minutes old: refused, although the request deadline (30 min) is still ahead.
        c.now = ts + chrono::Duration::minutes(5);
        assert!(
            matches!(
                c.check(),
                Err(Refusal::QuoteTooOld {
                    max_age_secs: 300,
                    ..
                })
            ),
            "{:?}",
            c.check()
        );
        // A longer policy accepts it.
        let policy = Policy {
            max_quote_age: chrono::Duration::minutes(10),
            ..Policy::default()
        };
        assert!(check_quote(&c.approved, &c.quote, &policy, c.now).is_ok());
        // Up to one minute in the future is clock difference; more is refused.
        c.now = ts - chrono::Duration::minutes(1);
        assert!(c.check().is_ok(), "{:?}", c.check());
        c.now = ts - chrono::Duration::minutes(1) - chrono::Duration::milliseconds(1);
        assert!(
            matches!(c.check(), Err(Refusal::QuoteFromFuture { .. })),
            "{:?}",
            c.check()
        );
    }
    // The timestamp is signed: moving it breaks the signature.
    for mut c in both() {
        let fresh = time::format(c.now);
        c.edit_quote(|q| q["timestamp"] = json!(fresh));
        assert!(matches!(c.check(), Err(Refusal::QuoteSignature)));
    }
}

// ---------------------------------------------------------------------------
// Review-1 hardening note H4: the generated nonce must be versioned and expire with the
// intent.

impl Case {
    fn set_nonce(&mut self, nonce: &Nonce) {
        let b64 = nonce.to_base64();
        match self.standard {
            Standard::RawEd25519 => self.edit_payload(|v| v["nonce"] = json!(b64)),
            Standard::Nep413 => self.generated.payload["nonce"] = json!(b64),
        }
    }
}

#[test]
fn refuses_a_legacy_or_long_lived_nonce() {
    for mut c in both() {
        c.set_nonce(&Nonce::legacy([7u8; 32]).unwrap());
        assert!(
            matches!(c.check(), Err(Refusal::BadNonce(_))),
            "{:?}",
            c.check()
        );
    }
    for mut c in both() {
        let ok = c.check().unwrap();
        let salt = zecret_intents::nonce::Salt::from_hex("252812b3").unwrap();
        // The same nonce, re-encoded, is accepted.
        let same = Nonce::versioned(salt, ok.deadline, [1u8; 15]).unwrap();
        c.set_nonce(&same);
        assert!(c.check().is_ok(), "{:?}", c.check());
        // A nonce that outlives the intent is refused.
        let later =
            Nonce::versioned(salt, ok.deadline + chrono::Duration::days(30), [1u8; 15]).unwrap();
        c.set_nonce(&later);
        assert!(
            matches!(c.check(), Err(Refusal::BadNonce(_))),
            "{:?}",
            c.check()
        );
    }
}
