// Security review 2: proofs of concept against the taker state machine
// (crates/zecret-core/src/taker.rs). Each test FAILS on the reviewed code and passes
// once the finding is fixed. See docs/security/review-2.md.
//
// To run: copy `common.rs` and this file to crates/zecret-core/tests/ (as
// `poc2_common.rs` and `poc2_taker.rs`), change `use super::common` below to
// `#[path = "poc2_common.rs"] mod common;`, then
//   cd crates && cargo test -p zecret-core --test poc2_taker

use std::rc::Rc;

use base64::Engine;
use serde_json::{json, Value};
use zecret_core::{
    config::Windows,
    engine::Event,
    keys,
    near::NearKey,
    store::{put_json, MemStore},
    taker::{QuoteRec, Taker, TakerStage, TakerSwap},
    Me,
};

#[path = "poc2_common.rs"]
mod common;
use common::*;

const RFQ: &str = "0102030405060708090a0b0c0d0e0f10";

fn record(stage: TakerStage, a: &pasta_curves::pallas::Scalar, near: &NearKey) -> TakerSwap {
    use group::GroupEncoding;
    TakerSwap {
        rfq_id: RFQ.into(),
        stage,
        zats: 100_000_000,
        a: hex::encode(keys::scalar_to_bytes(a)),
        s_t: "11".repeat(32),
        e_t: hex::encode(keys::scalar_to_bytes(&keys::random_scalar())),
        e_t_pub: "00".repeat(32),
        near_sk: hex::encode(near.seed()),
        taker_account: near.implicit_account(),
        reply_index: 1,
        created_at: 0,
        min_price: 1000,
        stage_at: 0,
        quote: Some(QuoteRec {
            price: "5000000".into(),
            premium: 1000,
            stipend: "30000000000000000000000".into(),
            b_pub: hex::encode((keys::g() * keys::random_scalar()).to_bytes()),
            e_m_pub: "00".repeat(32),
            windows: Windows { t0_secs: 1800, t1_secs: 600, t2_secs: 900, reveal_secs: 600 },
            height: 10,
        }),
        negated: false,
        joint_fvk: None,
        joint_address: Some(Me::generate().ua()),
        escrow_id: Some("ab".repeat(32)),
        pending_then: None,
        pending: None,
        rfq_txid: None,
        accept_txid: None,
        fund_txid: Some("f".repeat(64)),
        fund_expiry: Some(10_000),
        sweep_txid: None,
        sweep_height: None,
        commit_tx: None,
        claim_tx: None,
        claim_at: None,
        // Field added by the F2-02 fix (the signed first claim, stored before its broadcast).
        claim_raw: None,
        withdraw_tx: None,
        withdraw_at: None,
        log: vec![],
    }
}

fn swap_view(state: &str, reveal_window: u64, locked_at: Option<u64>, near: &NearKey, rec: &TakerSwap, t0: u64) -> Value {
    use group::GroupEncoding;
    let a = keys::scalar_from_hex(&rec.a).unwrap();
    json!({
        "id": rec.escrow_id, "maker": "maker.near", "token": "usdc.near", "amount": "5000000",
        "taker_pk": hex::encode(near.public_bytes()), "taker_account": near.implicit_account(),
        "a_pub": hex::encode((keys::g() * a).to_bytes()), "b_pub": rec.quote.as_ref().unwrap().b_pub,
        "t0": t0, "t1": t0 + 600, "t2": t0 + 1500, "reveal_window": reveal_window, "state": state,
        "locked_at": locked_at, "commitment": null, "a": null, "b": null
    })
}

fn open_taker(rec: &TakerSwap, mock: Rc<Mock>, store: &mut MemStore) -> Taker {
    put_json(store, &format!("swap/{}", rec.rfq_id), rec).unwrap();
    let mut tk = Taker::open(taker_cfg(&Me::generate().ua()), mock, store).unwrap();
    tk.wallet.height = 100; // the scan is at the tip; `getblockcount` answers 100
    tk
}

/// F2-02. R2r must hold at the hand-off: a taker in `Revealed` that has not sent its
/// claim yet must not publish `a` once `now + B_R >= lockedAt + W`. Here the lock's
/// window closed 50 s ago (for example the taker restarted, or its Zcash sync failed for
/// a few minutes), and the taker still broadcasts claim(a).
#[test]
fn poc_f2_02_claim_is_not_sent_after_the_reveal_deadline() {
    let near = NearKey::from_seed([4u8; 32]);
    let a = keys::random_scalar();
    let rec = record(TakerStage::Revealed, &a, &near);
    let (locked_at, w) = (1_000_000u64, 600u64);
    let now_chain = locked_at + w + 50; // past the deadline
    let view = swap_view("Locked", w, Some(locked_at), &near, &rec, locked_at - 100);
    let mock = Rc::new(Mock::new(move |m, p| match m {
        "getblockcount" => json!(100),
        "block" => block_result(now_chain),
        "query" if view_method(p).as_deref() == Some("get_swap") => view_result(&view),
        "query" if p["request_type"] == "view_access_key" => json!({"nonce": 7, "permission": "FullAccess"}),
        "send_tx" => json!({"final_execution_status": "EXECUTED", "status": {"Failure": {"ActionError": {"kind": "reveal window closed"}}},
                            "transaction_outcome": {"outcome": {"logs": [], "status": {"Failure": {}}, "executor_id": "escrow.near"}}, "receipts_outcome": []}),
        _ => Value::Null,
    }));
    let mut store = MemStore::new();
    let mut tk = open_taker(&rec, mock.clone(), &mut store);
    tk.tick(now_chain, &mut store);

    let a_hex = hex::encode(keys::scalar_to_bytes(&a));
    let leaked = mock.calls.borrow().iter().filter(|(m, _)| m == "send_tx").any(|(_, p)| {
        let raw = base64::engine::general_purpose::STANDARD.decode(p["signed_tx_base64"].as_str().unwrap()).unwrap();
        String::from_utf8_lossy(&raw).contains(&a_hex)
    });
    assert!(!leaked, "F2-02: the taker broadcast claim(a) {}s after lockedAt + W; a is now public in a reverted transaction", now_chain - locked_at - w);
}

