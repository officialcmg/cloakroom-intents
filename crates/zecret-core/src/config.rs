//! Protocol timing bounds and the settings of each party.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::wallet::CoverPolicy;

/// u128 amounts (token base units, yoctoNEAR) as decimal strings in JSON, because
/// JavaScript numbers cannot hold them. Numbers are accepted on input too.
pub mod u128_str {
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum V {
            S(String),
            N(u64),
        }
        match V::deserialize(d)? {
            V::S(s) => s.trim().parse().map_err(de::Error::custom),
            V::N(n) => Ok(n as u128),
        }
    }
}

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
        r.saturating_add(1).saturating_mul(d).saturating_add(r.saturating_mul(f)).saturating_add(self.margin_secs)
    }

    /// A key-revealing transaction sent at `now` is final before `deadline` even if it is
    /// reorganized R times: `now + B_R < deadline` (rules R2c, R2r, R3d).
    pub fn in_time(&self, now: u64, deadline: u64) -> bool {
        now.saturating_add(self.budget()) < deadline
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
            ("reveal window W", w.reveal_secs as u64, f.saturating_add(b)),
            ("t0 - open", w.t0_secs as u64, f.saturating_add(self.fund_secs)),
            ("t1 - t0", w.t1_secs as u64, f.saturating_add(b)),
            ("t2 - t1", w.t2_secs as u64, d.saturating_add(f).saturating_add(b)),
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
    /// NEAR JSON-RPC endpoints, in order of preference: one URL, or a list. The client
    /// uses the first that answers and moves to the next on a rate limit, a server error
    /// or a timeout. More endpoints give availability, not a second opinion: each answer
    /// is trusted as it comes (docs/ENGINE.md, NEAR RPC endpoints).
    pub near_rpc: RpcUrls,
    pub escrow: String,
    pub token: String,
    pub intents: String,
}

/// An ordered, non-empty list of endpoint URLs. In JSON it is one string (the format
/// before 2026-11-14) or an array of strings; one URL is written back as a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcUrls(Vec<String>);

impl RpcUrls {
    pub fn new(urls: Vec<String>) -> Result<RpcUrls> {
        let urls: Vec<String> = urls.into_iter().map(|u| u.trim().to_string()).collect();
        if urls.is_empty() || urls.iter().any(|u| u.is_empty()) {
            bail!("near_rpc needs at least one URL, and no empty one");
        }
        for (i, u) in urls.iter().enumerate() {
            if urls[..i].contains(u) {
                bail!("near_rpc lists {u} twice");
            }
        }
        Ok(RpcUrls(urls))
    }
    /// The first (preferred) endpoint.
    pub fn primary(&self) -> &str {
        &self.0[0]
    }
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
    /// True when every endpoint is an https URL.
    pub fn all_https(&self) -> bool {
        self.0.iter().all(|u| u.starts_with("https://"))
    }
}

impl From<&str> for RpcUrls {
    fn from(u: &str) -> RpcUrls {
        RpcUrls(vec![u.to_string()])
    }
}

impl From<String> for RpcUrls {
    fn from(u: String) -> RpcUrls {
        RpcUrls(vec![u])
    }
}

impl std::fmt::Display for RpcUrls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join(", "))
    }
}

impl Serialize for RpcUrls {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        if self.0.len() == 1 {
            s.serialize_str(&self.0[0])
        } else {
            self.0.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for RpcUrls {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<RpcUrls, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum V {
            One(String),
            Many(Vec<String>),
        }
        let urls = match V::deserialize(d)? {
            V::One(u) => vec![u],
            V::Many(v) => v,
        };
        RpcUrls::new(urls).map_err(serde::de::Error::custom)
    }
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
    /// The maker's limits, covered by `sig` when present (`keys::directory_digest_v2`).
    /// The taker checks a request against them before it sends anything. An entry without
    /// limits is signed with the first digest (`keys::directory_digest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<MakerLimits>,
}

impl MakerEntry {
    /// The digest that `sig` signs: with the limits when the entry has them.
    pub fn digest(&self) -> anyhow::Result<[u8; 32]> {
        let rfq_pub = crate::keys::unhex32(&self.rfq_pub)?;
        Ok(match &self.limits {
            Some(l) => crate::keys::directory_digest_v2(&self.ua, self.epoch, &rfq_pub, &self.near_account, l),
            None => crate::keys::directory_digest(&self.ua, self.epoch, &rfq_pub, &self.near_account),
        })
    }
}

/// What a maker publishes about the requests it takes (signed in its directory entry).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MakerLimits {
    /// Smallest request (zatoshis sold) the maker quotes.
    pub min_zats: u64,
    /// Largest request the maker quotes.
    pub max_zats: u64,
    /// The premium (zatoshis) the maker asks with every quote.
    pub premium_zats: u64,
}

