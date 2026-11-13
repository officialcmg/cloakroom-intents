//! The shipped testnet configurations (`config/testnet-*.json`, and the website's
//! `web/public/config/testnet.json`) must keep every bound of the paper and agree with one
//! another.
//!
//! Zcash testnet blocks (NU7) were measured on 2026-11-13 over 3000 intervals before block
//! 4,615,373 through the light-wallet server: mean 20.4 s, median 13 s, 99.9th percentile
//! 211 s, maximum 451 s; three consecutive intervals (what k_Z = 3 confirmations take):
//! 99.9th percentile 467 s, maximum 538 s. ∆fund (`fund_secs`) is k_Z confirmations at a
//! high percentile plus D, so it must be at least 538 + D.

use zecret_core::config::{MakerConfig, TakerConfig, Timing};

const MAKER: &str = include_str!("../config/testnet-maker.json");
const TAKER: &str = include_str!("../config/testnet-taker.json");
const WEB: &str = include_str!("../../../web/public/config/testnet.json");

/// The longest time three consecutive testnet blocks took in the measurement above.
const MEASURED_MAX_3_BLOCKS: u64 = 538;
/// The longest single testnet block interval at the 99.9th percentile.
const MEASURED_P999_1_BLOCK: u64 = 211;

fn maker() -> MakerConfig {
    serde_json::from_str(MAKER).unwrap()
}

fn taker() -> TakerConfig {
    let mut v: serde_json::Value = serde_json::from_str(TAKER).unwrap();
    // The template has a placeholder for the maker's directory entry.
    v["maker"] = serde_json::json!({"ua": "u", "epoch": 1, "rfq_pub": "00", "near_account": "m", "near_pk": "ed25519:1", "sig": "00"});
    serde_json::from_value(v).unwrap()
}

fn same(a: &Timing, b: &Timing) -> bool {
    (a.inclusion_secs, a.finality_secs, a.reorg_budget, a.margin_secs, a.fund_secs, a.k_z) == (b.inclusion_secs, b.finality_secs, b.reorg_budget, b.margin_secs, b.fund_secs, b.k_z)
}

#[test]
fn testnet_maker_keeps_the_paper_bounds() {
    let m = maker();
    m.timing.validate().unwrap();
    // W > F + B_R, t0 − open > F + ∆fund, t1 − t0 > F + B_R, t2 − t1 > D + F + B_R.
    m.timing.check_windows(&m.windows).unwrap();
    m.check_limits().unwrap();
    m.check_premium().unwrap();
    assert!(m.timing.fund_secs >= MEASURED_MAX_3_BLOCKS + m.timing.inclusion_secs, "fund_secs below k_Z blocks at the measured worst case plus D");
    assert_eq!(m.timing.k_z, 3);
}

#[test]
fn testnet_taker_accepts_what_the_maker_quotes() {
    let (m, t) = (maker(), taker());
    t.timing.validate().unwrap();
    assert!(same(&m.timing, &t.timing), "maker and taker must use the same bounds");
    // The taker's R1q checks of the windows (taker.rs `reject_quote`), cover delays off.
    let tt = &t.timing;
    let w = &m.windows;
    assert!(w.reveal_secs as u64 > tt.finality_secs + tt.budget());
    assert!(w.t0_secs as u64 > tt.finality_secs + tt.fund_secs + 3 * t.cover.max_secs);
    assert!(w.t1_secs as u64 > tt.finality_secs + tt.budget());
    assert!((w.t0_secs + w.t1_secs + w.t2_secs) as u64 <= t.max_horizon_secs);
    // The quote must reach the taker in time: two blocks (request, quote) at the 99.9th
    // percentile, with room for two proofs and the scans.
    assert!(t.quote_timeout_secs >= 2 * MEASURED_P999_1_BLOCK + 120);
    // A request must not expire at the maker while the taker still waits (F2-04 TTL).
    assert!(m.quote_ttl_secs > 2 * t.quote_timeout_secs + MEASURED_P999_1_BLOCK);
    // A quote not sent by quote_send_secs after the maker read the request is dropped: a
    // quote sent then and mined at the 99.9th percentile still reaches the taker in time.
    assert!(m.quote_send_secs + MEASURED_P999_1_BLOCK < t.quote_timeout_secs);
    assert!(m.premium_zats <= t.max_premium);
}

#[test]
fn website_testnet_config_matches_the_taker_template() {
    let web: serde_json::Value = serde_json::from_str(WEB).unwrap();
    let wt: Timing = serde_json::from_value(web["timing"].clone()).unwrap();
    let t = taker();
    assert!(same(&wt, &t.timing), "web/public/config/testnet.json timing differs from testnet-taker.json");
    assert_eq!(web["quoteTimeoutSecs"].as_u64(), Some(t.quote_timeout_secs));
}

#[test]
fn a_window_that_breaks_a_bound_is_refused() {
    let m = maker();
    let b = m.timing.budget();
    let f = m.timing.finality_secs;
    for w in [
        zecret_core::config::Windows { reveal_secs: (f + b) as u32, ..m.windows },
        zecret_core::config::Windows { t0_secs: (f + m.timing.fund_secs) as u32, ..m.windows },
        zecret_core::config::Windows { t1_secs: (f + b) as u32, ..m.windows },
        zecret_core::config::Windows { t2_secs: (m.timing.inclusion_secs + f + b) as u32, ..m.windows },
    ] {
        assert!(m.timing.check_windows(&w).is_err(), "{w:?}");
    }
    let mut bad = m.clone();
    bad.min_zats = bad.max_zats + 1;
    assert!(bad.check_limits().is_err());
    assert!(Timing { k_z: 0, ..m.timing.clone() }.validate().is_err());
}
