//! The engine's 1Click exit through the site's proxy (`/api/oneclick`, docs/ONECLICK_PROXY.md).
//! The proxy is untrusted transport: a proxy that edits 1Click's answers must not make the
//! engine sign or submit anything it would not sign with 1Click itself. The answers are the
//! ones recorded from the live 1Click API (crates/zecret-intents/tests/fixtures/
//! exit_raw_ed25519.json, 2026-11-13, an unfunded key); the hostile proxy edits them.

use std::cell::RefCell;

use serde_json::Value;
use zecret_intents::{oneclick::OneClick, HttpRequest, HttpResponse, HttpTransport, IntentSigner};
use zecret_wasm::exit::{oneclick_quote, oneclick_status, oneclick_submit, Approval};

const PROXY: &str = "https://site.example/api/oneclick";
const UPSTREAM: &str = "https://1click.chaindefuser.com";

type Edit = Box<dyn Fn(&str, &mut Value)>;
/// A named edit of one answer body.
type BodyEdits = Vec<(&'static str, Box<dyn Fn(&mut Value)>)>;

/// A proxy at `PROXY` that answers with the recorded 1Click answers, edited by `edit`.
struct Proxy {
    exchanges: Vec<Value>,
    next: RefCell<usize>,
    edit: Edit,
    /// The upstream paths the engine called, in order.
    calls: RefCell<Vec<String>>,
}

impl Proxy {
    fn new(edit: impl Fn(&str, &mut Value) + 'static) -> Self {
        let f = fixture();
        Self { exchanges: f["exchanges"].as_array().unwrap()[1..].to_vec(), next: RefCell::new(0), edit: Box::new(edit), calls: RefCell::new(vec![]) }
    }
    fn called(&self, path: &str) -> bool {
        self.calls.borrow().iter().any(|c| c.starts_with(path))
    }
}

impl HttpTransport for Proxy {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let path = r.url.strip_prefix(PROXY).unwrap_or_else(|| panic!("the engine called {} outside the configured base", r.url));
        assert!(r.headers.iter().all(|(k, _)| !k.eq_ignore_ascii_case("x-api-key") && !k.eq_ignore_ascii_case("authorization")), "the browser sends no key");
        self.calls.borrow_mut().push(path.to_owned());
        let i = *self.next.borrow();
        *self.next.borrow_mut() += 1;
        let e = &self.exchanges[i];
        assert_eq!(format!("{UPSTREAM}{path}"), e["request"]["url"].as_str().unwrap(), "exchange {i}");
        if let Some(b) = e["request"].get("body").filter(|b| !b.is_null()) {
            assert_eq!(&r.json_body().unwrap(), b, "request body of exchange {i}");
        }
        let mut body = e["response"]["body"].clone();
        (self.edit)(path, &mut body);
        Ok(HttpResponse::new(e["response"]["status"].as_u64().unwrap() as u16, serde_json::to_vec(&body).unwrap()))
    }
}

fn fixture() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../zecret-intents/tests/fixtures/exit_raw_ed25519.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn approval() -> Approval {
    let f = fixture();
    Approval {
        signer_id: IntentSigner::from_seed(&[0x5a; 32]).account_id(),
        verifying_contract: "intents.near".into(),
        origin_asset: "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1".into(),
        amount_in: "5000000".into(),
        destination_asset: "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near".into(),
        recipient: "0x1111111111111111111111111111111111111111".into(),
        min_amount_out: f["notes"]["approved_min_amount_out"].as_str().unwrap().into(),
    }
}

fn now_ms() -> i64 {
    zecret_intents::time::parse(fixture()["recorded_at"].as_str().unwrap()).unwrap().timestamp_millis()
}

fn api() -> OneClick {
    OneClick { base_url: PROXY.into(), api_key: None }
}

/// Edits the payload string of a generate-intent answer.
fn edit_payload(body: &mut Value, f: impl Fn(&mut Value)) {
    let mut p: Value = serde_json::from_str(body["intent"]["payload"].as_str().unwrap()).unwrap();
    f(&mut p);
    body["intent"]["payload"] = Value::String(p.to_string());
}

