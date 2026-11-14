//! The mainnet configuration templates (`config/mainnet-*.json`, and the website's
//! `web/public/config/mainnet.json`) must keep every bound of the paper, agree with one
//! another, and name the real mainnet contracts. Real funds move on mainnet, so the maker
//! must also refuse to start without the premium check.
//!
//! Zcash mainnet blocks after NU7 (ZIP 218: target spacing 25 s; NU7 activation height
//! 3,543,405 on mainnet) were measured on 2026-11-14 with
//! `scripts/zcash-block-intervals.py https://zcash-mainnet.chainsafe.dev 3000`, the same
//! method as for testnet (`testnet_config.rs`): 3000 intervals, blocks 3,567,916 to
//! 3,570,916, 74,697 s. One interval: mean 24.9 s, median 17 s, 99th percentile 103 s,
//! 99.9th 153 s, maximum 182 s. Two consecutive intervals: 99.9th percentile 198 s,
//! maximum 222 s. Three (what k_Z = 3 confirmations take): median 67 s, 99.9th percentile
//! 250 s, maximum 299 s. ∆fund (`fund_secs`) is k_Z confirmations at a high percentile
//! plus D, so it must be at least 299 + D. The templates keep 600 s, about twice that, the
//! same as testnet.
//!
//! The templates hold placeholders that the owner fills at deploy time (the escrow and
//! maker accounts, the escrow's code hash, the maker's price). The tests check the
//! placeholders too, so that a template cannot start a maker by accident
//! (`deploy/maker/railway/start.sh` refuses them).

use zecret_core::config::{MakerConfig, TakerConfig, Timing};

const MAKER: &str = include_str!("../config/mainnet-maker.json");
const TAKER: &str = include_str!("../config/mainnet-taker.json");
const WEB: &str = include_str!("../../../web/public/config/mainnet.json");
const TESTNET_MAKER: &str = include_str!("../config/testnet-maker.json");

/// Measured on Zcash mainnet (above): the longest three consecutive block intervals.
const MEASURED_MAX_3_BLOCKS: u64 = 299;
/// The 99.9th percentile of one block interval.
const MEASURED_P999_1_BLOCK: u64 = 153;
/// The 99.9th percentile of two consecutive intervals (request and quote).
const MEASURED_P999_2_BLOCKS: u64 = 198;

/// Circle's USDC on NEAR mainnet. Read on 2026-11-14 at final block 224,453,879:
/// `ft_metadata` symbol "USDC", 6 decimals; code hash DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q.
const USDC_MAINNET: &str = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const VERIFIER_MAINNET: &str = "intents.near";
const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

fn maker() -> MakerConfig {
    serde_json::from_str(MAKER).unwrap()
}

fn taker() -> TakerConfig {
    let mut v: serde_json::Value = serde_json::from_str(TAKER).unwrap();
    v["maker"] = serde_json::json!({"ua": "u", "epoch": 1, "rfq_pub": "00", "near_account": "m", "near_pk": "ed25519:1", "sig": "00"});
    serde_json::from_value(v).unwrap()
}

fn same(a: &Timing, b: &Timing) -> bool {
    (a.inclusion_secs, a.finality_secs, a.reorg_budget, a.margin_secs, a.fund_secs, a.k_z) == (b.inclusion_secs, b.finality_secs, b.reorg_budget, b.margin_secs, b.fund_secs, b.k_z)
}

#[test]
fn mainnet_maker_keeps_the_paper_bounds() {
    let m = maker();
    m.timing.validate().unwrap();
    // W > F + B_R, t0 − open > F + ∆fund, t1 − t0 > F + B_R, t2 − t1 > D + F + B_R.
    m.timing.check_windows(&m.windows).unwrap();
    m.check_limits().unwrap();
    assert!(m.timing.fund_secs >= MEASURED_MAX_3_BLOCKS + m.timing.inclusion_secs, "fund_secs below k_Z mainnet blocks at the measured worst case plus D");
    assert_eq!(m.timing.k_z, 3);
    // The quote must be sent while the taker still waits, and a request must outlive both waits.
    assert!(m.quote_send_secs + MEASURED_P999_1_BLOCK < taker().quote_timeout_secs);
    assert!(m.quote_ttl_secs > 2 * taker().quote_timeout_secs + MEASURED_P999_1_BLOCK);
}

#[test]
fn mainnet_maker_requires_and_passes_the_premium_check() {
    let m = maker();
    assert_eq!(m.chains.network.as_deref(), Some("main"));
    m.check_premium().unwrap();
    // Without premium_check the maker refuses to start on mainnet (Maker::open calls check_premium).
    let mut none = m.clone();
    none.premium_check = None;
    assert!(none.check_premium().unwrap_err().to_string().contains("required on mainnet"));
    // One zatoshi less than the smallest passing premium at the template's prices is refused.
    let p = m.premium_check.clone().unwrap();
    let need = (m.stipend + p.costs_per_request_yocto).saturating_mul(100_000_000).div_ceil(p.yocto_near_per_zec);
    assert!(m.premium_zats as u128 >= need);
    let mut low = m.clone();
    low.premium_zats = (need - 1) as u64;
    assert!(low.check_premium().is_err());
    // The stipend pays the taker's commit_claim, claim and withdraw(id) with a margin of 2
    // (docs/experiments/light-payout.md: 0.06 NEAR; the largest balance NEAR checks is
    // 0.0278 NEAR). The taker accepts 0.03 NEAR, which works at today's gas price.
    assert_eq!(m.stipend, 6 * ONE_NEAR / 100);
    assert_eq!(taker().min_stipend, 3 * ONE_NEAR / 100);
    assert!(m.stipend >= 2 * (27_831 * ONE_NEAR / 1_000_000));
    // costs_per_request covers the account creation that the stipend transfer pays
    // (0.007115 NEAR burnt), the swap's escrow storage (522 bytes, 0.00522 NEAR), the
    // maker's deposit, open and set_ready gas, and a refund.
    assert!(p.costs_per_request_yocto >= 7_115 * ONE_NEAR / 1_000_000 + 5_220 * ONE_NEAR / 1_000_000 + 5 * ONE_NEAR / 1000);
    assert!(m.premium_zats <= taker().max_premium);
}