/// Test-only behaviours. The scenario runner uses them to play a dishonest party and check
/// that the honest party's rules hold. An honest deployment leaves them unset.
///
/// A configuration can set `misbehave` only in a build with the cargo feature `misbehave`
/// (the scenario runner builds `zecret-node` with it). Other builds, such as the release
/// binary and the browser engine, refuse a configuration that sets it (security review 2,
/// H5).
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
    /// Claim, then never send withdraw: the payout stays Due until another account
    /// (the maker) sends it.
    NoWithdraw,
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

/// Reads the `misbehave` field. Without the cargo feature `misbehave`, any value other
/// than null is an error, so a production configuration cannot turn a test behaviour on.
fn de_misbehave<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Misbehave>, D::Error> {
    let v = Option::<Misbehave>::deserialize(d)?;
    if v.is_some() && !cfg!(feature = "misbehave") {
        return Err(serde::de::Error::custom(
            "misbehave is a test-only setting; this build does not accept it (build with the cargo feature `misbehave`)",
        ));
    }
    Ok(v)
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
    /// The code hash (base58 sha256 of the wasm, as `view_account` shows it) of the
    /// reviewed escrow build. Before funding, the taker checks that the escrow runs this
    /// code, has no access keys, and pays into `chains.intents`.
    pub escrow_code_hash: String,
    /// Smallest price (token base units) the taker accepts for the requested amount.
    #[serde(default, with = "u128_str")]
    pub min_price: u128,
    /// Largest premium (zatoshis) the taker accepts.
    pub max_premium: u64,
    /// Smallest stipend (yoctoNEAR) the taker accepts. It must pay for commit_claim, claim
    /// and withdraw (the largest balance NEAR checks in that sequence is 0.0278 NEAR at
    /// today's gas price; docs/experiments/light-payout.md).
    #[serde(with = "u128_str")]
    pub min_stipend: u128,
    /// Largest t2 − open the taker accepts (seconds), rule R1q's horizon H.
    #[serde(default = "default_horizon")]
    pub max_horizon_secs: u64,
    /// Seconds to wait for the quote, and for the escrow after the acceptance.
    #[serde(default = "default_quote_timeout")]
    pub quote_timeout_secs: u64,
    #[serde(default)]
    pub cover: Cover,
    #[serde(default, deserialize_with = "de_misbehave")]
    pub misbehave: Option<Misbehave>,
    /// Seconds (wall clock) after the claim is final before the engine reports that the
    /// payout is pending (stage `PayoutPending`, event `payout_pending`). The taker sends
    /// `withdraw(id)` itself from the stipend and keeps retrying; the stage means that
    /// the payout still failed (for example the Verifier refuses `mt_transfer`). Any NEAR
    /// account can send the same call.
    #[serde(default = "default_payout_wait")]
    pub payout_wait_secs: u64,
}

