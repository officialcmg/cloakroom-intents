//! Protocol timing bounds and the settings of each party.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::wallet::CoverPolicy;

/// Timing bounds of the paper (Section 4), measured on the NEAR network in use. All in
/// seconds of NEAR block time.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Timing {
    /// D: bound on the delay until an honest transaction is included.
    pub inclusion_secs: u64,
    /// F: finality lag. A block is final F after it is produced.
    pub finality_secs: u64,
    /// R: number of reorganizations a key-revealing transaction must survive (R ≥ 1).
    pub reorg_budget: u64,
    /// Extra safety margin added to the budget B_R.
    #[serde(default)]
    pub margin_secs: u64,
    /// ∆fund (rule R1b): time from the taker's funding until the maker can set Ready:
    /// k_Z Zcash confirmations at a high percentile of block intervals, plus D.
    pub fund_secs: u64,
    /// k_Z: Zcash confirmations the maker waits for before Ready, and that a sweep needs
    /// before the secrets are erased.
    pub k_z: u64,
}

impl Timing {
    /// The reorganization budget B_R = (R+1)·D + R·F, plus the extra margin.
    pub fn budget(&self) -> u64 {
        let (d, f, r) = (self.inclusion_secs, self.finality_secs, self.reorg_budget);
        (r + 1) * d + r * f + self.margin_secs
    }

    /// A key-revealing transaction sent at `now` is final before `deadline` even if it is
    /// reorganized R times: `now + B_R < deadline` (rules R2c, R2r, R3d).
    pub fn in_time(&self, now: u64, deadline: u64) -> bool {
        now + self.budget() < deadline
    }

    pub fn validate(&self) -> Result<()> {
        if self.reorg_budget == 0 {
            bail!("reorg_budget R must be at least 1");
        }
        if self.k_z == 0 {
            bail!("k_z must be at least 1");
        }
        Ok(())
    }

    /// The paper's parameter bounds for windows a maker quotes:
    /// W > F + B_R, t0 − open > F + ∆fund, t1 − t0 > F + B_R, t2 − t1 > D + F + B_R.
    pub fn check_windows(&self, w: &Windows) -> Result<()> {
        let (d, f, b) = (self.inclusion_secs, self.finality_secs, self.budget());
        let checks = [
            ("reveal window W", w.reveal_secs as u64, f + b),
            ("t0 - open", w.t0_secs as u64, f + self.fund_secs),
            ("t1 - t0", w.t1_secs as u64, f + b),
            ("t2 - t1", w.t2_secs as u64, d + f + b),
        ];
        for (what, have, bound) in checks {
            if have <= bound {
                bail!("unsafe parameters: {what} = {have}s must be > {bound}s (D={d}, F={f}, R={}, B_R={b})", self.reorg_budget);
            }
        }
        Ok(())
    }
}

/// The windows a maker quotes, relative to the time of `open`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Windows {
    /// t0 = open + t0_secs: last moment for Ready.
    pub t0_secs: u32,
    /// t1 = t0 + t1_secs: last moment for a normal commit.
    pub t1_secs: u32,
    /// t2 = t1 + t2_secs: the punish path opens.
    pub t2_secs: u32,
    /// W: reveal window after commit_claim.
    pub reveal_secs: u32,
}

/// Where the parties reach the two chains.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chains {
    /// Zebra JSON-RPC URL (regtest and tests). Used when `lightwalletd` is not set.
    #[serde(default)]
    pub zebra_rpc: Option<String>,
    /// lightwalletd gRPC-web base URL (public networks).
    #[serde(default)]
    pub lightwalletd: Option<String>,
    /// First block a light wallet scans. Required with `lightwalletd`.
    #[serde(default)]
    pub birthday: Option<u64>,
    /// "regtest" (default), "test" or "main": address encoding and consensus rules.
    #[serde(default)]
    pub network: Option<String>,
    /// Regtest only: NU7 activation height, if NU7 is active.
    #[serde(default)]
    pub nu7_height: Option<u32>,
    pub near_rpc: String,
    pub escrow: String,
    pub token: String,
    pub intents: String,
}

/// The maker's directory entry: what a taker must know about a maker before the first
/// message. `sig` is the maker's ed25519 signature over `keys::directory_digest`, by the
/// NEAR key `near_pk`, which must be an access key of `near_account`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MakerEntry {
    pub ua: String,
    pub epoch: u32,
    /// Hex of the maker's RFQ encryption key (Pallas point).
    pub rfq_pub: String,
    pub near_account: String,
    /// `ed25519:<base58>`.
    pub near_pk: String,
    /// Hex of the 64-byte ed25519 signature.
    pub sig: String,
}

