// Regression tests for the taker side of security review 2 (docs/security/review-2.md)
// and the testnet findings (docs/TESTNET.md, "Problems met"), beyond the review's own
// proofs of concept in `poc2_taker.rs`:
//
// - F2-01: notes at or below the sweep fee are ignored; a refund while the funding is not
//   mined keeps `a`; after one sweep, a note that is left is swept too, the funding first.
// - F2-02: the first claim(a) is signed and stored in one tick and sent in the next; R2r
//   is checked again before the broadcast; a Zcash sync error does not stop the NEAR stages.
// - F2-03: the escrow's windows must equal the quote.
// - The maker's signed refusal ends the swap at once; a refusal not signed by the maker is
//   ignored.
// - The maker's signed limits are checked before anything is sent.
// - A claimed swap whose payout does not arrive reports `PayoutPending` with the call.
//
// Everything runs through the public API against a scripted JSON-RPC transport.

#[path = "poc2_common.rs"]
mod common;

use std::{cell::Cell, rc::Rc};

use base64::Engine;
use group::GroupEncoding;
use serde_json::{json, Value};
use zecret_core::{
    config::{MakerEntry, MakerLimits, TakerConfig, Windows},
    engine::Event,
    keys,
    msg::{self, Envelope, RefuseCode, Refusal},
    near::NearKey,
    net::{Response, Transport},
    store::{put_json, MemStore, Store},
    taker::{QuoteRec, Taker, TakerStage, TakerSwap},
    wallet::decode_ua,
    Me,
};

use common::*;

const RFQ: &str = "0102030405060708090a0b0c0d0e0f10";
const NOW: u64 = 1_000_000;

struct Fixture {
    maker_key: NearKey,
    rfq_sk: pasta_curves::pallas::Scalar,
    maker_ua: String,
    b: pasta_curves::pallas::Scalar,
    e_t: pasta_curves::pallas::Scalar,
}

fn fixture() -> Fixture {
    Fixture { maker_key: NearKey::from_seed([3u8; 32]), rfq_sk: keys::random_scalar(), maker_ua: Me::generate().ua(), b: keys::random_scalar(), e_t: keys::random_scalar() }
}

fn entry(f: &Fixture, limits: Option<MakerLimits>) -> MakerEntry {
    let mut e = MakerEntry {
        ua: f.maker_ua.clone(),
        epoch: 1,
        rfq_pub: hex::encode(msg::dh_public(&f.rfq_sk)),
        near_account: "maker.near".into(),
        near_pk: f.maker_key.public_key_str(),
        sig: String::new(),
        limits,
    };
    e.sig = hex::encode(f.maker_key.sign(&e.digest().unwrap()));
    e
}

fn cfg(f: &Fixture) -> TakerConfig {
    let mut c = taker_cfg(&f.maker_ua);
    c.maker = entry(f, None);
    c
}

fn record(f: &Fixture, stage: TakerStage, a: &pasta_curves::pallas::Scalar, near: &NearKey) -> TakerSwap {
    let mut s: TakerSwap = serde_json::from_value(json!({
        "rfq_id": RFQ, "stage": stage, "zats": 100_000_000, "a": hex::encode(keys::scalar_to_bytes(a)), "s_t": "11".repeat(32),
        "e_t": hex::encode(keys::scalar_to_bytes(&f.e_t)), "e_t_pub": hex::encode(msg::dh_public(&f.e_t)),
        "near_sk": hex::encode(near.seed()), "taker_account": near.implicit_account(), "reply_index": 1, "created_at": 0,
        "min_price": "1000", "stage_at": 0, "quote": null, "negated": false, "joint_fvk": null, "joint_address": Me::generate().ua(),
        "escrow_id": "ab".repeat(32), "pending_then": null, "pending": null, "rfq_txid": null, "accept_txid": null,
        "fund_txid": "f".repeat(64), "fund_expiry": 10_000, "sweep_txid": null, "sweep_height": null, "commit_tx": null,
        "claim_tx": null, "claim_at": null, "withdraw_tx": null, "withdraw_at": null, "log": []
    }))
    .unwrap();
    s.quote = Some(QuoteRec {
        price: "5000000".into(),
        premium: 1000,
        stipend: "30000000000000000000000".into(),
        b_pub: hex::encode((keys::g() * f.b).to_bytes()),
        e_m_pub: "00".repeat(32),
        windows: Windows { t0_secs: 1800, t1_secs: 600, t2_secs: 900, reveal_secs: 600 },
        height: 10,
    });
    s
}