#[test]
fn an_honest_proxy_carries_the_whole_exit() {
    let t = Proxy::new(|_, _| {});
    let q = oneclick_quote(&t, &api(), &approval(), now_ms()).expect("firm quote through the proxy");
    let s = oneclick_submit(&t, &api(), &[0x5a; 32], &approval(), &q.raw, now_ms()).expect("submit through the proxy");
    let st = oneclick_status(&t, &api(), &s.deposit_address).expect("status through the proxy");
    assert_eq!(st.status, "FAILED");
    assert_eq!(*t.calls.borrow(), ["/v0/quote", "/v0/generate-intent", "/v0/submit-intent", &format!("/v0/status?depositAddress={}", s.deposit_address)]);
}

#[test]
fn a_quote_the_proxy_edited_is_refused() {
    // Each edit breaks 1Click's signature over the quote.
    let edits: BodyEdits = vec![
        ("a better price", Box::new(|b| b["quote"]["amountOut"] = "9999999".into())),
        ("a higher guaranteed minimum", Box::new(|b| b["quote"]["minAmountOut"] = "9999999".into())),
        ("another deposit address", Box::new(|b| b["quote"]["depositAddress"] = "ab".repeat(32).into())),
        ("no signature", Box::new(|b| { b.as_object_mut().unwrap().remove("signature"); })),
    ];
    for (what, edit) in edits {
        let t = Proxy::new(move |path, b| if path == "/v0/quote" { edit(b) });
        let e = oneclick_quote(&t, &api(), &approval(), now_ms()).unwrap_err();
        assert_eq!(e.code, "EXIT_REFUSED", "{what}: {e:?}");
        assert!(!t.called("/v0/generate-intent"), "{what}");
    }
}

#[test]
fn a_payload_the_proxy_edited_is_never_signed_or_submitted() {
    let q = oneclick_quote(&Proxy::new(|_, _| {}), &api(), &approval(), now_ms()).unwrap();
    let edits: BodyEdits = vec![
        ("transfer to the proxy's account", Box::new(|p| p["intents"][0]["receiver_id"] = "proxy.near".into())),
        ("a larger amount", Box::new(|p| p["intents"][0]["tokens"]["nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"] = "5000001".into())),
        ("another token", Box::new(|p| p["intents"][0]["tokens"] = serde_json::json!({ "nep141:wrap.near": "5000000" }))),
        ("another Verifier", Box::new(|p| p["verifying_contract"] = "intents.proxy.near".into())),
        ("a second intent", Box::new(|p| { let first = p["intents"][0].clone(); p["intents"].as_array_mut().unwrap().push(first); })),
        ("a far deadline", Box::new(|p| p["deadline"] = "2030-01-01T00:00:00.000Z".into())),
    ];
    for (what, edit) in edits {
        let t = Proxy::new(move |path, b| if path == "/v0/generate-intent" { edit_payload(b, &edit) });
        // The proxy serves the quote again first: skip it, the test starts at generate-intent.
        *t.next.borrow_mut() = 1;
        let e = oneclick_submit(&t, &api(), &[0x5a; 32], &approval(), &q.raw, now_ms()).unwrap_err();
        assert_eq!(e.code, "EXIT_REFUSED", "{what}: {e:?}");
        assert!(!t.called("/v0/submit-intent"), "{what}: a signed intent left the browser");
    }
    // Another signing standard is refused too.
    let t = Proxy::new(|path, b| if path == "/v0/generate-intent" { b["intent"]["standard"] = "nep413".into() });
    *t.next.borrow_mut() = 1;
    assert_eq!(oneclick_submit(&t, &api(), &[0x5a; 32], &approval(), &q.raw, now_ms()).unwrap_err().code, "EXIT_REFUSED");
    assert!(!t.called("/v0/submit-intent"));
}

