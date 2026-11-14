//! The engine's 1Click exit steps (`exit::oneclick_quote`, `oneclick_submit`,
//! `oneclick_status`) replayed against answers recorded from the live 1Click API
//! (crates/zecret-intents/tests/fixtures/exit_raw_ed25519.json, 2026-11-13, an unfunded
//! key). Each request the engine makes must equal the recorded one.

use std::cell::RefCell;

use serde_json::Value;
use zecret_intents::{oneclick::OneClick, HttpRequest, HttpResponse, HttpTransport, IntentSigner};
use zecret_wasm::exit::{oneclick_quote, oneclick_status, oneclick_submit, Approval};

struct Replay {
    exchanges: Vec<Value>,
    next: RefCell<usize>,
}

impl HttpTransport for Replay {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let i = *self.next.borrow();
        *self.next.borrow_mut() += 1;
        let e = &self.exchanges[i];
        assert_eq!(r.method.as_str(), e["request"]["method"].as_str().unwrap(), "exchange {i}");
        assert_eq!(r.url, e["request"]["url"].as_str().unwrap(), "exchange {i}");
        if let Some(b) = e["request"].get("body").filter(|b| !b.is_null()) {
            assert_eq!(&r.json_body().unwrap(), b, "request body of exchange {i}");
        }
        let body = &e["response"]["body"];
        Ok(HttpResponse::new(e["response"]["status"].as_u64().unwrap() as u16, serde_json::to_vec(body).unwrap()))
    }
}

fn fixture() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../zecret-intents/tests/fixtures/exit_raw_ed25519.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn approval(min: &str) -> Approval {
    Approval {
        signer_id: IntentSigner::from_seed(&[0x5a; 32]).account_id(),
        verifying_contract: "intents.near".into(),
        origin_asset: "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1".into(),
        amount_in: "5000000".into(),
        destination_asset: "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near".into(),
        recipient: "0x1111111111111111111111111111111111111111".into(),
        min_amount_out: min.into(),
    }
}

fn now_ms(f: &Value) -> i64 {
    zecret_intents::time::parse(f["recorded_at"].as_str().unwrap()).unwrap().timestamp_millis()
}

#[test]
fn quote_sign_submit_and_status_as_recorded() {
    let f = fixture();
    let ex = f["exchanges"].as_array().unwrap()[1..].to_vec();
    let t = Replay { exchanges: ex, next: RefCell::new(0) };
    let api = OneClick::default();
    let min = f["notes"]["approved_min_amount_out"].as_str().unwrap();
    let q = oneclick_quote(&t, &api, &approval(min), now_ms(&f)).expect("firm quote");
    assert_eq!(q.amount_in, "5000000");
    assert!(!q.deposit_address.is_empty());
    let s = oneclick_submit(&t, &api, &[0x5a; 32], &approval(min), &q.raw, now_ms(&f)).expect("submit");
    assert_eq!(s.deposit_address, q.deposit_address);
    let st = oneclick_status(&t, &api, &s.deposit_address).expect("status");
    assert_eq!(st.status, "FAILED");
    assert!(st.is_final);
}

#[test]
fn a_firm_minimum_below_the_approval_is_offered_not_signed() {
    let f = fixture();
    let ex = f["exchanges"].as_array().unwrap()[1..2].to_vec();
    let t = Replay { exchanges: ex, next: RefCell::new(0) };
    let e = oneclick_quote(&t, &OneClick::default(), &approval("4999999"), now_ms(&f)).unwrap_err();
    assert_eq!(e.code, "EXIT_BELOW_MINIMUM");
    let o = e.offered.expect("offer");
    assert_eq!(o.min_amount_out, f["exchanges"][1]["response"]["body"]["quote"]["minAmountOut"].as_str().unwrap());
}

#[test]
fn a_quote_for_another_recipient_is_refused() {
    let f = fixture();
    let ex = f["exchanges"].as_array().unwrap()[1..].to_vec();
    let t = Replay { exchanges: ex, next: RefCell::new(0) };
    let api = OneClick::default();
    let min = f["notes"]["approved_min_amount_out"].as_str().unwrap();
    let q = oneclick_quote(&t, &api, &approval(min), now_ms(&f)).unwrap();
    let mut other = approval(min);
    other.recipient = "0x2222222222222222222222222222222222222222".into();
    let e = oneclick_submit(&t, &api, &[0x5a; 32], &other, &q.raw, now_ms(&f)).unwrap_err();
    assert_eq!(e.code, "EXIT_REFUSED", "{e:?}");
}

#[test]
fn a_status_answer_that_is_not_signed_for_the_deposit_address_is_not_believed() {
    let f = fixture();
    let status = f["exchanges"].as_array().unwrap().iter().find(|e| e["request"]["url"].as_str().unwrap().contains("/v0/status")).unwrap().clone();
    let deposit = f["exchanges"][1]["response"]["body"]["quote"]["depositAddress"].as_str().unwrap().to_owned();
    // The recorded answer is signed for its deposit address.
    let t = Replay { exchanges: vec![status.clone()], next: RefCell::new(0) };
    assert_eq!(oneclick_status(&t, &OneClick::default(), &deposit).unwrap().status, "FAILED");
    // A forged SUCCESS whose quote was changed: not signed, so an error to retry, not a state.
    let mut forged = status;
    forged["response"]["body"]["status"] = "SUCCESS".into();
    forged["response"]["body"]["quoteResponse"]["quote"]["amountOut"] = "1".into();
    let t = Replay { exchanges: vec![forged.clone()], next: RefCell::new(0) };
    let e = oneclick_status(&t, &OneClick::default(), &deposit).unwrap_err();
    assert_eq!((e.code, e.transient), ("NETWORK", true), "{e:?}");
    // Without the quote, the answer is not believed either.
    forged["response"]["body"].as_object_mut().unwrap().remove("quoteResponse");
    let t = Replay { exchanges: vec![forged], next: RefCell::new(0) };
    assert_eq!(oneclick_status(&t, &OneClick::default(), &deposit).unwrap_err().code, "NETWORK");
}