/// The escrow's view of the swap in `rec`, with these deadlines (t0, t1, t2), reveal
/// window and state.
fn view(rec: &TakerSwap, near: &NearKey, state: &str, (t0, t1, t2): (u64, u64, u64), w: u64, locked_at: Option<u64>, b: Option<String>) -> Value {
    let a = keys::scalar_from_hex(&rec.a).unwrap();
    json!({
        "id": rec.escrow_id, "maker": "maker.near", "token": "usdc.near", "amount": "5000000",
        "taker_pk": hex::encode(near.public_bytes()), "taker_account": near.implicit_account(),
        "a_pub": hex::encode((keys::g() * a).to_bytes()), "b_pub": rec.quote.as_ref().unwrap().b_pub,
        "t0": t0, "t1": t1, "t2": t2, "reveal_window": w, "state": state,
        "locked_at": locked_at, "commitment": null, "a": null, "b": b
    })
}

fn ok_tx() -> Value {
    json!({"final_execution_status": "EXECUTED", "status": {"SuccessValue": ""},
           "transaction_outcome": {"id": "x", "outcome": {"logs": [], "status": {"SuccessValue": ""}, "executor_id": "escrow.near"}},
           "receipts_outcome": []})
}

/// Scripted chains: Zcash tip `tip` (or an error when `zcash_down`), NEAR final time
/// `near_time`, the escrow answering `swap`, and the taker's credit and Verifier balance.
struct Net {
    mock: Rc<Mock>,
    transport: Rc<Down>,
    near_time: Rc<Cell<u64>>,
    zcash_down: Rc<Cell<bool>>,
    credit: Rc<Cell<u64>>,
    paid: Rc<Cell<u64>>,
}

fn net(tip: u64, swap: Value) -> Net {
    let near_time = Rc::new(Cell::new(NOW));
    let zcash_down = Rc::new(Cell::new(false));
    let credit = Rc::new(Cell::new(0u64));
    let paid = Rc::new(Cell::new(0u64));
    let (nt, cr, pd) = (near_time.clone(), credit.clone(), paid.clone());
    let mock = Rc::new(Mock::new(move |m, p| match m {
        "getblockcount" => json!(tip),
        "block" => block_result(nt.get()),
        "query" => match (p["request_type"].as_str(), view_method(p).as_deref()) {
            (_, Some("get_swap")) => view_result(&swap),
            (_, Some("get_intents_account")) => view_result(&json!("intents.near")),
            (_, Some("get_credit")) => view_result(&json!({"direct": "0", "intents": cr.get().to_string()})),
            (_, Some("mt_balance_of")) => view_result(&json!(pd.get().to_string())),
            (Some("view_account"), _) => json!({"amount": "40000000000000000000000", "storage_usage": 0, "code_hash": "CODEHASH"}),
            (Some("view_access_key_list"), _) => json!({"keys": []}),
            (Some("view_access_key"), _) => json!({"nonce": 7, "permission": "FullAccess", "block_height": 1, "block_hash": "11111111111111111111111111111111"}),
            _ => Value::Null,
        },
        "send_tx" => ok_tx(),
        _ => Value::Null,
    }));
    let transport = Rc::new(Down { mock: mock.clone(), down: zcash_down.clone() });
    Net { mock, transport, near_time, zcash_down, credit, paid }
}

