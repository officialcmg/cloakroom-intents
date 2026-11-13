// The maker's refusal reply (docs/TESTNET.md, "Problems met", item 1) and the false
// "0 spendable zats" warning (item 3).
//
// A request the maker does not quote gets a small signed reply, so the taker stops at once
// instead of waiting for its quote timeout. A reply costs the maker a Zcash fee and a
// proof, so these tests also pin the limits: at most `max_refusals_per_hour` replies in
// any hour, one per request id, none when the setting is 0, and no build while a request
// waits for its quote or the wallet cannot pay a fee.
//
// Everything runs through `Maker::tick` against a scripted JSON-RPC transport.

#[path = "poc2_common.rs"]
mod common;

use std::rc::Rc;

use group::GroupEncoding;
use serde_json::{json, Value};
use zecret_core::{
    config::MakerConfig,
    engine::Event,
    keys,
    maker::{Maker, MakerRefusal, MakerStage, KEY_NEAR, KEY_RFQ},
    msg::{self, Envelope, Refusal, Rfq},
    near::NearKey,
    store::{get_json, put_json, MemStore, Store},
    wallet::empty_memo,
    Me,
};

use common::*;

const NOW: u64 = 1_000_000;

fn cfg(extra: Value) -> MakerConfig {
    let mut j = json!({
        "chains": {"zebra_rpc": "http://zebra.invalid", "nu7_height": 2, "near_rpc": "http://near.invalid",
                   "escrow": "escrow.near", "token": "usdc.near", "intents": "intents.near"},
        "timing": {"inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2, "margin_secs": 0, "fund_secs": 360, "k_z": 3},
        "near_account": "maker.near", "price_per_zec": "30000000", "min_zats": 100000, "max_zats": 10000000,
        "premium_zats": 1000, "stipend": "30000000000000000000000",
        "windows": {"t0_secs": 600, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600}
    });
    for (k, v) in extra.as_object().unwrap() {
        j[k] = v.clone();
    }
    serde_json::from_value(j).unwrap()
}

fn mock() -> Rc<Mock> {
    Rc::new(Mock::new(|m, _| match m {
        "getblockcount" => json!(100),
        _ => Value::Null,
    }))
}

/// A maker whose wallet holds `funds` zatoshis of its own.
fn maker(config: MakerConfig, funds: u64, mk: &Rc<Mock>) -> (Maker, MemStore) {
    let mut store = MemStore::new();
    store.put(KEY_NEAR, NearKey::from_seed([1u8; 32]).secret_str().as_bytes()).unwrap();
    let rfq_sk = keys::random_scalar();
    put_json(&mut store, KEY_RFQ, &json!({"epoch": 1, "sk": hex::encode(keys::scalar_to_bytes(&rfq_sk))})).unwrap();
    let me = Me::generate();
    store.put("wallet", hex::encode(me.sk_bytes()).as_bytes()).unwrap();
    let mut m = Maker::open(config, mk.clone(), &mut store).unwrap();
    m.wallet.height = 100;
    if funds > 0 {
        m.wallet.received.push(note("wallet", me.address(), funds, 999_999, 40, &"f".repeat(64), empty_memo()));
    }
    (m, store)
}

/// A request memo for `zats`, as a taker sends it. Returns its id and the taker's
/// ephemeral secret.
fn request(m: &mut Maker, zats: u64, n: u64) -> ([u8; 16], pasta_curves::pallas::Scalar) {
    let rfq_pub: [u8; 32] = hex::decode(&m.directory_entry().rfq_pub).unwrap().try_into().unwrap();
    let mut id = [0u8; 16];
    id[0] = 0xb0;
    id[8..].copy_from_slice(&n.to_be_bytes());
    let a = keys::random_scalar();
    let rfq = Rfq {
        rfq_id: id,
        zats,
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
    let me = Me::generate().address(); // the recipient does not matter to the scan
    m.wallet.received.push(note("wallet", me, 0, 7000 + n, 99, &format!("{:064x}", 7000 + n), memo));
    (id, e_t)
}

fn queued(store: &MemStore) -> Vec<MakerRefusal> {
    store.keys("refusal/").unwrap().iter().map(|k| get_json(store, k).unwrap().unwrap()).collect()
}

/// An amount above the limit gets a signed refusal that the taker can open and verify,
/// with the code and the maker's limits; no swap is created.
#[test]
fn over_limit_request_gets_a_signed_refusal() {
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({})), 0, &mk);
    let (id, e_t) = request(&mut m, 50_000_000, 1);
    let t = m.tick(NOW, &mut store);
    assert!(t.events.iter().any(|e| matches!(e, Event::Log { line, .. } if line.contains("RFQ rejected: amount 50000000") && line.contains("refusal reply (over_limit) is queued"))), "{:?}", t.events);
    assert_eq!(m.swaps().count(), 0);
    let q = queued(&store);
    assert_eq!(q.len(), 1);
    assert_eq!((q[0].rfq_id.as_str(), q[0].code.as_str()), (hex::encode(id).as_str(), "over_limit"));
    // What the taker sees.
    let memo: [u8; 512] = hex::decode(&q[0].memo).unwrap().try_into().unwrap();
    let Envelope::Refuse { rfq_id, ct } = Envelope::decode(&memo).unwrap().unwrap() else { panic!("not a refusal") };
    assert_eq!(rfq_id, id);
    let e = m.directory_entry();
    let maker_pub: [u8; 32] = hex::decode(&e.rfq_pub).unwrap().try_into().unwrap();
    let eph = msg::dh_public(&e_t);
    let r = Refusal::from_bytes(&msg::open(b"refuse", &e_t, &maker_pub, &eph, &maker_pub, &ct).unwrap()).unwrap();
    assert_eq!((r.min_zats, r.max_zats), (100_000, 10_000_000));
    let pk = zecret_core::near::tx::parse_public_key(&e.near_pk).unwrap();
    assert!(zecret_core::near::tx::verify(&pk, &r.sig_digest(&id), &r.sig));
    // Below the minimum: the other code.
    request(&mut m, 10, 2);
    m.tick(NOW + 1, &mut store);
    assert!(queued(&store).iter().any(|r| r.code == "under_minimum"));
}