/// F2-01. R4: the taker may erase `a` only when no joint note it funded can still need it.
/// After a refund the taker swept one note (here: a 20 000-zat note the maker sent to the
/// joint address before the funding) and, once that sweep has k_Z confirmations, erases
/// `a` although its own 1 ZEC funding note is still unspent at the joint address.
#[test]
fn poc_f2_01_recovery_does_not_erase_a_while_a_funded_note_is_unspent() {
    let near = NearKey::from_seed([5u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(TakerStage::Recovering, &a, &near);
    rec.sweep_txid = Some("d".repeat(64)); // the sweep of the maker's 20 000-zat note
    rec.sweep_height = Some(90);
    let mock = Rc::new(Mock::new(|m, _| if m == "getblockcount" { json!(100) } else { Value::Null }));
    let mut store = MemStore::new();
    let mut tk = open_taker(&rec, mock, &mut store);
    // The taker's own funding (1 ZEC + fee, tx fund_txid) is still at the joint address.
    let joint_addr = zecret_core::wallet::decode_ua(rec.joint_address.as_deref().unwrap()).unwrap();
    tk.wallet.received.push(note(RFQ, joint_addr, 100_010_000, 42, 60, &"f".repeat(64), plain_memo()));

    tk.tick(2_000_000_000, &mut store);
    let s = tk.swap().unwrap();
    let left: u64 = tk.wallet.unspent(RFQ).iter().map(|n| n.note.value().inner()).sum();
    assert!(
        !(s.stage == TakerStage::Recovered && s.a.is_empty() && left > 0),
        "F2-01: stage {:?}, a erased, but {left} zats of the taker's funding are still unspent at the joint address (now unspendable)",
        s.stage
    );
}

/// F2-03 (part 1). R1: every stored parameter must equal the quote. The escrow holds a
/// reveal window of u64::MAX (the maker chose it in `open`; the escrow only refuses 0),
/// the quote said 600 s, and the taker accepts the escrow and proceeds to fund.
#[test]
fn poc_f2_03_r1_refuses_a_reveal_window_other_than_quoted() {
    let near = NearKey::from_seed([6u8; 32]);
    let a = keys::random_scalar();
    let mut rec = record(TakerStage::Accepted, &a, &near);
    rec.fund_txid = None;
    rec.fund_expiry = None;
    let now_chain = 1_000_000u64;
    let view = swap_view("Open", u64::MAX, None, &near, &rec, now_chain + 1800);
    let mock = Rc::new(Mock::new(move |m, p| match m {
        "getblockcount" => json!(100),
        "block" => block_result(now_chain),
        "query" => match (p["request_type"].as_str(), view_method(p).as_deref()) {
            (_, Some("get_swap")) => view_result(&view),
            (_, Some("get_intents_account")) => view_result(&json!("intents.near")),
            (Some("view_account"), _) => json!({"amount": "40000000000000000000000", "storage_usage": 0, "code_hash": "CODEHASH"}),
            (Some("view_access_key_list"), _) => json!({"keys": []}),
            _ => Value::Null,
        },
        _ => Value::Null,
    }));
    let mut store = MemStore::new();
    let mut tk = open_taker(&rec, mock, &mut store);
    let t = tk.tick(now_chain, &mut store);
    let verified = t.events.iter().any(|e| matches!(e, Event::EscrowVerified { .. }));
    assert!(!verified, "F2-03: the taker verified an escrow whose reveal window is u64::MAX (quoted 600 s); events: {:?}", t.events);
}

/// F2-03 (part 2). With that escrow locked, `locked_at + reveal_window` overflows: a
/// debug build panics in `tick`, a release (wasm) build wraps and never reveals, and the
/// escrow never lets the maker refund the lock, so both legs stay frozen.
#[test]
fn poc_f2_03_huge_reveal_window_does_not_break_r2r() {
    let near = NearKey::from_seed([8u8; 32]);
    let a = keys::random_scalar();
    let rec = record(TakerStage::Committed, &a, &near);
    let now_chain = 1_000_000u64;
    let view = swap_view("Locked", u64::MAX, Some(now_chain - 20), &near, &rec, now_chain + 100);
    let mock = Rc::new(Mock::new(move |m, p| match m {
        "getblockcount" => json!(100),
        "block" => block_result(now_chain),
        "query" if view_method(p).as_deref() == Some("get_swap") => view_result(&view),
        _ => Value::Null,
    }));
    let mut store = MemStore::new();
    let mut tk = open_taker(&rec, mock, &mut store);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tk.tick(now_chain, &mut store)));
    assert!(r.is_ok(), "F2-03: tick panicked on lockedAt + W with W = u64::MAX (debug build: integer overflow)");
    let s = tk.swap().unwrap();
    assert_eq!(s.stage, TakerStage::Revealed, "F2-03: a final lock with an open (unbounded) window must be revealed; log: {:?}", s.log);
}
