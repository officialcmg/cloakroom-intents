// Regression tests for security review 2 (docs/security/review-2.md): F2-04 (unpaid
// requests must not grow the maker's work per tick or delay its refunds), H2 (a Zcash
// sync failure must not stop the maker's NEAR duties), H5 (no test-only behaviour from a
// production configuration) and NV2-02 (the premium must cover what an accepted request
// costs the maker).
//
// Everything runs through the public `Maker::tick` API against a scripted JSON-RPC
// transport. Nothing here touches a network.

#[path = "poc2_common.rs"]
mod common;

use std::{
    cell::Cell,
    collections::BTreeSet,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use group::GroupEncoding;
use serde_json::{json, Value};
use zecret_core::{
    config::{MakerConfig, Timing},
    engine::Event,
    keys,
    maker::{Maker, MakerStage, KEY_NEAR, KEY_RFQ, KEY_SEEN},
    msg::{self, Envelope, Rfq},
    near::NearKey,
    net::{Response, Transport},
    store::{get_json, put_json, MemStore, Store},
    wallet::empty_memo,
    Me,
};

use common::*;

const T1: u64 = 2_000_000_000;
const T2: u64 = T1 + 900;
const VICTIM: &str = "ffffffffffffffffffffffffffffffff";

fn timing() -> Timing {
    serde_json::from_value(json!({"inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2, "margin_secs": 0, "fund_secs": 900, "k_z": 3})).unwrap()
}

fn cfg_json() -> Value {
    json!({
        "chains": {"zebra_rpc": "http://zebra.invalid", "nu7_height": 2, "near_rpc": "http://near.invalid",
                   "escrow": "escrow.near", "token": "usdc.near", "intents": "intents.near"},
        "timing": {"inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2, "margin_secs": 0, "fund_secs": 900, "k_z": 3},
        "near_account": "maker.near", "price_per_zec": "30000000", "min_zats": 100000, "max_zats": 10000000,
        "premium_zats": 1000, "stipend": "30000000000000000000000",
        "windows": {"t0_secs": 1800, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600}
    })
}

fn cfg() -> MakerConfig {
    serde_json::from_value(cfg_json()).unwrap()
}

/// The scripted chains: Zebra at tip 100 (the Zcash RPC can be switched off), and a NEAR
/// node whose final block is at `near_time` and whose escrow holds the victim swap in
/// state `escrow_state`.
struct Chains {
    mock: Rc<Mock>,
    near_time: Rc<Cell<u64>>,
    zcash_down: Cell<bool>,
}

impl Transport for Chains {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        if url.contains("zebra") && self.zcash_down.get() {
            self.mock.calls.borrow_mut().push(("zcash-down".into(), Value::Null));
            bail!("connection refused (test: the Zcash node is down)");
        }
        self.mock.post(url, headers, body)
    }
}

fn chains(escrow_state: &'static str) -> Rc<Chains> {
    let near_time = Rc::new(Cell::new(T1 + 10));
    let nt = near_time.clone();
    let mock = Rc::new(Mock::new(move |m, p| match m {
        "getblockcount" => json!(100),
        "block" => block_result(nt.get()),
        "query" => match p["request_type"].as_str() {
            Some("call_function") if p["method_name"] == "get_swap" => view_result(&victim_view(escrow_state)),
            Some("view_access_key") => json!({"nonce": 7, "permission": "FullAccess", "block_height": 1, "block_hash": "11111111111111111111111111111111"}),
            _ => Value::Null,
        },
        "send_tx" => json!({
            "final_execution_status": "EXECUTED",
            "status": {"SuccessValue": ""},
            "transaction_outcome": {"id": "x", "outcome": {"logs": [], "status": {"SuccessValue": ""}, "executor_id": "maker.near"}},
            "receipts_outcome": []
        }),
        _ => Value::Null,
    }));
    Rc::new(Chains { mock, near_time, zcash_down: Cell::new(false) })
}

fn victim_view(state: &str) -> Value {
    json!({
        "id": "00".repeat(32), "maker": "maker.near", "token": "usdc.near", "amount": "300000",
        "taker_pk": "09".repeat(32), "taker_account": "09".repeat(32), "a_pub": "00".repeat(32), "b_pub": "00".repeat(32),
        "t0": T1 - 600, "t1": T1, "t2": T2, "reveal_window": 600, "state": state,
        "locked_at": null, "commitment": null, "a": null, "b": null
    })
}

fn swap_json(id: &str, stage: &str, e_t_pub: [u8; 32]) -> Value {
    let opened = stage != "Quoting";
    json!({
        "rfq_id": id, "stage": stage, "zats": 1_000_000, "price": "300000", "premium": 1000,
        "stipend": "30000000000000000000000", "taker_pk": "09".repeat(32), "taker_account": "09".repeat(32),
        "a_pub": "00".repeat(32), "commit_s_t": "00".repeat(32), "b": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())),
        "s_m": "00".repeat(32), "e_m": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())), "e_t_pub": hex::encode(e_t_pub),
        "quote_memo": hex::encode(empty_memo()), "quote_hash": "00".repeat(32),
        "windows": {"t0_secs": 1800, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600},
        "rfq_height": 50, "accept_height": if opened { json!(60) } else { Value::Null }, "negated": false, "joint_fvk": null,
        "salt": if opened { json!("00".repeat(32)) } else { Value::Null },
        "escrow_id": if opened { json!("00".repeat(32)) } else { Value::Null },
        "t0": if opened { json!(T1 - 600) } else { Value::Null }, "t1": if opened { json!(T1) } else { Value::Null },
        "t2": if opened { json!(T2) } else { Value::Null },
        "pending_then": null, "pending": null, "quote_txid": null, "open_tx": "open", "stipend_tx": "stipend", "ready_tx": null,
        "refund_tx": null, "refund_at": null, "sweep_txid": null, "sweep_height": null, "taker_withdraw_at": null, "log": []
    })
}