/// At most `max_refusals_per_hour` replies in any hour; the window slides.
#[test]
fn refusals_are_capped_per_hour() {
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({"max_refusals_per_hour": 2})), 0, &mk);
    for n in 0..5 {
        request(&mut m, 50_000_000, n);
    }
    let t = m.tick(NOW, &mut store);
    assert_eq!(queued(&store).len(), 2, "two replies, three dropped");
    let capped = t.events.iter().filter(|e| matches!(e, Event::Log { line, .. } if line.contains("max_refusals_per_hour"))).count();
    assert_eq!(capped, 3);
    // An hour later the window has room again.
    request(&mut m, 50_000_000, 10);
    m.tick(NOW + 3601, &mut store);
    assert_eq!(queued(&store).len(), 3);
}

/// 0 turns the replies off; a refused request is still marked handled.
#[test]
fn refusals_can_be_turned_off() {
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({"max_refusals_per_hour": 0})), 0, &mk);
    request(&mut m, 50_000_000, 1);
    let t = m.tick(NOW, &mut store);
    assert!(queued(&store).is_empty());
    assert!(t.events.iter().any(|e| matches!(e, Event::Log { line, .. } if line.contains("refusal replies are off"))));
}

/// A request that arrives while `max_pending_quotes` requests wait gets the `busy` code.
#[test]
fn busy_maker_says_so() {
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({"max_pending_quotes": 1})), 0, &mk);
    request(&mut m, 1_000_000, 1);
    request(&mut m, 1_000_000, 2);
    m.tick(NOW, &mut store);
    let q = queued(&store);
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].code, "busy");
}

/// No reply is built (no proof, no RPC for it) while the wallet cannot pay a fee, and
/// none while a request waits for its quote: quotes come first.
#[test]
fn refusal_is_built_only_from_the_leftover_quote_budget() {
    // No funds: only the tick's own sync reaches the Zcash node.
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({})), 0, &mk);
    request(&mut m, 50_000_000, 1);
    m.tick(NOW, &mut store);
    m.tick(NOW + 1, &mut store);
    assert_eq!(mk.count("getblockcount"), 2, "one sync per tick, no build");
    assert!(queued(&store)[0].pending.is_none());
    // Funds, but a request waits for its quote in the same tick: the quote takes the
    // budget (its build is tried), the refusal waits.
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({})), 1_000_000, &mk);
    request(&mut m, 50_000_000, 1);
    request(&mut m, 1_000_000, 2);
    let t = m.tick(NOW, &mut store);
    assert!(!t.events.iter().any(|e| matches!(e, Event::Warning { message, .. } if message.contains("refusal reply not built"))), "{:?}", t.events);
    // With funds and nothing waiting, the reply build is tried (the test wallet has no
    // note tree, so it fails with a warning, which uses the tick's budget).
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({})), 1_000_000, &mk);
    request(&mut m, 50_000_000, 1);
    let t = m.tick(NOW, &mut store);
    assert!(t.events.iter().any(|e| matches!(e, Event::Warning { message, .. } if message.contains("refusal reply not built"))), "{:?}", t.events);
}

/// A reply not built within `quote_ttl_secs` is dropped.
#[test]
fn unsent_refusal_expires() {
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({"quote_ttl_secs": 100, "quote_send_secs": 100})), 0, &mk);
    request(&mut m, 50_000_000, 1);
    m.tick(NOW, &mut store);
    m.tick(NOW + 101, &mut store);
    assert!(queued(&store)[0].done);
}

