// Security review 2: proof of concept against the maker state machine
// (crates/zecret-core/src/maker.rs), kept as a regression test for F2-04. It failed on
// the reviewed code and passes with the fix (docs/security/fixes-2-core.md).
// Copied from docs/security/poc2/maker_pocs.rs; only the `common` import differs.
//
//   cd crates && cargo test -p zecret-core --test poc2_maker -- --nocapture

use std::{collections::BTreeSet, rc::Rc, time::Instant};

use serde_json::{json, Value};
use zecret_core::{
    config::MakerConfig,
    keys,
    maker::{Maker, MakerStage, KEY_NEAR, KEY_RFQ, KEY_SEEN},
    msg::{self, Envelope, Rfq},
    near::NearKey,
    store::{put_json, MemStore, Store},
    Me,
};

#[path = "poc2_common.rs"]
mod common;
use common::*;

fn maker_cfg() -> MakerConfig {
    serde_json::from_value(json!({
        "chains": {"zebra_rpc": "http://zebra.invalid", "nu7_height": 2, "near_rpc": "http://near.invalid",
                   "escrow": "escrow.near", "token": "usdc.near", "intents": "intents.near"},
        "timing": {"inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2, "margin_secs": 0, "fund_secs": 900, "k_z": 3},
        "near_account": "maker.near", "price_per_zec": "30000000", "min_zats": 100000, "max_zats": 10000000,
        "premium_zats": 1000, "stipend": "30000000000000000000000",
        "windows": {"t0_secs": 1800, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600}
    }))
    .unwrap()
}

/// Builds a maker whose wallet holds `n` request memos that anyone could have sent (no
/// premium is paid before the quote), each already turned into a `Quoting` swap, as
/// `on_rfq` does. The maker's wallet has no ZEC left for quote fees (the requests drained
/// it, or its notes are reserved by quotes not mined yet). Returns the maker, the
/// transport and the store.
fn spammed_maker(n: usize) -> (Maker, Rc<Mock>, MemStore) {
    let mut store = MemStore::new();
    store.put(KEY_NEAR, NearKey::from_seed([1u8; 32]).secret_str().as_bytes()).unwrap();
    let rfq_sk = keys::random_scalar();
    put_json(&mut store, KEY_RFQ, &json!({"epoch": 1, "sk": hex::encode(keys::scalar_to_bytes(&rfq_sk))})).unwrap();
    let rfq_pub = msg::dh_public(&rfq_sk);
    let me = Me::generate();
    let reply = Me::generate().address().to_raw_address_bytes();
    let mut notes = vec![];
    let mut seen = BTreeSet::new();
    for i in 0..n {
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
        let swap = json!({
            "rfq_id": hex::encode(id), "stage": "Quoting", "zats": 1_000_000, "price": "300000", "premium": 1000,
            "stipend": "30000000000000000000000", "taker_pk": "09".repeat(32), "taker_account": "09".repeat(32),
            "a_pub": "00".repeat(32), "commit_s_t": "00".repeat(32), "b": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())),
            "s_m": "00".repeat(32), "e_m": hex::encode(keys::scalar_to_bytes(&keys::random_scalar())), "e_t_pub": hex::encode(eph),
            "quote_memo": hex::encode(zecret_core::wallet::empty_memo()), "quote_hash": "00".repeat(32),
            "windows": {"t0_secs": 1800, "t1_secs": 600, "t2_secs": 900, "reveal_secs": 600},
            "rfq_height": 50, "accept_height": null, "negated": false, "joint_fvk": null, "salt": null, "escrow_id": null,
            "t0": null, "t1": null, "t2": null, "pending_then": null, "pending": null, "quote_txid": null, "open_tx": null,
            "stipend_tx": null, "ready_tx": null, "refund_tx": null, "refund_at": null, "sweep_txid": null,
            "sweep_height": null, "taker_withdraw_at": null, "log": []
        });
        put_json(&mut store, &format!("swap/{}", hex::encode(id)), &swap).unwrap();
    }
    put_json(&mut store, KEY_SEEN, &seen).unwrap();
    let mock = Rc::new(Mock::new(|m, _| if m == "getblockcount" { json!(100) } else { Value::Null }));
    store.put("wallet", hex::encode(me.sk_bytes()).as_bytes()).unwrap();
    let mut m = Maker::open(maker_cfg(), mock.clone(), &mut store).unwrap();
    m.wallet.height = 100;
    m.wallet.received.extend(notes);
    (m, mock, store)
}

/// F2-04. Requests cost the maker work before any premium is paid, and a request that
/// cannot be quoted stays `Quoting` for ever. Every tick then repeats one Zcash RPC round
/// trip (and an O(n) note scan) per stuck request before it reaches the swaps whose
/// deadlines matter, such as an R3d refund that must land in [t1, t2 - B_R).
#[test]
fn poc_f2_04_unpaid_requests_do_not_grow_the_makers_tick() {
    let mut results = vec![];
    for n in [100usize, 400, 1600] {
        let (mut m, mock, mut store) = spammed_maker(n);
        let t = Instant::now();
        m.tick(1_000_000, &mut store);
        let dt = t.elapsed();
        let rpc = mock.count("getblockcount");
        // Thirty days later the requests are still there and still cost the same.
        mock.calls.borrow_mut().clear();
        m.tick(1_000_000 + 30 * 86_400, &mut store);
        let stuck = m.swaps().filter(|s| s.stage == MakerStage::Quoting).count();
        println!("n = {n}: {rpc} Zcash RPC round trips and {dt:?} of CPU in one tick; {stuck} still Quoting 30 days later ({} round trips in that tick)", mock.count("getblockcount"));
        results.push((n, rpc, stuck));
    }
    let (n, rpc, stuck) = results[0];
    assert!(
        rpc <= 10 && stuck == 0,
        "F2-04: {n} unpaid requests cost {rpc} Zcash RPC round trips in every maker tick and never expire ({stuck} still Quoting after 30 days)"
    );
}