/// A maker with `stuck` requests in `Quoting` (each one's request memo is in the wallet,
/// as `on_rfq` leaves it), optionally one more swap `victim` in the given stage, and a
/// wallet whose own notes hold `funds` zatoshis (0: it cannot pay a quote fee).
fn maker_with(stuck: usize, victim: Option<&str>, funds: u64, c: &Rc<Chains>, config: MakerConfig) -> (Maker, MemStore) {
    let mut store = MemStore::new();
    store.put(KEY_NEAR, NearKey::from_seed([1u8; 32]).secret_str().as_bytes()).unwrap();
    let rfq_sk = keys::random_scalar();
    put_json(&mut store, KEY_RFQ, &json!({"epoch": 1, "sk": hex::encode(keys::scalar_to_bytes(&rfq_sk))})).unwrap();
    let rfq_pub = msg::dh_public(&rfq_sk);
    let me = Me::generate();
    let reply = Me::generate().address().to_raw_address_bytes();
    let mut notes = vec![];
    let mut seen = BTreeSet::new();
    for i in 0..stuck {
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&(i as u64).to_be_bytes());
        let e_t = keys::random_scalar();
        let eph = msg::dh_public(&e_t);
        let rfq = Rfq { rfq_id: id, zats: 1_000_000, a_pub: [0; 32], pok_a: [0; 64], commit_s_t: [0; 32], taker_pk: [9; 32], reply_addr: reply };
        let ct = msg::seal(b"rfq", &e_t, &rfq_pub, &eph, &rfq_pub, &rfq.to_bytes()).unwrap();
        let memo = Envelope::Rfq { epoch: 1, eph, ct }.encode();
        let txid = format!("{i:064x}");
        seen.insert(format!("{txid}:{i}"));
        notes.push(note("wallet", me.address(), 0, i as u64, 50, &txid, memo));
        put_json(&mut store, &format!("swap/{}", hex::encode(id)), &swap_json(&hex::encode(id), "Quoting", eph)).unwrap();
    }
    if funds > 0 {
        let pos = 1_000_000;
        let txid = "f".repeat(64);
        seen.insert(format!("{txid}:{pos}"));
        notes.push(note("wallet", me.address(), funds, pos, 40, &txid, empty_memo()));
    }
    if let Some(stage) = victim {
        put_json(&mut store, &format!("swap/{VICTIM}"), &swap_json(VICTIM, stage, [0; 32])).unwrap();
    }
    put_json(&mut store, KEY_SEEN, &seen).unwrap();
    store.put("wallet", hex::encode(me.sk_bytes()).as_bytes()).unwrap();
    let mut m = Maker::open(config, c.clone(), &mut store).unwrap();
    m.wallet.height = 100;
    m.wallet.received.extend(notes);
    (m, store)
}