/// Testnet item 3: a quote that is built but not yet broadcast keeps its swap in
/// `Quoting` for one more tick. That is not a lack of funds, and the maker must not warn
/// that the wallet has 0 spendable zats.
#[test]
fn no_false_low_funds_warning_while_a_quote_is_in_flight() {
    let mk = mock();
    let (mut m, mut store) = maker(cfg(json!({})), 0, &mk);
    let rec = json!({
        "rfq_id": "c0".repeat(16), "stage": "Quoting", "zats": 1_000_000, "price": "300000", "premium": 1000,
        "stipend": "30000000000000000000000", "taker_pk": "09".repeat(32), "taker_account": "09".repeat(32),
        "a_pub": "00".repeat(32), "commit_s_t": "00".repeat(32), "b": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())),
        "s_m": "00".repeat(32), "e_m": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())), "e_t_pub": "00".repeat(32),
        "quote_memo": hex::encode(empty_memo()), "quote_hash": "00".repeat(32),
        "windows": {"t0_secs": 600, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600},
        "rfq_height": 50, "accept_height": null, "negated": false, "joint_fvk": null, "salt": null, "escrow_id": null,
        "t0": null, "t1": null, "t2": null, "pending_then": "Quoted",
        "pending": {"what": "quote", "txid": "ab".repeat(32), "raw_hex": "00", "expiry_height": 140, "broadcast": false},
        "quote_txid": null, "open_tx": null, "stipend_tx": null, "ready_tx": null, "refund_tx": null, "refund_at": null,
        "sweep_txid": null, "sweep_height": null, "taker_withdraw_at": null, "log": [], "created_at": NOW
    });
    put_json(&mut store, &format!("swap/{}", "c0".repeat(16)), &rec).unwrap();
    let mut m2 = Maker::open(m.cfg.clone(), mk.clone(), &mut store).unwrap();
    m2.wallet.height = 100;
    let t = m2.tick(NOW, &mut store);
    assert!(!t.events.iter().any(|e| matches!(e, Event::Warning { message, .. } if message.contains("less than a quote fee"))), "{:?}", t.events);
    let _ = &mut m;
}

/// Incident 2026-11-13: a quote that nodes refuse is built again, but only until
/// `quote_send_secs` after the request. Then the request ends and the built quote is not
/// broadcast.
#[test]
fn a_quote_not_sent_in_time_is_dropped() {
    let mk = mock();
    let (_m, mut store) = maker(cfg(json!({})), 1_000_000, &mk);
    let rec = |created: u64| json!({
        "rfq_id": "c1".repeat(16), "stage": "Quoting", "zats": 1_000_000, "price": "300000", "premium": 1000,
        "stipend": "30000000000000000000000", "taker_pk": "09".repeat(32), "taker_account": "09".repeat(32),
        "a_pub": "00".repeat(32), "commit_s_t": "00".repeat(32), "b": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())),
        "s_m": "00".repeat(32), "e_m": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())), "e_t_pub": "00".repeat(32),
        "quote_memo": hex::encode(empty_memo()), "quote_hash": "00".repeat(32),
        "windows": {"t0_secs": 600, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600},
        "rfq_height": 50, "accept_height": null, "negated": false, "joint_fvk": null, "salt": null, "escrow_id": null,
        "t0": null, "t1": null, "t2": null, "pending_then": "Quoted",
        "pending": {"what": "quote", "txid": "ab".repeat(32), "raw_hex": "00", "expiry_height": 140, "broadcast": false},
        "quote_txid": null, "open_tx": null, "stipend_tx": null, "ready_tx": null, "refund_tx": null, "refund_at": null,
        "sweep_txid": null, "sweep_height": null, "taker_withdraw_at": null, "log": [], "created_at": created
    });
    put_json(&mut store, &format!("swap/{}", "c1".repeat(16)), &rec(NOW - 301)).unwrap();
    let cfg2 = cfg(json!({}));
    let mut m2 = Maker::open(cfg2, mk.clone(), &mut store).unwrap();
    m2.wallet.height = 100;
    let sends = mk.count("sendrawtransaction");
    let t = m2.tick(NOW, &mut store);
    let s = m2.swaps().next().unwrap();
    assert_eq!(s.stage, MakerStage::Aborted("quote not sent in time".into()), "{:?}", t.events);
    assert!(s.pending.is_none() && s.b.is_empty());
    assert_eq!(mk.count("sendrawtransaction"), sends, "the late quote is not broadcast");
    assert!(t.events.iter().any(|e| matches!(e, Event::Log { line, .. } if line.contains("no quote sent within 300s"))), "{:?}", t.events);
}

/// The directory entry publishes the limits and signs them.
#[test]
fn directory_entry_signs_the_limits() {
    let mk = mock();
    let (m, _) = maker(cfg(json!({})), 0, &mk);
    let e = m.directory_entry();
    let l = e.limits.expect("limits published");
    assert_eq!((l.min_zats, l.max_zats, l.premium_zats), (100_000, 10_000_000, 1000));
    let pk = zecret_core::near::tx::parse_public_key(&e.near_pk).unwrap();
    let sig: [u8; 64] = hex::decode(&e.sig).unwrap().try_into().unwrap();
    assert!(zecret_core::near::tx::verify(&pk, &e.digest().unwrap(), &sig));
    let mut changed = e.clone();
    changed.limits = Some(zecret_core::config::MakerLimits { max_zats: 1, ..l });
    assert!(!zecret_core::near::tx::verify(&pk, &changed.digest().unwrap(), &sig));
}