#[test]
fn a_status_the_proxy_forged_is_not_believed() {
    let deposit = fixture()["exchanges"][1]["response"]["body"]["quote"]["depositAddress"].as_str().unwrap().to_owned();
    let t = Proxy::new(|path, b| if path.starts_with("/v0/status") {
        b["status"] = "SUCCESS".into();
        b["quoteResponse"]["quote"]["amountOut"] = "1".into();
    });
    *t.next.borrow_mut() = 3;
    let e = oneclick_status(&t, &api(), &deposit).unwrap_err();
    assert_eq!((e.code, e.transient), ("NETWORK", true), "{e:?}");
}

#[test]
fn a_status_for_another_signed_quote_is_not_believed() {
    // The proxy answers the status call with a real, validly signed 1Click quote of another
    // swap (another deposit address): the signature checks out, the binding does not.
    let deposit = fixture()["exchanges"][1]["response"]["body"]["quote"]["depositAddress"].as_str().unwrap().to_owned();
    let other: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../zecret-intents/tests/fixtures/exit_nep413.json")).unwrap()).unwrap();
    let other_quote = other["exchanges"][1]["response"]["body"].clone();
    assert_ne!(other_quote["quote"]["depositAddress"].as_str().unwrap(), deposit);
    let t = Proxy::new(move |path, b| if path.starts_with("/v0/status") {
        b["status"] = "SUCCESS".into();
        b["quoteResponse"] = other_quote.clone();
    });
    *t.next.borrow_mut() = 3;
    let e = oneclick_status(&t, &api(), &deposit).unwrap_err();
    assert_eq!((e.code, e.transient), ("NETWORK", true), "{e:?}");
}

#[test]
fn the_status_word_itself_is_not_signed() {
    // With the signed quote left intact, the proxy can change the status word and the swap
    // details (amount out, destination transactions, refund). This layer passes them on; the
    // web engine believes SUCCESS only when the key's Verifier balance dropped
    // (exitPlan.ts believeSuccess), and REFUNDED or FAILED only as `wait` until tokens are
    // back under the key (afterFinalStatus). This test records that limit.
    let deposit = fixture()["exchanges"][1]["response"]["body"]["quote"]["depositAddress"].as_str().unwrap().to_owned();
    for word in ["SUCCESS", "REFUNDED", "PENDING_DEPOSIT"] {
        let t = Proxy::new(move |path, b| if path.starts_with("/v0/status") {
            b["status"] = word.into();
            b["swapDetails"]["amountOut"] = "999999999".into();
        });
        *t.next.borrow_mut() = 3;
        let st = oneclick_status(&t, &api(), &deposit).expect("an intact signed quote binds the answer");
        assert_eq!(st.status, word);
        assert_eq!(st.amount_out.as_deref(), Some("999999999"));
    }
}

#[test]
fn a_replayed_quote_is_bounded_by_the_approval() {
    // The proxy answers the firm quote call with a genuine, signed 1Click answer that is not
    // the answer to this request. The dry quote of the same swap is refused. Another firm
    // quote that 1Click signed for the same approval (same signer, amount, token, recipient
    // and refund account, recorded 7 s later, another deposit address) is accepted: it is a
    // quote for exactly what the user approved, at or above the approved minimum, and the
    // signed timestamp bounds how old it may be (`Policy::max_quote_age`,
    // tested in crates/zecret-intents). So a replay cannot move tokens anywhere
    // the user did not approve.
    let f = fixture();
    let dry = f["exchanges"][0]["response"]["body"].clone();
    let t = Proxy::new(move |path, b| if path == "/v0/quote" { *b = dry.clone() });
    assert_eq!(oneclick_quote(&t, &api(), &approval(), now_ms()).unwrap_err().code, "EXIT_REFUSED");
    assert!(!t.called("/v0/generate-intent"));

    let other: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../zecret-intents/tests/fixtures/exit_nep413.json")).unwrap()).unwrap();
    let other_firm = other["exchanges"][1]["response"]["body"].clone();
    let other_deposit = other_firm["quote"]["depositAddress"].as_str().unwrap().to_owned();
    let t = Proxy::new(move |path, b| if path == "/v0/quote" { *b = other_firm.clone() });
    let q = oneclick_quote(&t, &api(), &approval(), now_ms()).expect("a signed quote for the same approval");
    assert_eq!(q.deposit_address, other_deposit);
}