fn victim(m: &Maker) -> MakerStage {
    m.swaps().find(|s| s.rfq_id == VICTIM).unwrap().stage.clone()
}

/// F2-04: a maker with thousands of requests it cannot quote must still send the R3d
/// refund of an Open/Ready swap in time. The work per tick (RPC round trips) must not
/// depend on the number of stuck requests.
#[test]
fn refund_deadline_is_met_with_thousands_of_stuck_requests() {
    let budget = timing().budget();
    let mut per_tick: Vec<Vec<usize>> = vec![];
    for stuck in [0usize, 3000] {
        let c = chains("Ready");
        let (mut m, mut store) = maker_with(stuck, Some("Ready"), 0, &c, cfg());
        let mut rounds = vec![];
        let mut sent_at = None;
        for tick in 0..4 {
            c.mock.calls.borrow_mut().clear();
            let t = Instant::now();
            let now = 1_000_000 + tick;
            m.tick(now, &mut store);
            let dt = t.elapsed();
            let n = c.mock.calls.borrow().len();
            rounds.push(n);
            println!("{stuck} stuck requests, tick {tick}: {n} RPC round trips, {dt:?}; victim {:?}", victim(&m));
            assert!(dt < Duration::from_secs(5), "one tick took {dt:?} with {stuck} stuck requests");
            if c.mock.count("send_tx") > 0 && sent_at.is_none() {
                sent_at = Some(c.near_time.get());
            }
            c.near_time.set(c.near_time.get() + 2);
        }
        let sent_at = sent_at.unwrap_or_else(|| panic!("{stuck} stuck requests: the refund was never sent"));
        assert!(sent_at + budget < T2, "refund sent at NEAR time {sent_at}, too late for t2 {T2} (B_R {budget})");
        assert!(rounds.iter().all(|&n| n <= 8), "{stuck} stuck requests: RPC round trips per tick {rounds:?}");
        per_tick.push(rounds);
    }
    assert_eq!(per_tick[0], per_tick[1], "the work per tick depends on the number of stuck requests");
}

/// F2-04: with ZEC for fees, at most `max_quotes_per_tick` quotes are built per tick. Each
/// build attempt syncs the wallet (`getblockcount`), so the count of those calls bounds
/// the attempts.
#[test]
fn quote_builds_per_tick_are_capped() {
    for cap in [1usize, 3] {
        let c = chains("Ready");
        let mut j = cfg_json();
        j["max_quotes_per_tick"] = json!(cap);
        let (mut m, mut store) = maker_with(50, None, 10_000_000, &c, serde_json::from_value(j).unwrap());
        m.tick(1_000_000, &mut store);
        let syncs = c.mock.count("getblockcount");
        // one sync per tick, plus at most two per build attempt (build_pay and build)
        assert!(syncs <= 1 + 2 * cap, "cap {cap}: {syncs} Zcash round trips in one tick for 50 requests");
        assert!(syncs > 1, "cap {cap}: no quote was attempted at all");
    }
}