/// The mock, with the Zcash node unreachable while `down` is set.
struct Down {
    mock: Rc<Mock>,
    down: Rc<Cell<bool>>,
}

impl Transport for Down {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> anyhow::Result<Response> {
        if url.contains("zebra") && self.down.get() {
            anyhow::bail!("connection refused (test: the Zcash node is down)");
        }
        self.mock.post(url, headers, body)
    }
}

fn open(f: &Fixture, rec: &TakerSwap, n: &Net, tip: u64, store: &mut MemStore) -> Taker {
    put_json(store, &format!("swap/{}", rec.rfq_id), rec).unwrap();
    let mut tk = Taker::open(cfg(f), n.transport.clone(), store).unwrap();
    tk.wallet.height = tip;
    tk
}

fn sends(n: &Net) -> Vec<String> {
    n.mock.calls.borrow().iter().filter(|(m, _)| m == "send_tx").map(|(_, p)| p["signed_tx_base64"].as_str().unwrap().to_string()).collect()
}

// ------------------------------------------------------------------------- F2-01

/// A note at the joint address that cannot pay its own sweep fee (here 10 000 zats, the
/// fee itself) is ignored: once the real funding is swept with k_Z confirmations, the swap
/// ends `Recovered` and needs no NEAR call for that.
#[test]
fn f2_01_dust_note_does_not_hold_the_recovery() {
    let f = fixture();
    let near = NearKey::from_seed([5u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(&f, TakerStage::Recovering, &a, &near);
    rec.fund_txid = None; // H10: the decision must not depend on fund_txid
    rec.sweep_txid = Some("d".repeat(64));
    rec.sweep_height = Some(90);
    let n = net(100, Value::Null);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    let joint = decode_ua(rec.joint_address.as_deref().unwrap()).unwrap();
    tk.wallet.received.push(note(RFQ, joint, 10_000, 42, 60, &"e".repeat(64), plain_memo()));
    tk.tick(NOW, &mut store);
    let s = tk.swap().unwrap();
    assert_eq!(s.stage, TakerStage::Recovered, "log: {:?}", s.log);
    assert!(s.a.is_empty(), "a is erased in the terminal stage");
    assert_eq!(n.mock.count("query"), 0, "no NEAR call was needed");
}

/// R4: the maker refunds while the taker's funding is broadcast but not mined. The taker
/// keeps `a` (the stage stays where it was) until the funding is mined or has expired unmined by k_Z
/// blocks; only then does it end with nothing to recover.
#[test]
fn f2_01_refund_before_the_funding_is_mined_keeps_a() {
    let f = fixture();
    let near = NearKey::from_seed([6u8; 32]);
    let a = keys::random_scalar();
    let rec = record(&f, TakerStage::Funded, &a, &near);
    let b = Some(hex::encode(keys::scalar_to_bytes(&f.b)));
    let v = view(&rec, &near, "Refunded", (NOW - 100, NOW + 600, NOW + 1500), 600, None, b.clone());
    let n = net(100, v);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    tk.tick(NOW, &mut store);
    let s = tk.swap().unwrap().clone();
    assert_eq!(s.stage, TakerStage::Funded, "log: {:?}", s.log);
    assert!(!s.a.is_empty(), "a must be kept while the funding may still be mined");
    assert!(s.log.last().unwrap().contains("keeping a"));

    // Past expiry (10 000) + k_Z (2): nothing more can arrive.
    let v = view(&rec, &near, "Refunded", (NOW - 100, NOW + 600, NOW + 1500), 600, None, b);
    let n2 = net(10_003, v);
    let mut store2 = MemStore::new();
    let mut tk2 = open(&f, &s, &n2, 10_003, &mut store2);
    tk2.tick(NOW + 10, &mut store2);
    let s2 = tk2.swap().unwrap();
    assert_eq!(s2.stage, TakerStage::Recovered, "log: {:?}", s2.log);
}

/// After one confirmed recovery sweep, a note above the fee is left: the taker keeps `a`
/// and starts the next sweep with the taker's own funding first (the maker's extra note
/// is swept after it).
#[test]
fn f2_01_second_note_is_swept_funding_first() {
    let f = fixture();
    let near = NearKey::from_seed([7u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(&f, TakerStage::Recovering, &a, &near);
    rec.sweep_txid = Some("d".repeat(64));
    rec.sweep_height = Some(90);
    let b = Some(hex::encode(keys::scalar_to_bytes(&f.b)));
    let v = view(&rec, &near, "Refunded", (NOW - 100, NOW + 600, NOW + 1500), 600, None, b);
    let n = net(100, v);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    let joint = decode_ua(rec.joint_address.as_deref().unwrap()).unwrap();
    // A larger note from the maker (H10) and the taker's own funding.
    tk.wallet.received.push(note(RFQ, joint, 200_000_000, 41, 59, &"c".repeat(64), plain_memo()));
    tk.wallet.received.push(note(RFQ, joint, 100_010_000, 42, 60, &"f".repeat(64), plain_memo()));
    let t1 = tk.tick(NOW, &mut store);
    let s = tk.swap().unwrap().clone();
    assert_eq!(s.stage, TakerStage::Recovering);
    assert!(!s.a.is_empty());
    assert!(s.log.iter().any(|l| l.contains("joint note(s) above the fee are left")), "log: {:?}", s.log);
    assert!(t1.events.iter().any(|e| matches!(e, Event::Activity { .. })), "the next sweep is announced: {:?}", t1.events);
    // The next tick builds the sweep. The test wallet has no note tree, so the build
    // fails, but the line before it names the note it chose.
    let t2 = tk.tick(NOW + 1, &mut store);
    let chose = t2.events.iter().find_map(|e| match e {
        Event::Log { line, .. } if line.contains("sweeping the joint note") => Some(line.clone()),
        _ => None,
    });
    let chose = chose.unwrap_or_else(|| panic!("no sweep chosen: {:?}", t2.events));
    assert!(chose.contains(&"f".repeat(64)) && chose.contains("100010000"), "funding first: {chose}");
    assert!(!tk.swap().unwrap().a.is_empty());
}

// ------------------------------------------------------------------------- F2-02

/// The first claim(a) is signed and stored in one tick (nothing sent), and the stored
/// bytes are broadcast in the next.
#[test]
fn f2_02_first_claim_is_stored_then_sent() {
    let f = fixture();
    let near = NearKey::from_seed([8u8; 32]);
    let a = keys::random_scalar();
    let rec = record(&f, TakerStage::Revealed, &a, &near);
    let v = view(&rec, &near, "Locked", (NOW - 900, NOW - 300, NOW + 600), 600, Some(NOW - 20), None);
    let n = net(100, v);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    tk.tick(NOW, &mut store);
    let s = tk.swap().unwrap().clone();
    assert!(sends(&n).is_empty(), "nothing is sent in the tick that signs");
    let raw = s.claim_raw.clone().expect("signed claim stored");
    assert!(s.claim_at.is_none());
    let stored: TakerSwap = serde_json::from_slice(&store.get(&format!("swap/{RFQ}")).unwrap().unwrap()).unwrap();
    assert_eq!(stored.claim_raw.as_deref(), Some(raw.as_str()), "the signed claim is in the store before it is sent");
    tk.tick(NOW + 1, &mut store);
    let sent = sends(&n);
    assert_eq!(sent.len(), 1);
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(&sent[0]).unwrap(), hex::decode(&raw).unwrap(), "the stored bytes are sent");
    assert!(tk.swap().unwrap().claim_at.is_some());
}

/// The claim is signed and stored, then the reveal deadline comes too close before the
/// broadcast (for example the tab was closed between the two ticks): R2r fails at the
/// hand-off, nothing is sent, the stored claim is dropped and the stage goes back to
/// Committed.
#[test]
fn f2_02_r2r_is_checked_again_before_the_broadcast() {
    let f = fixture();
    let near = NearKey::from_seed([9u8; 32]);
    let a = keys::random_scalar();
    let rec = record(&f, TakerStage::Revealed, &a, &near);
    let (locked_at, w) = (NOW - 20, 600u64);
    let v = view(&rec, &near, "Locked", (NOW - 900, NOW - 300, NOW + 600), w, Some(locked_at), None);
    let n = net(100, v);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    tk.tick(NOW, &mut store);
    assert!(tk.swap().unwrap().claim_raw.is_some());
    // NEAR time moves to lockedAt + W - B_R (B_R = 110): no longer in time.
    n.near_time.set(locked_at + w - 110);
    tk.tick(NOW + 500, &mut store);
    let s = tk.swap().unwrap();
    assert!(sends(&n).is_empty(), "claim(a) must not be sent once R2r fails");
    assert_eq!(s.stage, TakerStage::Committed, "log: {:?}", s.log);
    assert!(s.claim_raw.is_none() && s.claim_tx.is_none());
    assert!(!s.a.is_empty());
}

/// A Zcash sync error does not stop the NEAR stages: in Revealed the claim is still
/// signed and sent, with a warning about the sync.
#[test]
fn f2_02_zcash_outage_does_not_stall_the_claim() {
    let f = fixture();
    let near = NearKey::from_seed([10u8; 32]);
    let a = keys::random_scalar();
    let rec = record(&f, TakerStage::Revealed, &a, &near);
    let v = view(&rec, &near, "Locked", (NOW - 900, NOW - 300, NOW + 600), 600, Some(NOW - 20), None);
    let n = net(100, v);
    n.zcash_down.set(true);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 90, &mut store);
    let t = tk.tick(NOW, &mut store);
    assert!(t.events.iter().any(|e| matches!(e, Event::Warning { message, .. } if message.contains("Zcash sync"))), "{:?}", t.events);
    assert!(tk.swap().unwrap().claim_raw.is_some(), "the claim was signed despite the Zcash outage");
    tk.tick(NOW + 1, &mut store);
    assert_eq!(sends(&n).len(), 1, "and sent");
}

/// With a Zcash outage, a refund seen in Funded does not end the swap: the recovery needs
/// the Zcash view, so `a` is kept and the stage does not become Recovered.
#[test]
fn f2_02_zcash_outage_never_ends_a_recovery() {
    let f = fixture();
    let near = NearKey::from_seed([11u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(&f, TakerStage::Funded, &a, &near);
    rec.fund_txid = None;
    rec.fund_expiry = None;
    let b = Some(hex::encode(keys::scalar_to_bytes(&f.b)));
    let v = view(&rec, &near, "Refunded", (NOW - 100, NOW + 600, NOW + 1500), 600, None, b);
    let n = net(100, v);
    n.zcash_down.set(true);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 90, &mut store);
    tk.tick(NOW, &mut store);
    let s = tk.swap().unwrap();
    assert_eq!(s.stage, TakerStage::Funded);
    assert!(!s.a.is_empty());
}

// ------------------------------------------------------------------------- F2-03

/// The escrow's windows must equal the quote: a claim window one second longer, or a
/// reveal window one second shorter, stops the swap before the funding.
#[test]
fn f2_03_escrow_windows_must_equal_the_quote() {
    for (what, t1_extra, w) in [("claim window", 1u64, 600u64), ("reveal window", 0, 599), ("punish window", 0, 600)] {
        let f = fixture();
        let near = NearKey::from_seed([12u8; 32]);
        let a = keys::random_scalar();
        let mut rec = record(&f, TakerStage::Accepted, &a, &near);
        rec.fund_txid = None;
        rec.fund_expiry = None;
        let t0 = NOW + 1800;
        let t1 = t0 + 600 + t1_extra;
        let t2 = if what == "punish window" { t1 + 901 } else { t1 + 900 };
        let n = net(100, view(&rec, &near, "Open", (t0, t1, t2), w, None, None));
        let mut store = MemStore::new();
        let mut tk = open(&f, &rec, &n, 100, &mut store);
        tk.tick(NOW, &mut store);
        let s = tk.swap().unwrap();
        match &s.stage {
            TakerStage::Aborted(why) => assert!(why.contains(what), "{what}: {why}"),
            other => panic!("{what}: stage {other:?}"),
        }
    }
    // The quoted values exactly: no abort.
    let f = fixture();
    let near = NearKey::from_seed([13u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(&f, TakerStage::Accepted, &a, &near);
    rec.fund_txid = None;
    let n = net(100, view(&rec, &near, "Open", (NOW + 1800, NOW + 2400, NOW + 3300), 600, None, None));
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    let t = tk.tick(NOW, &mut store);
    assert!(t.events.iter().any(|e| matches!(e, Event::EscrowVerified { .. })), "{:?}", tk.swap().unwrap().log);
}

// ------------------------------------------------------------------------- refusal

fn refusal_memo(f: &Fixture, signer: &NearKey, code: RefuseCode) -> [u8; 512] {
    let id: [u8; 16] = hex::decode(RFQ).unwrap().try_into().unwrap();
    let mut r = Refusal { code, min_zats: 100_000, max_zats: 10_000_000, sig: [0; 64] };
    r.sig = signer.sign(&r.sig_digest(&id));
    let eph = msg::dh_public(&f.e_t);
    let maker_pub = msg::dh_public(&f.rfq_sk);
    let ct = msg::seal(b"refuse", &f.rfq_sk, &eph, &eph, &maker_pub, &r.to_bytes()).unwrap();
    Envelope::Refuse { rfq_id: id, ct }.encode()
}

/// The maker's signed refusal ends the swap at once with the reason and code; nothing is
/// funded and no premium is paid.
#[test]
fn signed_refusal_stops_the_taker() {
    let f = fixture();
    let near = NearKey::from_seed([14u8; 32]);
    let a = keys::random_scalar();
    let rec = record(&f, TakerStage::RfqSent, &a, &near);
    let n = net(100, Value::Null);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    let me = tk.me.address();
    tk.wallet.received.push(note("wallet", me, 0, 7, 99, &"a".repeat(64), refusal_memo(&f, &f.maker_key, RefuseCode::OverLimit)));
    let t = tk.tick(NOW, &mut store);
    let s = tk.swap().unwrap();
    match &s.stage {
        TakerStage::Aborted(why) => assert!(why.starts_with("MAKER REFUSED (over_limit)") && why.contains("10000000"), "{why}"),
        other => panic!("stage {other:?}"),
    }
    assert!(t.events.iter().any(|e| matches!(e, Event::Refused { code, max_zats: 10_000_000, .. } if code == "over_limit")), "{:?}", t.events);
}

/// A refusal signed by another key is ignored: the taker keeps waiting for the quote.
#[test]
fn refusal_not_signed_by_the_maker_is_ignored() {
    let f = fixture();
    let near = NearKey::from_seed([15u8; 32]);
    let a = keys::random_scalar();
    let rec = record(&f, TakerStage::RfqSent, &a, &near);
    let n = net(100, Value::Null);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    let me = tk.me.address();
    tk.wallet.received.push(note("wallet", me, 0, 7, 99, &"a".repeat(64), refusal_memo(&f, &NearKey::generate(), RefuseCode::Busy)));
    tk.tick(10, &mut store);
    assert_eq!(tk.swap().unwrap().stage, TakerStage::RfqSent);
}

// ------------------------------------------------------------------------- limits

/// The signed limits are checked in `start`, before anything is stored or sent. Limits
/// changed after signing break the signature.
#[test]
fn published_limits_are_checked_before_anything_is_sent() {
    let f = fixture();
    let limits = MakerLimits { min_zats: 100_000, max_zats: 10_000_000, premium_zats: 1000 };
    let n = net(100, Value::Null);
    let mk = |e: MakerEntry| {
        let mut c = cfg(&f);
        c.maker = e;
        let mut store = MemStore::new();
        let tk = Taker::open(c, n.transport.clone(), &mut store).unwrap();
        (tk, store)
    };
    // Above the limit: refused, nothing stored.
    let (mut tk, mut store) = mk(entry(&f, Some(limits)));
    let e = tk.start(50_000_000, NOW, &mut store).unwrap_err().to_string();
    assert!(e.contains("outside the maker's published limits"), "{e}");
    assert!(store.keys("swap/").unwrap().is_empty());
    // Inside: starts.
    tk.start(1_000_000, NOW, &mut store).unwrap();
    // A premium above the taker's limit (max_premium 50 000).
    let (mut tk, mut store) = mk(entry(&f, Some(MakerLimits { premium_zats: 60_000, ..limits })));
    let e = tk.start(1_000_000, NOW, &mut store).unwrap_err().to_string();
    assert!(e.contains("published premium"), "{e}");
    // Limits widened after signing: the signature check fails.
    let mut forged = entry(&f, Some(limits));
    forged.limits = Some(MakerLimits { max_zats: 100_000_000, ..limits });
    let (mut tk, mut store) = mk(forged);
    let e = tk.start(50_000_000, NOW, &mut store).unwrap_err().to_string();
    assert!(e.contains("not signed by"), "{e}");
    // Limits removed: the old digest does not match the signature either.
    let mut stripped = entry(&f, Some(limits));
    stripped.limits = None;
    let (mut tk, mut store) = mk(stripped);
    assert!(tk.start(50_000_000, NOW, &mut store).unwrap_err().to_string().contains("not signed by"));
}

// ------------------------------------------------------------------------- payout pending

/// The claim is final, the maker does not withdraw: after `payout_wait_secs` the taker
/// reports PayoutPending with the exact call. It does not send a withdraw it cannot pay
/// for. When the payout arrives, the swap completes.
#[test]
fn payout_pending_is_reported_with_the_call() {
    let f = fixture();
    let near = NearKey::from_seed([16u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(&f, TakerStage::Claimed, &a, &near);
    rec.stage_at = NOW;
    let n = net(100, Value::Null);
    n.credit.set(5_000_000);
    let mut store = MemStore::new();
    let mut tk = open(&f, &rec, &n, 100, &mut store);
    tk.tick(NOW + 10, &mut store);
    assert_eq!(tk.swap().unwrap().stage, TakerStage::Claimed);
    assert!(sends(&n).is_empty(), "the stipend cannot pay a withdraw; the taker does not try");
    let t = tk.tick(NOW + 301, &mut store);
    let s = tk.swap().unwrap();
    assert_eq!(s.stage, TakerStage::PayoutPending, "log: {:?}", s.log);
    let ev = t.events.iter().find_map(|e| match e {
        Event::PayoutPending { receiver, method, args, gas, credit, .. } => Some((receiver.clone(), method.clone(), args.clone(), gas.clone(), credit.clone())),
        _ => None,
    });
    let (receiver, method, args, gas, credit) = ev.expect("payout_pending event");
    assert_eq!((receiver.as_str(), method.as_str(), gas.as_str(), credit.as_str()), ("escrow.near", "withdraw", "200000000000000", "5000000"));
    let args: Value = serde_json::from_str(&args).unwrap();
    assert_eq!(args, json!({"token": "usdc.near", "owner": near.implicit_account()}));
    assert!(s.log.last().unwrap().contains("Any NEAR account can send it"));
    // Someone sends withdraw: the payout arrives.
    n.credit.set(0);
    n.paid.set(5_000_000);
    tk.tick(NOW + 320, &mut store);
    assert_eq!(tk.swap().unwrap().stage, TakerStage::Completed);
}