#[test]
fn mainnet_templates_name_the_mainnet_contracts_and_hold_placeholders() {
    let (m, t) = (maker(), taker());
    for c in [&m.chains, &t.chains] {
        assert_eq!(c.network.as_deref(), Some("main"));
        assert_eq!(c.token, USDC_MAINNET);
        assert_eq!(c.intents, VERIFIER_MAINNET);
        assert!(c.near_rpc.all_https(), "{}", c.near_rpc);
        assert!(c.near_rpc.as_slice().len() >= 2, "mainnet needs a second NEAR RPC endpoint (incident 2026-11-14): {}", c.near_rpc);
        let lwd = c.lightwalletd.as_deref().unwrap();
        assert!(lwd.starts_with("https://") && lwd.contains("mainnet"), "{lwd}");
        assert!(c.escrow.ends_with(".near"));
    }
    // Filled at deploy time; deploy/maker/railway/start.sh refuses a config that still has them.
    assert!(m.near_account.contains("OWNER") && m.chains.escrow.contains("OWNER"));
    assert_eq!(m.price_per_zec, 0, "the price is the owner's decision; the template must not carry one");
    assert!(t.escrow_code_hash.starts_with("REPLACE"));
    // The mainnet limits are smaller than testnet's: real money, unaudited code.
    let tn: MakerConfig = serde_json::from_str(TESTNET_MAKER).unwrap();
    assert!(m.max_zats <= tn.max_zats);
    // The maker does not check its USDC float before it quotes: an open that the float
    // cannot pay fails after the taker paid the premium. The float must cover every request
    // that can wait at once (docs/MAINNET.md, the float), so that number is kept small.
    assert!(m.max_pending_quotes <= 3);
}

#[test]
fn mainnet_taker_accepts_what_the_maker_quotes() {
    let (m, t) = (maker(), taker());
    t.timing.validate().unwrap();
    assert!(same(&m.timing, &t.timing), "maker and taker must use the same bounds");
    let tt = &t.timing;
    let w = &m.windows;
    assert!(w.reveal_secs as u64 > tt.finality_secs + tt.budget());
    assert!(w.t0_secs as u64 > tt.finality_secs + tt.fund_secs + 3 * t.cover.max_secs);
    assert!(w.t1_secs as u64 > tt.finality_secs + tt.budget());
    assert!((w.t0_secs + w.t1_secs + w.t2_secs) as u64 <= t.max_horizon_secs);
    // Request and quote: two blocks at the 99.9th percentile, with room for two proofs and the scans.
    assert!(t.quote_timeout_secs >= MEASURED_P999_2_BLOCKS + 120);
    assert!(t.quote_timeout_secs >= 2 * MEASURED_P999_1_BLOCK + 120);
    assert!(t.min_stipend <= m.stipend);
}

#[test]
fn website_mainnet_config_matches_the_taker_template() {
    let web: serde_json::Value = serde_json::from_str(WEB).unwrap();
    let wt: Timing = serde_json::from_value(web["timing"].clone()).unwrap();
    let t = taker();
    assert!(same(&wt, &t.timing), "web/public/config/mainnet.json timing differs from mainnet-taker.json");
    assert_eq!(web["quoteTimeoutSecs"].as_u64(), Some(t.quote_timeout_secs));
    assert_eq!(web["network"], "mainnet");
    assert_eq!(web["near"]["token"], USDC_MAINNET);
    assert_eq!(web["near"]["verifier"], VERIFIER_MAINNET);
    assert_eq!(web["near"]["tokenDecimals"], 6);
    let web_rpc: zecret_core::config::RpcUrls = serde_json::from_value(web["near"]["rpc"].clone()).unwrap();
    assert_eq!(web_rpc, t.chains.near_rpc, "web/public/config/mainnet.json near.rpc differs from mainnet-taker.json");
    assert_eq!(web["zcash"]["lightwalletd"].as_str(), t.chains.lightwalletd.as_deref());
    assert_eq!(web["exit"]["mode"], "oneclick");
    assert_eq!(web["minStipend"].as_str().unwrap().parse::<u128>().unwrap(), t.min_stipend);
    // The escrow and its code hash come from the build (VITE_MAINNET_ESCROW, ..._CODE_HASH), never from the file.
    assert_eq!(web["near"]["escrow"], "@build");
    assert_eq!(web["near"]["escrowCodeHash"], "@build");
}