/// F2-04: requests beyond `max_pending_quotes` are refused (and marked handled), so they
/// add no work.
#[test]
fn requests_beyond_the_pending_cap_are_refused() {
    let c = chains("Ready");
    let mut j = cfg_json();
    j["max_pending_quotes"] = json!(4);
    let (mut m, mut store) = maker_with(3, None, 0, &c, serde_json::from_value(j).unwrap());
    // Five valid requests arrive in one block.
    let rfq_pub: [u8; 32] = hex::decode(&m.directory_entry().rfq_pub).unwrap().try_into().unwrap();
    let me = m.wallet.received[0].note.recipient();
    for i in 0..5u64 {
        let mut id = [0u8; 16];
        id[0] = 0xa0;
        id[8..].copy_from_slice(&i.to_be_bytes());
        let a = keys::random_scalar();
        let rfq = Rfq {
            rfq_id: id,
            zats: 1_000_000,
            a_pub: (keys::g() * a).to_bytes(),
            pok_a: keys::pok_prove(&a, &id, b"taker"),
            commit_s_t: [1; 32],
            taker_pk: NearKey::generate().public_bytes(),
            reply_addr: Me::generate().address().to_raw_address_bytes(),
        };
        let e_t = keys::random_scalar();
        let eph = msg::dh_public(&e_t);
        let ct = msg::seal(b"rfq", &e_t, &rfq_pub, &eph, &rfq_pub, &rfq.to_bytes()).unwrap();
        let memo = Envelope::Rfq { epoch: 1, eph, ct }.encode();
        m.wallet.received.push(note("wallet", me, 0, 5000 + i, 99, &format!("{:064x}", 5000 + i), memo));
    }
    let t = m.tick(1_000_000, &mut store);
    let pending = m.swaps().filter(|s| matches!(s.stage, MakerStage::Quoting | MakerStage::Quoted)).count();
    assert_eq!(pending, 4, "3 stuck + 1 new request fit under the cap of 4; the other 4 are refused");
    let refused = t.events.iter().filter(|e| matches!(e, Event::Log { line, .. } if line.contains("RFQ rejected") && line.contains("max_pending_quotes"))).count();
    assert_eq!(refused, 4);
    let seen: BTreeSet<String> = get_json(&store, KEY_SEEN).unwrap().unwrap();
    assert!((0..5u64).all(|i| seen.contains(&format!("{:064x}:{}", 5000 + i, 5000 + i))), "refused requests must be marked handled");
}

/// F2-04: requests that stay in `Quoting` expire after `quote_ttl_secs` and their secrets
/// are erased.
#[test]
fn stuck_requests_expire() {
    let c = chains("Ready");
    let (mut m, mut store) = maker_with(10, None, 0, &c, cfg());
    let ttl = 3600; // the default quote_ttl_secs
    m.tick(1_000_000, &mut store);
    m.tick(1_000_000 + ttl, &mut store);
    assert_eq!(m.swaps().filter(|s| s.stage == MakerStage::Quoting).count(), 10, "not expired at exactly the TTL");
    m.tick(1_000_000 + ttl + 1, &mut store);
    for s in m.swaps() {
        assert_eq!(s.stage, MakerStage::Aborted("request expired".into()));
        assert!(s.b.is_empty() && s.s_m.is_empty() && s.e_m.is_empty(), "secrets of an expired request are erased");
    }
}

/// F2-04: an Accept memo for a request the maker does not know is handled once, not
/// retried with a warning on every tick.
#[test]
fn accept_for_unknown_request_is_handled_once() {
    let c = chains("Ready");
    let (mut m, mut store) = maker_with(0, None, 0, &c, cfg());
    let me = Me::generate().address();
    let memo = Envelope::Accept { rfq_id: [0x77; 16], ct: vec![0u8; 80] }.encode();
    m.wallet.received.push(note("wallet", me, 0, 9, 99, &"9".repeat(64), memo));
    let t1 = m.tick(1_000_000, &mut store);
    assert!(!t1.events.iter().any(|e| matches!(e, Event::Warning { .. })), "{:?}", t1.events);
    let seen: BTreeSet<String> = get_json(&store, KEY_SEEN).unwrap().unwrap();
    assert!(seen.contains(&format!("{}:9", "9".repeat(64))));
    let t2 = m.tick(1_000_002, &mut store);
    assert!(t2.events.is_empty(), "the Accept was processed again: {:?}", t2.events);
}