fn default_payout_wait() -> u64 {
    300
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
    #[serde(with = "u128_str")]
    pub price_per_zec: u128,
    pub min_zats: u64,
    pub max_zats: u64,
    /// Non-refundable premium (zatoshis) the taker pays with the acceptance.
    pub premium_zats: u64,
    /// yoctoNEAR sent to the taker's account right after `open`.
    #[serde(with = "u128_str")]
    pub stipend: u128,
    pub windows: Windows,
    /// When the maker's storage funds in the escrow fall below this (yoctoNEAR), the
    /// maker adds `storage_topup` before it opens a swap.
    #[serde(default = "default_storage_min", with = "u128_str")]
    pub storage_min: u128,
    #[serde(default = "default_storage_topup", with = "u128_str")]
    pub storage_topup: u128,
    #[serde(default)]
    pub cover: Cover,
    #[serde(default, deserialize_with = "de_misbehave")]
    pub misbehave: Option<Misbehave>,
    /// Requests waiting for a quote or for the acceptance (`Quoting` + `Quoted`). A request
    /// that arrives when this many wait is refused (security review 2, F2-04).
    #[serde(default = "default_max_pending_quotes")]
    pub max_pending_quotes: usize,
    /// Seconds (wall clock) after which a request that is still `Quoting` or `Quoted` is
    /// dropped (stage `Aborted`, secrets erased). It must be longer than the taker's
    /// `quote_timeout_secs` twice (the wait for the quote and the wait for the escrow) plus
    /// the time to mine the acceptance.
    #[serde(default = "default_quote_ttl")]
    pub quote_ttl_secs: u64,
    /// Seconds (wall clock) after the maker read a request within which its quote must be
    /// broadcast. A request still in `Quoting` after that is dropped (stage `Aborted`,
    /// secrets erased): the taker stops waiting after its `quote_timeout_secs`, counted
    /// from its own broadcast of the request, so a later quote is not used. Keep it below
    /// the takers' `quote_timeout_secs` minus the time to mine the quote.
    #[serde(default = "default_quote_send")]
    pub quote_send_secs: u64,
    /// Quotes the maker builds at most in one tick. Each one is a proof (seconds of CPU).
    #[serde(default = "default_max_quotes_per_tick")]
    pub max_quotes_per_tick: usize,
    /// Prices for the check that the premium pays for an accepted request (security review
    /// 2, NV2-02). Required on mainnet (`chains.network` = "main"). On other networks the
    /// check runs when it is set.
    #[serde(default)]
    pub premium_check: Option<PremiumCheck>,
    /// Refusal replies the maker sends at most in one hour (sliding window). A refusal tells
    /// a taker why its request gets no quote (amount above or below the limits, the maker is
    /// busy, the request is not supported). Each one costs the maker a Zcash fee and a proof,
    /// so they are capped, sent only from the quote budget that no quote used, and at most
    /// one per request. 0 turns the replies off.
    #[serde(default = "default_max_refusals_per_hour")]
    pub max_refusals_per_hour: usize,
    /// How often the maker reads NEAR for each open swap (docs/ENGINE.md, request budget).
    #[serde(default)]
    pub polling: Polling,
}

fn default_max_refusals_per_hour() -> usize {
    6
}

/// How often the maker reads the escrow of one open swap, in seconds (wall clock).
///
/// The maker reads a swap's escrow at the slow interval of its stage, and always just
/// after each deadline that matters for the stage (the end of the funding window, t0,
/// t1, t2, the end of the reveal window), but never more often than `fast_secs`. A read
/// that fails is tried again after a back-off that grows to `open_secs`, or to
/// `fast_secs` while a refund window is open. Incident 2026-11-14: the maker read every
/// swap every second and was rate-limited.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Polling {
    /// The shortest interval between two reads of one swap.
    pub fast_secs: u64,
    /// `Opened` with no confirmed funding: nothing can change on NEAR before a deadline.
    pub open_secs: u64,
    /// `Ready`, `Locked`, `Sweeping`: the taker acts; nothing is due before a deadline.
    pub ready_secs: u64,
    /// After a refund or another send of the maker, until a final block shows it.
    pub settle_secs: u64,
    /// Past t2 with the escrow still Open or Ready: only the taker can act.
    pub stuck_secs: u64,
    /// A deadline-critical NEAR step that has failed this long raises an alert.
    pub alert_after_secs: u64,
    /// The same alert or warning is repeated at most once in this time.
    pub quiet_secs: u64,
}

impl Default for Polling {
    fn default() -> Polling {
        Polling { fast_secs: 3, open_secs: 30, ready_secs: 10, settle_secs: 5, stuck_secs: 300, alert_after_secs: 60, quiet_secs: 600 }
    }
}

/// What an accepted request costs the maker, and the price of ZEC in NEAR. `Maker::open`
/// refuses to start when `premium_zats` is worth less than `stipend` +
/// `costs_per_request_yocto` at this price.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PremiumCheck {
    /// yoctoNEAR for 1 ZEC (10^8 zatoshis). Use a low estimate of the ZEC price.
    #[serde(with = "u128_str")]
    pub yocto_near_per_zec: u128,
    /// yoctoNEAR the maker spends for each accepted request besides the stipend: the
    /// account creation that the stipend transfer pays (0.007 NEAR since protocol 85), the
    /// gas of its Verifier deposit, `open` (`mt_transfer_call`), `set_ready` and a refund,
    /// and the escrow storage of one swap. Default 0.015 NEAR; the mainnet template uses
    /// 0.02 NEAR (docs/MAINNET.md); measure it on the network in use.
    #[serde(default = "default_costs_per_request", with = "u128_str")]
    pub costs_per_request_yocto: u128,
}