/// Test-only behaviours. The scenario runner uses them to play a dishonest party and check
/// that the honest party's rules hold. An honest deployment leaves them unset.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Misbehave {
    // taker
    /// Fund half the amount.
    Underfund,
    /// Never fund; try to lock the escrow anyway.
    FreeClaim,
    /// Send commit_claim after t1 on purpose.
    LateCommit,
    /// Lock, then never reveal a.
    NoReveal,
    // maker
    /// Quote a rogue key B = b'·G − A.
    RogueKey,
    /// Quote a reveal window too short for finality.
    ShortReveal,
    /// Quote a punish deadline far in the future.
    FarHorizon,
    /// Never set Ready.
    NeverReady,
    /// Open the escrow but send no stipend.
    NoStipend,
    /// Open the escrow with less than the quoted price.
    ShortOpen,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct Cover {
    #[serde(default)]
    pub min_secs: u64,
    #[serde(default)]
    pub max_secs: u64,
    #[serde(default)]
    pub k: u64,
}

impl From<Cover> for CoverPolicy {
    fn from(c: Cover) -> CoverPolicy {
        CoverPolicy { min_secs: c.min_secs, max_secs: c.max_secs, k: c.k }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TakerConfig {
    pub chains: Chains,
    pub timing: Timing,
    pub maker: MakerEntry,
    /// Smallest price (token base units) the taker accepts for the requested amount.
    #[serde(default)]
    pub min_price: u128,
    /// Largest premium (zatoshis) the taker accepts.
    pub max_premium: u64,
    /// Smallest stipend (yoctoNEAR) the taker accepts. It must pay for commit_claim and claim.
    pub min_stipend: u128,
    /// Largest t2 − open the taker accepts (seconds), rule R1q's horizon H.
    #[serde(default = "default_horizon")]
    pub max_horizon_secs: u64,
    /// Seconds to wait for the quote, and for the escrow after the acceptance.
    #[serde(default = "default_quote_timeout")]
    pub quote_timeout_secs: u64,
    #[serde(default)]
    pub cover: Cover,
    #[serde(default)]
    pub misbehave: Option<Misbehave>,
}

fn default_horizon() -> u64 {
    48 * 3600
}

fn default_quote_timeout() -> u64 {
    600
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MakerConfig {
    pub chains: Chains,
    pub timing: Timing,
    /// The maker's NEAR account. Its key is in the store.
    pub near_account: String,
    /// Token base units paid for 1 ZEC (10^8 zatoshis).
    pub price_per_zec: u128,
    pub min_zats: u64,
    pub max_zats: u64,
    /// Non-refundable premium (zatoshis) the taker pays with the acceptance.
    pub premium_zats: u64,
    /// yoctoNEAR sent to the taker's account right after `open`.
    pub stipend: u128,
    pub windows: Windows,
    /// When the maker's storage funds in the escrow fall below this (yoctoNEAR), the
    /// maker adds `storage_topup` before it opens a swap.
    #[serde(default = "default_storage_min")]
    pub storage_min: u128,
    #[serde(default = "default_storage_topup")]
    pub storage_topup: u128,
    #[serde(default)]
    pub cover: Cover,
    #[serde(default)]
    pub misbehave: Option<Misbehave>,
}

fn default_storage_min() -> u128 {
    crate::near::ONE_NEAR / 100
}

fn default_storage_topup() -> u128 {
    crate::near::ONE_NEAR / 10
}

impl MakerConfig {
    /// Token base units for `zats` zatoshis, rounded down.
    pub fn price_for(&self, zats: u64) -> u128 {
        zats as u128 * self.price_per_zec / 100_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing() -> Timing {
        Timing { inclusion_secs: 30, finality_secs: 10, reorg_budget: 2, margin_secs: 0, fund_secs: 120, k_z: 2 }
    }

    #[test]
    fn budget_matches_the_paper_example() {
        // docs/DESIGN.md section 7: D = 30 s, F = 10 s, R = 2 gives B_R = 110 s.
        let t = timing();
        assert_eq!(t.budget(), 110);
        assert!(t.in_time(1000, 1111));
        assert!(!t.in_time(1000, 1110));
    }

    #[test]
    fn window_bounds_are_strict() {
        let t = timing();
        let ok = Windows { t0_secs: 131, t1_secs: 121, t2_secs: 151, reveal_secs: 121 };
        t.check_windows(&ok).unwrap();
        for bad in [
            Windows { reveal_secs: 120, ..ok },
            Windows { t0_secs: 130, ..ok },
            Windows { t1_secs: 120, ..ok },
            Windows { t2_secs: 150, ..ok },
        ] {
            assert!(t.check_windows(&bad).is_err(), "{bad:?}");
        }
        assert!(Timing { reorg_budget: 0, ..t }.validate().is_err());
    }
}