/// H2: when the Zcash light-wallet server fails, the maker still decides and sends the
/// R3d refund of a swap past t1.
#[test]
fn zcash_outage_does_not_stop_refunds() {
    let c = chains("Ready");
    let (mut m, mut store) = maker_with(0, Some("Ready"), 0, &c, cfg());
    c.zcash_down.set(true);
    let t = m.tick(1_000_000, &mut store);
    assert!(t.events.iter().any(|e| matches!(e, Event::Warning { message, .. } if message.contains("Zcash sync"))), "{:?}", t.events);
    assert_eq!(victim(&m), MakerStage::RefundSent, "the refund decision waits for the Zcash server");
    m.tick(1_000_001, &mut store);
    assert_eq!(c.mock.count("send_tx"), 1, "the refund was not sent while the Zcash server is down");
}

/// H2: with a stale wallet the maker does not take decisions that need fresh Zcash data:
/// no early refund (R3e) of an Open swap, because the funding may be there unseen.
#[test]
fn zcash_outage_blocks_no_early_refund_decision() {
    let c = chains("Open");
    // NEAR time inside the R3e window: past t0 - fund_secs, before t0 - D.
    c.near_time.set(T1 - 600 - 100);
    let (mut m, mut store) = maker_with(0, Some("Opened"), 0, &c, cfg());
    c.zcash_down.set(true);
    m.tick(1_000_000, &mut store);
    m.tick(1_000_001, &mut store);
    assert_eq!(victim(&m), MakerStage::Opened);
    assert_eq!(c.mock.count("send_tx"), 0);
    // With the Zcash server back and still no note, R3e applies.
    c.zcash_down.set(false);
    m.tick(1_000_002, &mut store);
    assert_eq!(victim(&m), MakerStage::RefundSent);
}

/// NV2-02: a maker whose premium is worth less than the stipend plus its own costs per
/// accepted request does not start, when prices are configured; on mainnet the prices are
/// required.
#[test]
fn premium_below_cost_is_refused() {
    let c = chains("Ready");
    let open = |j: Value| -> Result<Maker> {
        let mut store = MemStore::new();
        store.put(KEY_NEAR, NearKey::from_seed([1u8; 32]).secret_str().as_bytes()).unwrap();
        Maker::open(serde_json::from_value(j)?, c.clone(), &mut store)
    };
    // 1 ZEC = 10 NEAR. The stipend is 0.03 NEAR; costs 0.015 NEAR: 0.045 NEAR per request
    // needs at least 450 000 zats of premium.
    let price = (10u128 * 10u128.pow(24)).to_string();
    let mut j = cfg_json();
    j["premium_check"] = json!({"yocto_near_per_zec": price, "costs_per_request_yocto": "15000000000000000000000"});
    j["premium_zats"] = json!(449_999);
    let e = open(j.clone()).err().expect("a premium below cost must be refused").to_string();
    assert!(e.contains("premium"), "{e}");
    j["premium_zats"] = json!(450_000);
    open(j.clone()).unwrap();
    // Mainnet without prices: refused.
    let mut main = cfg_json();
    main["chains"]["network"] = json!("main");
    let e = open(main).err().expect("mainnet needs premium_check").to_string();
    assert!(e.contains("premium_check"), "{e}");
}

/// H5: a test-only `misbehave` setting is refused by a build without the `misbehave`
/// feature (the release binary).
#[cfg(not(feature = "misbehave"))]
#[test]
fn misbehave_is_refused_without_the_feature() {
    let mut j = cfg_json();
    j["misbehave"] = json!("no-stipend");
    let e = serde_json::from_value::<MakerConfig>(j).err().expect("misbehave must be refused").to_string();
    assert!(e.contains("misbehave"), "{e}");
    let mut t = serde_json::to_value(taker_cfg("utest")).unwrap();
    t["misbehave"] = json!("no-reveal");
    assert!(serde_json::from_value::<zecret_core::config::TakerConfig>(t.clone()).is_err());
    t["misbehave"] = Value::Null;
    serde_json::from_value::<zecret_core::config::TakerConfig>(t).unwrap();
}