fn default_costs_per_request() -> u128 {
    crate::near::ONE_NEAR * 15 / 1000
}

fn default_max_pending_quotes() -> usize {
    32
}

fn default_quote_ttl() -> u64 {
    3600
}

fn default_quote_send() -> u64 {
    300
}

fn default_max_quotes_per_tick() -> usize {
    1
}

fn default_storage_min() -> u128 {
    crate::near::ONE_NEAR / 100
}

fn default_storage_topup() -> u128 {
    crate::near::ONE_NEAR / 10
}

impl MakerConfig {
    /// The limits this maker publishes in its directory entry.
    pub fn limits(&self) -> MakerLimits {
        MakerLimits { min_zats: self.min_zats, max_zats: self.max_zats, premium_zats: self.premium_zats }
    }

    /// Token base units for `zats` zatoshis, rounded down.
    pub fn price_for(&self, zats: u64) -> u128 {
        zats as u128 * self.price_per_zec / 100_000_000
    }

    /// NV2-02: the premium must be worth at least the stipend plus the maker's own costs
    /// for one accepted request, at the configured price. Required on mainnet.
    pub fn check_premium(&self) -> Result<()> {
        let mainnet = matches!(self.chains.network.as_deref(), Some("main") | Some("mainnet"));
        let Some(p) = &self.premium_check else {
            if mainnet {
                bail!("premium_check is required on mainnet: give yocto_near_per_zec (and costs_per_request_yocto) so the maker can check that premium_zats pays for the stipend and its costs");
            }
            return Ok(());
        };
        let worth = (self.premium_zats as u128).checked_mul(p.yocto_near_per_zec).map(|v| v / 100_000_000);
        let cost = self.stipend.checked_add(p.costs_per_request_yocto);
        match (worth, cost) {
            (Some(w), Some(c)) if w >= c => Ok(()),
            (Some(w), Some(c)) => bail!(
                "premium_zats {} is worth {w} yoctoNEAR at {} yoctoNEAR per ZEC, less than the stipend {} + costs {} = {c} yoctoNEAR that each accepted request costs the maker; raise premium_zats to at least {}",
                self.premium_zats,
                p.yocto_near_per_zec,
                self.stipend,
                p.costs_per_request_yocto,
                c.saturating_mul(100_000_000).div_ceil(p.yocto_near_per_zec.max(1))
            ),
            _ => bail!("premium_check: amounts overflow"),
        }
    }

    /// The limits on unpaid requests (F2-04) must allow some work.
    pub fn check_limits(&self) -> Result<()> {
        if self.max_pending_quotes == 0 || self.max_quotes_per_tick == 0 || self.quote_ttl_secs == 0 || self.quote_send_secs == 0 {
            bail!("max_pending_quotes, max_quotes_per_tick, quote_ttl_secs and quote_send_secs must be at least 1");
        }
        if self.quote_send_secs > self.quote_ttl_secs {
            bail!("quote_send_secs must not be above quote_ttl_secs");
        }
        if self.min_zats == 0 || self.min_zats > self.max_zats {
            bail!("min_zats must be at least 1 and not above max_zats");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing() -> Timing {
        Timing { inclusion_secs: 30, finality_secs: 10, reorg_budget: 2, margin_secs: 0, fund_secs: 120, k_z: 2 }
    }

    #[test]
    fn u128_fields_read_strings_and_numbers() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct T {
            #[serde(with = "u128_str")]
            v: u128,
        }
        let t: T = serde_json::from_str(r#"{"v": "30000000000000000000000000"}"#).unwrap();
        assert_eq!(t.v, 30 * 10u128.pow(24));
        assert_eq!(serde_json::from_str::<T>(r#"{"v": 7}"#).unwrap().v, 7);
        assert_eq!(serde_json::to_string(&t).unwrap(), r#"{"v":"30000000000000000000000000"}"#);
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
