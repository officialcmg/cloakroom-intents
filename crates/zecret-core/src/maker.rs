//! The maker: quotes requests that arrive as encrypted memos, opens escrows on NEAR, sends
//! the taker's stipend, sets Ready, and takes the ZEC once the taker claims (or refunds).
//!
//! Like the taker, it never sleeps and reads no clock: the daemon calls [`Maker::tick`]
//! in a loop. One maker handles any number of swaps at once.
//!
//! Stages of one swap (`MakerStage`), with the wallet rules of the paper:
//!
//! ```text
//! Quoting  --quote memo broadcast-----------------------> Quoted
//! Quoted   --Accept with premium, seed opens------------> Opening
//! Opening  --open (mt_transfer_call) + stipend----------> Opened
//! Opened   --joint note >= amount, k_Z conf, before t0--> Ready
//! Opened/Ready --claim(a) seen: sweep with a+b----------> Sweeping --k_Z conf--> Swept
//! Opened   --no note by the end of R1b (R3e)------------> RefundSent
//! Opened/Ready --final block >= t1, now + B_R < t2 (R3d)> RefundSent
//! Locked   --final block >= lockedAt + W (R3l)----------> RefundSent
//! RefundSent --refund final, payout Paid (withdraw if Due)--> Refunded
//! ```

use anyhow::{anyhow, bail, Context, Result};
use group::GroupEncoding;
use orchard::keys::{FullViewingKey, Scope};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    config::{MakerConfig, MakerEntry, Misbehave},
    engine::{self, drive_pending, short, Event, Next, PendingState, PendingZ, Tick},
    keys,
    msg::{self, Envelope, Quote, RefuseCode, Refusal, Rfq},
    near::{escrow, rpc::unavailable, tx::Action, At, Escrow, NearKey, NearRpc, Signer, SwapState, SwapView},
    net::SharedTransport,
    store::{get_json, put_json, Store},
    wallet::{self, CoverWait, Received, Wallet},
    Me,
};

pub const POLL_SECS: u64 = 1;
pub const RETRY_SECS: u64 = 5;
/// The same warning is repeated at most once in this time (seconds).
pub const WARN_QUIET_SECS: u64 = 60;

pub const KEY_WALLET: &str = "wallet";
pub const KEY_NEAR: &str = "near_key";
pub const KEY_RFQ: &str = "rfq_key";
pub const KEY_SEEN: &str = "seen";
pub const PREFIX_SWAP: &str = "swap/";
pub const PREFIX_REFUSAL: &str = "refusal/";

/// A refusal reply the maker owes a request it does not quote (see
/// `MakerConfig::max_refusals_per_hour`). Stored before its transaction is built, and with
/// the built transaction before the broadcast, like a quote.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MakerRefusal {
    pub rfq_id: String,
    /// `over_limit`, `under_minimum`, `busy` or `unsupported`.
    pub code: String,
    /// Hex of the request's reply address (43 bytes).
    pub reply: String,
    /// Hex of the sealed memo.
    pub memo: String,
    /// Wall-clock time the request was refused.
    pub created_at: u64,
    pub pending: Option<PendingZ>,
    /// True once a node took the reply, or once it was dropped.
    pub done: bool,
    pub txid: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum MakerStage {
    Quoting,
    Quoted,
    Opening,
    Opened,
    Ready,
    /// refund(b) sent; b is public. Kept until the refund is final (sent again if dropped).
    RefundSent,
    /// Sweep of the joint note built; kept until it has k_Z confirmations.
    Sweeping,
    Swept,
    Refunded,
    Aborted(String),
}

impl MakerStage {
    pub fn name(&self) -> &'static str {
        match self {
            MakerStage::Quoting => "Quoting",
            MakerStage::Quoted => "Quoted",
            MakerStage::Opening => "Opening",
            MakerStage::Opened => "Opened",
            MakerStage::Ready => "Ready",
            MakerStage::RefundSent => "RefundSent",
            MakerStage::Sweeping => "Sweeping",
            MakerStage::Swept => "Swept",
            MakerStage::Refunded => "Refunded",
            MakerStage::Aborted(_) => "Aborted",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, MakerStage::Swept | MakerStage::Refunded | MakerStage::Aborted(_))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MakerSwap {
    pub rfq_id: String,
    pub stage: MakerStage,
    pub zats: u64,
    pub price: String,
    pub premium: u64,
    pub stipend: String,
    pub taker_pk: String,
    pub taker_account: String,
    pub a_pub: String,
    pub commit_s_t: String,
    /// Maker share, un-normalized. Erased in a terminal stage.
    pub b: String,
    /// Erased in a terminal stage.
    pub s_m: String,
    /// Ephemeral DH secret. Erased in a terminal stage.
    pub e_m: String,
    pub e_t_pub: String,
    pub quote_memo: String,
    pub quote_hash: String,
    pub windows: crate::config::Windows,
    /// Zcash height of the request (cover trigger for the quote).
    pub rfq_height: u64,
    pub accept_height: Option<u64>,
    pub negated: bool,
    pub joint_fvk: Option<String>,
    pub salt: Option<String>,
    pub escrow_id: Option<String>,
    pub t0: Option<u64>,
    pub t1: Option<u64>,
    pub t2: Option<u64>,
    pub pending_then: Option<MakerStage>,
    pub pending: Option<PendingZ>,
    pub quote_txid: Option<String>,
    pub open_tx: Option<String>,
    pub stipend_tx: Option<String>,
    pub ready_tx: Option<String>,
    pub refund_tx: Option<String>,
    /// NEAR time of the last refund broadcast.
    pub refund_at: Option<u64>,
    pub sweep_txid: Option<String>,
    pub sweep_height: Option<u64>,
    /// NEAR time of the maker's last `withdraw(id)` for the taker's payout.
    pub taker_withdraw_at: Option<u64>,
    pub log: Vec<String>,
    /// Wall-clock time the request was taken (F2-04: `Quoting` and `Quoted` expire after
    /// `quote_ttl_secs`). Records from before this field get the time of their next tick.
    #[serde(default)]
    pub created_at: Option<u64>,
    /// NEAR time of the maker's last `withdraw(id)` of its own refund.
    #[serde(default)]
    pub withdraw_at: Option<u64>,
    /// NEAR time at which the maker first saw the claimed swap's payout Due at a final
    /// block. The taker sends withdraw itself; the maker sends it only when the payout
    /// stays Due for a while.
    #[serde(default)]
    pub payout_due_at: Option<u64>,
}

impl MakerSwap {
    fn id_bytes(&self) -> Result<[u8; 32]> {
        keys::unhex32(self.escrow_id.as_deref().ok_or_else(|| anyhow!("no escrow id"))?)
    }
    fn b_norm(&self) -> Result<pasta_curves::pallas::Scalar> {
        Ok(keys::normalize(keys::scalar_from_hex(&self.b)?, self.negated))
    }
}

/// The maker's NEAR reads of one open swap (in memory; after a restart every swap is
/// read at once).
#[derive(Clone, Debug, Default)]
struct Poll {
    /// Wall-clock time of the next read.
    due: u64,
    /// Failed steps in a row.
    fails: u32,
    /// What the last read showed.
    seen: Option<Seen>,
    /// Since when the swap's NEAR steps have failed without a break.
    critical_fail_since: Option<u64>,
    /// The confirmed funding was seen in the wallet (one immediate read for set_ready).
    funded_seen: bool,
}

/// The escrow as one read showed it.
#[derive(Clone, Copy, Debug)]
struct Seen {
    state: SwapState,
    /// NEAR time of the final block read.
    tnow: u64,
    /// Wall-clock time of the read.
    at: u64,
    t0: u64,
    t1: u64,
    t2: u64,
    /// lockedAt + W, when Locked.
    reveal_end: Option<u64>,
}

impl Seen {
    /// NEAR time now, estimated from the last read.
    fn est(&self, now: u64) -> u64 {
        self.tnow + now.saturating_sub(self.at)
    }
}

/// An alert that is active for a swap (for `/health`).
#[derive(Clone, Debug, Serialize)]
pub struct ActiveAlert {
    pub kind: String,
    pub message: String,
    pub since: u64,
    pub last: u64,
}

#[derive(Serialize, Deserialize)]
struct RfqKey {
    epoch: u32,
    sk: String,
}

pub struct Maker {
    pub cfg: MakerConfig,
    pub me: Me,
    pub wallet: Wallet,
    pub near: NearRpc,
    pub signer: Signer,
    epoch: u32,
    rfq_sk: pasta_curves::pallas::Scalar,
    seen: BTreeSet<String>,
    swaps: BTreeMap<String, MakerSwap>,
    covers: HashMap<String, CoverWait>,
    /// Cover delays that have passed (swap and broadcast), so a step that takes two ticks
    /// does not wait twice.
    covers_passed: BTreeSet<String>,
    ev: Vec<Event>,
    /// True in a tick whose Zcash sync failed: the wallet's view is old (H2).
    zcash_stale: bool,
    /// Quotes that may still be built in this tick (F2-04).
    quote_budget: usize,
    /// `Quoting` + `Quoted` swaps, counted at the start of the message scan (F2-04).
    pending_quotes: usize,
    /// The wallet could not pay a quote fee at the last look (to warn once, not every tick).
    quote_funds_low: bool,
    /// Refusal replies of the last hour and the ones not sent yet.
    refusals: BTreeMap<String, MakerRefusal>,
    /// NEAR reads of each open swap.
    polls: HashMap<String, Poll>,
    /// Repeated warnings and alerts held back.
    quiet: engine::Quiet,
    /// Active alerts by swap.
    alerts: BTreeMap<String, ActiveAlert>,
    /// True in a tick in which a deadline-critical NEAR step failed: other NEAR work waits.
    near_trouble: bool,
    /// Wall-clock time of the last Zcash sync that worked.
    last_zcash_ok: Option<u64>,
    /// Wall-clock time of the current tick.
    wall_now: u64,
}

impl Maker {
    /// Opens the maker. The NEAR key must be in the store under `near_key`
    /// (`ed25519:...`); the wallet key and the RFQ key are created on first use.
    pub fn open(cfg: MakerConfig, t: SharedTransport, store: &mut dyn Store) -> Result<Maker> {
        cfg.timing.validate()?;
        cfg.timing.check_windows(&cfg.windows)?;
        cfg.check_limits()?;
        cfg.check_premium()?;
        if cfg.misbehave.is_some() && !cfg!(feature = "misbehave") {
            bail!("misbehave is a test-only setting; this build does not accept it");
        }
        let me = match store.get(KEY_WALLET)? {
            Some(b) => Me::from_hex(std::str::from_utf8(&b)?)?,
            None => {
                let me = Me::generate();
                store.put(KEY_WALLET, hex::encode(me.sk_bytes()).as_bytes())?;
                me
            }
        };
        let key = NearKey::from_secret_str(std::str::from_utf8(&store.get(KEY_NEAR)?.ok_or_else(|| anyhow!("no NEAR key in the store ({KEY_NEAR})"))?)?)?;
        let rk: RfqKey = match get_json(store, KEY_RFQ)? {
            Some(k) => k,
            None => {
                let k = RfqKey { epoch: 1, sk: hex::encode(keys::scalar_to_bytes(&keys::random_scalar())) };
                put_json(store, KEY_RFQ, &k)?;
                k
            }
        };
        let mut wallet = engine::wallet_for(&cfg.chains, t.clone())?;
        wallet.watch("wallet", &me.fvk, &[Scope::External, Scope::Internal]);
        let near = NearRpc::with_urls(t, &cfg.chains.near_rpc);
        let seen = get_json(store, KEY_SEEN)?.unwrap_or_default();
        let mut swaps = BTreeMap::new();
        for k in store.keys(PREFIX_SWAP)? {
            let s: MakerSwap = get_json(store, &k)?.ok_or_else(|| anyhow!("record {k} vanished"))?;
            swaps.insert(s.rfq_id.clone(), s);
        }
        let signer = Signer::new(&cfg.near_account, key);
        let mut refusals = BTreeMap::new();
        for k in store.keys(PREFIX_REFUSAL)? {
            let r: MakerRefusal = get_json(store, &k)?.ok_or_else(|| anyhow!("record {k} vanished"))?;
            refusals.insert(r.rfq_id.clone(), r);
        }
        let mut m = Maker { epoch: rk.epoch, rfq_sk: keys::scalar_from_hex(&rk.sk)?, cfg, me, wallet, near, signer, seen, swaps, covers: HashMap::new(), covers_passed: BTreeSet::new(), ev: vec![], zcash_stale: false, quote_budget: 0, pending_quotes: 0, quote_funds_low: false, refusals, polls: HashMap::new(), quiet: engine::Quiet::default(), alerts: BTreeMap::new(), near_trouble: false, last_zcash_ok: None, wall_now: 0 };
        for r in m.refusals.values() {
            if let Some(p) = &r.pending {
                m.wallet.track(&p.txid);
            }
        }
        for s in m.swaps.values().cloned().collect::<Vec<_>>() {
            if !s.stage.is_terminal() {
                if let Some(f) = &s.joint_fvk {
                    let b: [u8; 96] = hex::decode(f)?.try_into().map_err(|_| anyhow!("fvk length"))?;
                    let fvk = FullViewingKey::from_bytes(&b).ok_or_else(|| anyhow!("bad joint fvk"))?;
                    m.wallet.watch(&s.rfq_id, &fvk, &[Scope::External]);
                }
                for t in [s.pending.as_ref().map(|p| &p.txid), s.sweep_txid.as_ref()].into_iter().flatten() {
                    m.wallet.track(t);
                }
            }
        }
        Ok(m)
    }

    pub fn ua(&self) -> String {
        self.me.ua()
    }

    pub fn swaps(&self) -> impl Iterator<Item = &MakerSwap> {
        self.swaps.values()
    }

    /// The maker's signed directory entry, for takers. It publishes the limits (smallest
    /// and largest request, premium); the signature covers them.
    pub fn directory_entry(&self) -> MakerEntry {
        let rfq_pub = msg::dh_public(&self.rfq_sk);
        let ua = self.ua();
        let limits = self.cfg.limits();
        let digest = keys::directory_digest_v2(&ua, self.epoch, &rfq_pub, &self.cfg.near_account, &limits);
        MakerEntry {
            ua,
            epoch: self.epoch,
            rfq_pub: hex::encode(rfq_pub),
            near_account: self.cfg.near_account.clone(),
            near_pk: self.signer.key.public_key_str(),
            sig: hex::encode(self.signer.key.sign(&digest)),
            limits: Some(limits),
        }
    }

    pub fn refusals(&self) -> impl Iterator<Item = &MakerRefusal> {
        self.refusals.values()
    }

    fn esc(&self) -> Escrow<'_> {
        Escrow { rpc: &self.near, escrow: self.cfg.chains.escrow.clone(), token: self.cfg.chains.token.clone(), intents: self.cfg.chains.intents.clone() }
    }

    pub fn tick(&mut self, now: u64, store: &mut dyn Store) -> Tick {
        self.near.set_time(now);
        self.near_trouble = false;
        self.wall_now = now;
        let next = match self.step(now, store) {
            Ok(n) => n,
            Err(e) => {
                self.ev.push(Event::Warning { swap: String::new(), message: format!("{e:#}") });
                Next::At(now + RETRY_SECS)
            }
        };
        for line in self.wallet.take_notes() {
            self.ev.push(Event::Log { swap: String::new(), line: format!("Zcash wallet: {line}") });
        }
        // Never "at once" twice in a row without a pause when NEAR is failing.
        let next = match next {
            Next::At(t) if self.near_trouble => Next::At(t.max(now + 1)),
            n => n,
        };
        let events = std::mem::take(&mut self.ev);
        let events = self.quiet.filter(events, now, WARN_QUIET_SECS);
        Tick { next, events }
    }

    /// One pass of the maker (security review 2, F2-04 and H2):
    ///
    /// 1. Sync the Zcash wallet. A failure is a warning: the NEAR duties still run, with
    ///    the wallet's older view.
    /// 2. Advance every swap from `Opening` on. These carry the deadlines (R3d, R3l, L_M),
    ///    so no amount of unpaid requests can delay them.
    /// 3. Read new memos (requests are refused above `max_pending_quotes`).
    /// 4. Expire old `Quoting` and `Quoted` swaps, and build at most `max_quotes_per_tick`
    ///    quotes, none when the wallet cannot pay a quote fee.
    fn step(&mut self, now: u64, store: &mut dyn Store) -> Result<Next> {
        self.zcash_stale = match self.wallet.sync() {
            Ok(()) => {
                self.last_zcash_ok = Some(now);
                false
            }
            Err(e) => {
                self.ev.push(Event::Warning { swap: String::new(), message: format!("Zcash sync: {e:#}; NEAR duties go on with the last synced view") });
                true
            }
        };
        let quoting = |s: &MakerSwap| matches!(s.stage, MakerStage::Quoting | MakerStage::Quoted);
        let mut next = now + POLL_SECS * 2;
        // Deadline work first.
        let mut ids: Vec<String> = self.swaps.iter().filter(|(_, s)| !s.stage.is_terminal() && !quoting(s)).map(|(k, _)| k.clone()).collect();
        // The swaps whose next step protects funds go first (their refund or set_ready).
        ids.sort_by_key(|id| (!self.is_critical(&self.swaps[id], now), self.next_deadline(id, now).map(|(_, t)| t).unwrap_or(u64::MAX)));
        self.advance_all(ids, now, store, &mut next)?;
        // Incoming messages.
        self.pending_quotes = self.swaps.values().filter(|s| quoting(s)).count();
        let incoming: Vec<Received> = self.wallet.received.iter().filter(|r| r.label == "wallet").cloned().collect();
        let mut seen_changed = false;
        for r in incoming {
            let tag = format!("{}:{}", r.txid, r.pos);
            if self.seen.contains(&tag) {
                continue;
            }
            let handled = match Envelope::decode(&r.memo) {
                Ok(Some(Envelope::Rfq { epoch, eph, ct })) => {
                    if let Err(e) = self.on_rfq(epoch, eph, &ct, r.height, now, store) {
                        self.ev.push(Event::Log { swap: String::new(), line: format!("RFQ rejected: {e:#}") });
                    }
                    true
                }
                Ok(Some(Envelope::Accept { rfq_id, ct })) => match self.on_accept(rfq_id, &ct, r.note.value().inner(), r.height, now, store) {
                    Ok(done) => {
                        if done {
                            next = now; // a swap may now be Opening: come back at once
                        }
                        done
                    }
                    Err(e) => {
                        self.ev.push(Event::Warning { swap: hex::encode(rfq_id), message: format!("ACCEPT processing failed, will retry: {e:#}") });
                        false
                    }
                },
                _ => true,
            };
            if handled {
                self.seen.insert(tag);
                seen_changed = true;
            }
        }
        if seen_changed {
            put_json(store, KEY_SEEN, &self.seen)?;
        }
        // Quoting work last, bounded per tick. No quote without ZEC for its fee: that check
        // costs no RPC round trip.
        let fee = wallet::zip317_fee(1, 2);
        let can_pay = !self.zcash_stale && self.wallet.spendable("wallet") >= fee;
        self.quote_budget = if can_pay { self.cfg.max_quotes_per_tick } else { 0 };
        let ids: Vec<String> = self.swaps.iter().filter(|(_, s)| quoting(s)).map(|(k, _)| k.clone()).collect();
        // Requests that still need a quote built. A quote that is built but not yet
        // broadcast (pending) holds its note: that is not a lack of funds.
        let waiting = ids.iter().any(|id| self.swaps[id].stage == MakerStage::Quoting && self.swaps[id].pending.is_none());
        // Short of funds only when the wallet's whole balance, notes set aside for
        // transactions in flight included, cannot pay a quote fee. Set-aside notes come
        // back (as change) when their transaction is mined.
        let short = self.wallet.balance("wallet") < fee;
        if waiting && !can_pay && short && !self.zcash_stale && !self.quote_funds_low {
            let have = self.wallet.balance("wallet");
            self.ev.push(Event::Warning { swap: String::new(), message: format!("requests wait for a quote, but the wallet holds {have} zats, less than a quote fee ({fee})") });
        }
        if !self.zcash_stale {
            self.quote_funds_low = waiting && !can_pay && short;
        }
        self.advance_all(ids, now, store, &mut next)?;
        // Refusal replies last: only from the quote budget that no quote used, and never
        // while a request waits for its quote (quotes come first).
        let waiting = self.swaps.values().any(|s| s.stage == MakerStage::Quoting && s.pending.is_none());
        self.advance_refusals(now, waiting, store, &mut next)?;
        if self.zcash_stale {
            next = next.min(now + RETRY_SECS);
        }
        Ok(Next::At(next))
    }

    fn advance_all(&mut self, ids: Vec<String>, now: u64, store: &mut dyn Store, next: &mut u64) -> Result<()> {
        for id in ids {
            let mut s = self.swaps[&id].clone();
            let before = s.clone();
            let r = self.advance(&mut s, now);
            if s != before {
                self.save(&mut s, store)?;
            }
            match r {
                Ok(Next::At(t)) => *next = (*next).min(t),
                Ok(Next::Done) => {}
                Err(e) => self.ev.push(Event::Warning { swap: id.clone(), message: format!("{e:#}") }),
            }
        }
        Ok(())
    }

    fn save(&mut self, s: &mut MakerSwap, store: &mut dyn Store) -> Result<()> {
        if s.stage.is_terminal() && !s.b.is_empty() {
            // R4: erase per-swap secrets once nothing more can be done with them.
            s.b.clear();
            s.s_m.clear();
            s.e_m.clear();
            s.joint_fvk = None;
            self.ev.push(Event::Done { swap: s.rfq_id.clone(), outcome: s.stage.name().into() });
        }
        put_json(store, &format!("{PREFIX_SWAP}{}", s.rfq_id), s)?;
        self.swaps.insert(s.rfq_id.clone(), s.clone());
        Ok(())
    }

    fn log(&mut self, s: &mut MakerSwap, line: String) {
        self.ev.push(Event::Log { swap: s.rfq_id.clone(), line: line.clone() });
        s.log.push(line);
    }

    fn log_once(&mut self, s: &mut MakerSwap, line: String) {
        if s.log.last() != Some(&line) {
            self.log(s, line);
        }
    }

    fn set_stage(&mut self, s: &mut MakerSwap, stage: MakerStage) {
        s.stage = stage;
        self.ev.push(Event::Stage { swap: s.rfq_id.clone(), stage: s.stage.name().into() });
    }

    fn cover_ready(&mut self, s: &mut MakerSwap, what: &str, trigger: u64, deadline: Option<u64>, now: u64) -> bool {
        let key = format!("{}:{what}", s.rfq_id);
        if self.covers_passed.contains(&key) {
            return true;
        }
        let policy = self.cfg.cover.into();
        let c = self.covers.entry(key.clone()).or_insert_with(|| CoverWait::new(policy, trigger, deadline, now));
        match c.poll(&self.wallet, now) {
            Some(outcome) => {
                self.covers.remove(&key);
                self.covers_passed.insert(key);
                if self.cfg.cover.k > 0 || self.cfg.cover.max_secs > 0 {
                    self.log(s, format!("{what}: {outcome}"));
                }
                true
            }
            None => false,
        }
    }

    fn pend(&mut self, s: &mut MakerSwap, what: &str, tx: &wallet::BuiltTx, then: MakerStage) -> Next {
        s.pending = Some(PendingZ::new(what, tx));
        s.pending_then = Some(then);
        Next::At(0)
    }

    // ------------------------------------------------------------------ messages

    fn on_rfq(&mut self, epoch: u32, eph: [u8; 32], ct: &[u8], height: u64, now: u64, store: &mut dyn Store) -> Result<()> {
        if epoch != self.epoch {
            bail!("RFQ for epoch {epoch}, current epoch is {}", self.epoch);
        }
        let me_pub = msg::dh_public(&self.rfq_sk);
        let rfq = Rfq::from_bytes(&msg::open(b"rfq", &self.rfq_sk, &eph, &eph, &me_pub, ct)?)?;
        let rfq_hex = hex::encode(rfq.rfq_id);
        if self.swaps.contains_key(&rfq_hex) {
            return Ok(());
        }
        let c = self.cfg.clone();
        let refusal = if self.pending_quotes >= c.max_pending_quotes {
            Some((RefuseCode::Busy, format!("{} requests already wait for a quote or an acceptance (max_pending_quotes); request refused", self.pending_quotes)))
        } else if rfq.zats > c.max_zats {
            Some((RefuseCode::OverLimit, format!("amount {} outside {}..={}", rfq.zats, c.min_zats, c.max_zats)))
        } else if rfq.zats < c.min_zats {
            Some((RefuseCode::UnderMinimum, format!("amount {} outside {}..={}", rfq.zats, c.min_zats, c.max_zats)))
        } else {
            let a_ok = keys::point_from_bytes(&rfq.a_pub).map(|a| keys::pok_verify(&a, &rfq.pok_a, &rfq.rfq_id, b"taker")).unwrap_or(false);
            if !a_ok {
                Some((RefuseCode::Unsupported, "taker proof of knowledge invalid".to_string()))
            } else if rfq.taker_pk == [0u8; 32] || ed25519_dalek::VerifyingKey::from_bytes(&rfq.taker_pk).is_err() {
                Some((RefuseCode::Unsupported, "taker NEAR key is not a valid ed25519 key".to_string()))
            } else {
                None
            }
        };
        if let Some((code, why)) = refusal {
            let reply = self.queue_refusal(&rfq, code, &eph, now, store)?;
            bail!("{why}; {reply}");
        }
        let a_pub = keys::point_from_bytes(&rfq.a_pub)?;
        wallet::addr_from_raw(&rfq.reply_addr)?;
        let gg = keys::g();
        let (b, b_pub) = if c.misbehave == Some(Misbehave::RogueKey) {
            let b2 = keys::random_scalar();
            (b2, gg * b2 - a_pub)
        } else {
            let b = keys::random_scalar();
            (b, gg * b)
        };
        let pok_b = keys::pok_prove(&b, &rfq.rfq_id, b"maker");
        let s_m: [u8; 32] = crate::random_bytes();
        let e_m = keys::random_scalar();
        let e_m_pub = msg::dh_public(&e_m);
        let mut w = c.windows;
        match c.misbehave {
            Some(Misbehave::ShortReveal) => w.reveal_secs = (c.timing.finality_secs + c.timing.budget()) as u32,
            Some(Misbehave::FarHorizon) => w.t2_secs = u32::MAX,
            _ => {}
        }
        let mut q = Quote {
            price: c.price_for(rfq.zats),
            premium: c.premium_zats,
            stipend: c.stipend,
            b_pub: b_pub.to_bytes(),
            pok_b,
            s_m,
            t0_secs: w.t0_secs,
            t1_secs: w.t1_secs,
            t2_secs: w.t2_secs,
            reveal_secs: w.reveal_secs,
            escrow: c.chains.escrow.clone(),
            maker: c.near_account.clone(),
            sig: [0u8; 64],
        };
        q.sig = self.signer.key.sign(&q.sig_digest(&rfq.rfq_id));
        let body = q.to_bytes();
        let qct = msg::seal(b"quote", &e_m, &eph, &eph, &e_m_pub, &body)?;
        let env = Envelope::Quote { rfq_id: rfq.rfq_id, eph: e_m_pub, ct: qct };
        let taker_account = hex::encode(rfq.taker_pk);
        let mut s = MakerSwap {
            rfq_id: rfq_hex,
            stage: MakerStage::Quoting,
            zats: rfq.zats,
            price: q.price.to_string(),
            premium: q.premium,
            stipend: q.stipend.to_string(),
            taker_pk: hex::encode(rfq.taker_pk),
            taker_account,
            a_pub: hex::encode(rfq.a_pub),
            commit_s_t: hex::encode(rfq.commit_s_t),
            b: hex::encode(keys::scalar_to_bytes(&b)),
            s_m: hex::encode(s_m),
            e_m: hex::encode(keys::scalar_to_bytes(&e_m)),
            e_t_pub: hex::encode(eph),
            quote_memo: hex::encode(env.encode()),
            quote_hash: hex::encode(keys::h32(b"quote", &[&rfq.rfq_id, &body])),
            windows: w,
            rfq_height: height,
            accept_height: None,
            negated: false,
            joint_fvk: None,
            salt: None,
            escrow_id: None,
            t0: None,
            t1: None,
            t2: None,
            pending_then: None,
            pending: None,
            quote_txid: None,
            open_tx: None,
            stipend_tx: None,
            ready_tx: None,
            refund_tx: None,
            refund_at: None,
            sweep_txid: None,
            sweep_height: None,
            taker_withdraw_at: None,
            log: vec![],
            created_at: Some(now),
            withdraw_at: None,
            payout_due_at: None,
        };
        let tag = c.misbehave.map(|m| format!(" [MISBEHAVE {m:?}]")).unwrap_or_default();
        let line = format!("RFQ for {} zats decrypted and verified; quoting price {} (premium {} zats, stipend {} yocto){tag}", s.zats, s.price, s.premium, s.stipend);
        self.log(&mut s, line);
        self.ev.push(Event::Stage { swap: s.rfq_id.clone(), stage: s.stage.name().into() });
        self.pending_quotes += 1;
        self.save(&mut s, store)
    }

    /// Decides whether a refused request gets a refusal reply, and if so stores it (the
    /// transaction is built in a later step of the tick, from the leftover quote budget).
    /// Returns a phrase for the log. A reply costs the maker a Zcash fee and a proof, so:
    /// at most `max_refusals_per_hour` in any hour (sliding window, counted from the stored
    /// records, also across restarts), at most one per request id, and none when the reply
    /// address does not parse.
    fn queue_refusal(&mut self, rfq: &Rfq, code: RefuseCode, eph: &[u8; 32], now: u64, store: &mut dyn Store) -> Result<String> {
        let rfq_hex = hex::encode(rfq.rfq_id);
        if self.cfg.max_refusals_per_hour == 0 {
            return Ok("no reply (refusal replies are off)".into());
        }
        if self.refusals.contains_key(&rfq_hex) {
            return Ok("a reply is already sent or queued".into());
        }
        let hour = now.saturating_sub(3600);
        let recent = self.refusals.values().filter(|r| r.created_at > hour).count();
        if recent >= self.cfg.max_refusals_per_hour {
            return Ok(format!("no reply ({recent} refusal replies in the last hour, max_refusals_per_hour)"));
        }
        if wallet::addr_from_raw(&rfq.reply_addr).is_err() {
            return Ok("no reply (the reply address does not parse)".into());
        }
        let mut f = Refusal { code, min_zats: self.cfg.min_zats, max_zats: self.cfg.max_zats, sig: [0u8; 64] };
        f.sig = self.signer.key.sign(&f.sig_digest(&rfq.rfq_id));
        let me_pub = msg::dh_public(&self.rfq_sk);
        let ct = msg::seal(b"refuse", &self.rfq_sk, eph, eph, &me_pub, &f.to_bytes())?;
        let env = Envelope::Refuse { rfq_id: rfq.rfq_id, ct };
        let r = MakerRefusal {
            rfq_id: rfq_hex.clone(),
            code: code.name().into(),
            reply: hex::encode(rfq.reply_addr),
            memo: hex::encode(env.encode()),
            created_at: now,
            pending: None,
            done: false,
            txid: None,
        };
        put_json(store, &format!("{PREFIX_REFUSAL}{rfq_hex}"), &r)?;
        self.refusals.insert(rfq_hex, r);
        Ok(format!("a refusal reply ({}) is queued", code.name()))
    }

    /// Builds and broadcasts queued refusal replies. A reply is built only when the wallet
    /// is in sync, no request waits for a quote, and this tick's quote budget is not used
    /// (at most one proof per tick for quotes and refusals together). A reply not built
    /// within `quote_ttl_secs` is dropped. A built reply goes out on the next tick, like a
    /// quote; one that dies unbroadcast is dropped, not built again.
    fn advance_refusals(&mut self, now: u64, quotes_waiting: bool, store: &mut dyn Store, next: &mut u64) -> Result<()> {
        let ids: Vec<String> = self.refusals.iter().filter(|(_, r)| !r.done).map(|(k, _)| k.clone()).collect();
        for id in ids {
            let mut r = self.refusals[&id].clone();
            let before = r.clone();
            if let Some(mut p) = r.pending.clone() {
                if self.zcash_stale {
                    continue;
                }
                match drive_pending(&mut self.wallet, &mut p) {
                    Ok(PendingState::Waiting) if p.broadcast => {
                        r.done = true;
                        r.txid = Some(p.txid.clone());
                        r.pending = None;
                        self.ev.push(Event::ZcashTx { swap: id.clone(), what: "refusal".into(), txid: p.txid.clone() });
                        self.ev.push(Event::Log { swap: id.clone(), line: format!("REFUSAL ({}) sent to the taker in tx {}", r.code, p.txid) });
                    }
                    Ok(PendingState::Mined(_)) => {
                        r.done = true;
                        r.txid = Some(p.txid.clone());
                        r.pending = None;
                    }
                    Ok(PendingState::Waiting) => r.pending = Some(p),
                    Ok(PendingState::Dead(why)) => {
                        r.done = true;
                        r.pending = None;
                        self.ev.push(Event::Log { swap: id.clone(), line: format!("refusal reply {} {why}; dropped", short(&p.txid)) });
                    }
                    Err(e) => self.ev.push(Event::Warning { swap: id.clone(), message: format!("refusal reply: {e:#}") }),
                }
            } else if now > r.created_at.saturating_add(self.cfg.quote_ttl_secs) {
                r.done = true;
                self.ev.push(Event::Log { swap: id.clone(), line: "refusal reply not sent within quote_ttl_secs; dropped".into() });
            } else if !self.zcash_stale && !quotes_waiting && self.quote_budget > 0 {
                self.quote_budget -= 1;
                let memo: [u8; 512] = hex::decode(&r.memo)?.try_into().map_err(|_| anyhow!("memo length"))?;
                let raw: [u8; 43] = hex::decode(&r.reply)?.try_into().map_err(|_| anyhow!("reply address length"))?;
                let to = wallet::addr_from_raw(&raw)?;
                match self.wallet.build_pay("wallet", &self.me.ask, self.me.change(), vec![(to, 0, memo)]) {
                    Ok(tx) => {
                        r.pending = Some(PendingZ::new("refusal", &tx));
                        *next = now;
                    }
                    Err(e) => self.ev.push(Event::Warning { swap: id.clone(), message: format!("refusal reply not built: {e:#}") }),
                }
            }
            if r != before {
                put_json(store, &format!("{PREFIX_REFUSAL}{id}"), &r)?;
                self.refusals.insert(id, r);
            }
        }
        // Done replies older than an hour no longer count against the cap; their records
        // stay in the store.
        let hour = now.saturating_sub(3600);
        self.refusals.retain(|_, r| !r.done || r.created_at > hour);
        Ok(())
    }

    /// Ok(true) when the message is dealt with (accepted or rejected for good).
    fn on_accept(&mut self, rfq_id: [u8; 16], ct: &[u8], paid: u64, height: u64, now: u64, store: &mut dyn Store) -> Result<bool> {
        let rfq_hex = hex::encode(rfq_id);
        let Some(mut s) = self.swaps.get(&rfq_hex).cloned() else {
            // Not a request of ours (or one refused at the cap): nothing will change that,
            // so the memo is handled, not tried again every tick (F2-04).
            self.ev.push(Event::Log { swap: rfq_hex, line: "ACCEPT for an unknown request; ignored".into() });
            return Ok(true);
        };
        match s.stage {
            MakerStage::Quoting => return Ok(false), // our quote is not out yet: look again later
            MakerStage::Quoted => {}
            _ => return Ok(true),
        }
        let e_m = keys::scalar_from_hex(&s.e_m)?;
        let e_t_pub = keys::unhex32(&s.e_t_pub)?;
        let acc = msg::Accept::from_bytes(&msg::open(b"accept", &e_m, &e_t_pub, &e_t_pub, &msg::dh_public(&e_m), ct)?)?;
        if hex::encode(acc.quote_hash) != s.quote_hash {
            bail!("ACCEPT references a different quote");
        }
        if hex::encode(keys::h32(b"commit-sT", &[&acc.s_t])) != s.commit_s_t {
            self.log(&mut s, "ABORT: the taker's seed does not open its commitment".into());
            self.set_stage(&mut s, MakerStage::Aborted("bad seed reveal".into()));
            self.save(&mut s, store)?;
            return Ok(true);
        }
        if paid < s.premium {
            let line = format!("ABORT: ACCEPT carried {paid} zats, the premium is {}", s.premium);
            self.log(&mut s, line);
            let why = format!("premium {paid} < {}", s.premium);
            self.set_stage(&mut s, MakerStage::Aborted(why));
            self.save(&mut s, store)?;
            return Ok(true);
        }
        let a_pub = keys::point_from_hex(&s.a_pub)?;
        let b = keys::scalar_from_hex(&s.b)?;
        let s_m = keys::unhex32(&s.s_m)?;
        let joint = keys::joint(&rfq_id, a_pub, keys::g() * b, &acc.s_t, &s_m)?;
        let salt = keys::escrow_salt(&rfq_id, &acc.s_t, &s_m);
        s.negated = joint.negated;
        s.joint_fvk = Some(hex::encode(joint.fvk.to_bytes()));
        s.salt = Some(hex::encode(salt));
        s.escrow_id = Some(hex::encode(escrow::swap_id(&self.cfg.near_account, &salt)));
        s.accept_height = Some(height);
        self.wallet.watch(&rfq_hex, &joint.fvk, &[Scope::External]);
        let line = format!("ACCEPT decrypted (premium {paid} zats received, seed commitment opens, negated: {}); escrow id {}", joint.negated, short(s.escrow_id.as_deref().unwrap()));
        self.log(&mut s, line);
        self.set_stage(&mut s, MakerStage::Opening);
        let _ = now;
        self.save(&mut s, store)?;
        Ok(true)
    }

    // ------------------------------------------------------------------ swaps

    fn advance(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        // A quote that no node took within quote_send_secs comes too late for the taker,
        // whether it waits to be built or was built and refused (incident 2026-11-13: a
        // maker whose quotes were refused rebuilt them for over 30 minutes).
        let quote_unsent = s.stage == MakerStage::Quoting && s.pending.as_ref().is_none_or(|p| !p.broadcast);
        if quote_unsent {
            let created = *s.created_at.get_or_insert(now);
            if now > created.saturating_add(self.cfg.quote_send_secs) {
                let tried = if s.pending.is_some() { "; the built quote is dropped unsent" } else { "" };
                self.log(s, format!("no quote sent within {}s of the request; the taker no longer waits for it{tried}", self.cfg.quote_send_secs));
                s.pending = None;
                s.pending_then = None;
                self.set_stage(s, MakerStage::Aborted("quote not sent in time".into()));
                return Ok(Next::Done);
            }
        }
        if matches!(s.stage, MakerStage::Quoting | MakerStage::Quoted) && s.pending.is_none() {
            // F2-04: unpaid requests expire. A quote still in flight (pending) is not
            // expired here; it is mined or it dies at its expiry height.
            let created = *s.created_at.get_or_insert(now);
            if now > created.saturating_add(self.cfg.quote_ttl_secs) {
                self.log(s, format!("request expired after {}s without {}", self.cfg.quote_ttl_secs, if s.stage == MakerStage::Quoting { "a quote" } else { "an acceptance" }));
                self.set_stage(s, MakerStage::Aborted("request expired".into()));
                return Ok(Next::Done);
            }
        }
        if s.pending.is_some() && self.zcash_stale {
            // H2: no Zcash server. A pending sweep does not block the NEAR side of
            // Sweeping (paying the taker's credit); everything else waits.
            if s.stage != MakerStage::Sweeping {
                return Ok(Next::At(now + RETRY_SECS));
            }
        } else if let Some(mut p) = s.pending.clone() {
            let st = drive_pending(&mut self.wallet, &mut p)?;
            let first = (p.broadcast || matches!(st, PendingState::Mined(_))) && s.pending_then.is_some();
            s.pending = Some(p.clone());
            if first {
                self.ev.push(Event::ZcashTx { swap: s.rfq_id.clone(), what: p.what.clone(), txid: p.txid.clone() });
                let then = s.pending_then.take().unwrap();
                let line = match p.what.as_str() {
                    "quote" => {
                        s.quote_txid = Some(p.txid.clone());
                        format!("encrypted QUOTE sent in tx {}", p.txid)
                    }
                    "sweep" => format!("swept the joint note with (a+b) to the maker's wallet, tx {}; waiting for {} confirmations", p.txid, self.cfg.timing.k_z),
                    w => format!("{w} tx {}", p.txid),
                };
                self.log(s, line);
                self.set_stage(s, then);
            }
            match st {
                PendingState::Mined(h) => {
                    if p.what == "sweep" {
                        s.sweep_height = Some(h);
                    }
                    s.pending = None;
                }
                PendingState::Waiting => {}
                PendingState::Dead(why) => {
                    s.pending = None;
                    let never_sent = s.pending_then.take().is_some();
                    self.log(s, format!("{} transaction {} {why}{}", p.what, short(&p.txid), if never_sent { "; building it again" } else { "" }));
                    if p.what == "sweep" {
                        s.sweep_txid = None;
                    }
                    if p.what == "quote" && !never_sent {
                        self.set_stage(s, MakerStage::Aborted("quote expired unmined".into()));
                        return Ok(Next::Done);
                    }
                    return Ok(Next::At(now + RETRY_SECS));
                }
            }
            if s.pending.as_ref().map(|p| !p.broadcast).unwrap_or(false) {
                return Ok(Next::At(now + RETRY_SECS));
            }
        }
        match s.stage.clone() {
            MakerStage::Quoting => self.send_quote(s, now),
            MakerStage::Quoted => Ok(Next::At(now + POLL_SECS * 2)),
            MakerStage::Opening if self.near_trouble => Ok(Next::At(now + self.cfg.polling.fast_secs)),
            MakerStage::Opening => self.open_escrow(s, now),
            MakerStage::Opened | MakerStage::Ready | MakerStage::RefundSent | MakerStage::Sweeping => self.near_step(s, now),
            MakerStage::Swept | MakerStage::Refunded | MakerStage::Aborted(_) => Ok(Next::Done),
        }
    }

    fn send_quote(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        // F2-04: no build (and no RPC round trip) when the wallet cannot pay the fee or
        // this tick has built its share of quotes; the request waits, up to its TTL.
        if self.quote_budget == 0 {
            return Ok(Next::At(now + RETRY_SECS));
        }
        if !self.cover_ready(s, "QUOTE", s.rfq_height, None, now) {
            return Ok(Next::At(now + POLL_SECS));
        }
        self.quote_budget -= 1;
        let memo: [u8; 512] = hex::decode(&s.quote_memo)?.try_into().map_err(|_| anyhow!("memo length"))?;
        // The reply goes to the address in the request; recover it from the stored request.
        let reply = self.reply_address(s)?;
        let tx = self.wallet.build_pay("wallet", &self.me.ask, self.me.change(), vec![(reply, 0, memo)])?;
        Ok(self.pend(s, "quote", &tx, MakerStage::Quoted))
    }

    fn reply_address(&self, s: &MakerSwap) -> Result<orchard::Address> {
        // The request is in the wallet's received notes; decrypt it again (cheap).
        let me_pub = msg::dh_public(&self.rfq_sk);
        for r in self.wallet.received.iter().filter(|r| r.label == "wallet") {
            if let Ok(Some(Envelope::Rfq { eph, ct, .. })) = Envelope::decode(&r.memo) {
                if hex::encode(eph) != s.e_t_pub {
                    continue;
                }
                let rfq = Rfq::from_bytes(&msg::open(b"rfq", &self.rfq_sk, &eph, &eph, &me_pub, &ct)?)?;
                return wallet::addr_from_raw(&rfq.reply_addr);
            }
        }
        bail!("the request of swap {} is not in the wallet", short(&s.rfq_id))
    }

    fn open_escrow(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        let id = s.id_bytes()?;
        let salt = keys::unhex32(s.salt.as_deref().ok_or_else(|| anyhow!("no salt"))?)?;
        if let Some(v) = self.esc().get_swap(&id, &At::Optimistic)? {
            if s.t0.is_none() {
                // Deadlines never decided, so we never sent an open: not ours.
                self.log(s, format!("ABORT: a swap exists at id {} before this maker opened one; no stipend sent", short(&hex::encode(id))));
                self.set_stage(s, MakerStage::Aborted("swap id taken by a foreign swap".into()));
                return Ok(Next::Done);
            }
            // A swap at our id: ours from an earlier try (a restart), or a look-alike that
            // someone else put there. Only our own gets a stipend (security review 1, H1).
            if let Some(what) = self.not_ours(s, &v)? {
                self.log(s, format!("ABORT: the swap at id {} is not the one this maker opens ({what} differs); no stipend sent", short(&hex::encode(id))));
                self.set_stage(s, MakerStage::Aborted(format!("swap id taken by a foreign swap ({what})")));
                return Ok(Next::Done);
            }
            return self.send_stipend(s, now);
        }
        // `open` is public; a cover delay keeps it from pinning the Accept from behind.
        if !self.cover_ready(s, "OPEN", s.accept_height.unwrap_or(0), None, now) {
            return Ok(Next::At(now + POLL_SECS));
        }
        let esc = self.esc();
        if esc.storage_funds(&self.cfg.near_account, &At::Optimistic)? < self.cfg.storage_min {
            let (to, actions) = esc.add_storage_funds(self.cfg.storage_topup);
            let out = self.near.transact(&self.signer, &to, actions)?.ok().context("add_storage_funds")?;
            let line = format!("added {} yoctoNEAR of storage funds to the escrow, tx {}", self.cfg.storage_topup, out.hash);
            self.log(s, line);
        }
        let a_pub = keys::point_from_hex(&s.a_pub)?;
        let b = keys::scalar_from_hex(&s.b)?;
        let (mut ap, mut bp) = (a_pub, keys::g() * b);
        if s.negated {
            ap = -ap;
            bp = -bp;
        }
        // The deadlines are decided and stored first, so that after a restart the maker can
        // compare every field of a swap found at its id with what it meant to open (H1).
        let w = s.windows;
        let tnow = self.near.block(&At::Final)?.secs();
        let fresh = s.t0.map(|t0| t0 > tnow + w.t0_secs as u64 / 2).unwrap_or(false);
        if !fresh {
            let t0 = tnow + w.t0_secs as u64;
            let t1 = t0 + w.t1_secs as u64;
            s.t0 = Some(t0);
            s.t1 = Some(t1);
            s.t2 = Some(t1.saturating_add(w.t2_secs as u64));
            return Ok(Next::At(0));
        }
        let (t0, t1, t2) = (s.t0.unwrap(), s.t1.unwrap(), s.t2.unwrap());
        let mut amount: u128 = s.price.parse()?;
        if self.cfg.misbehave == Some(Misbehave::ShortOpen) {
            amount -= 1;
        }
        // The maker opens from its own Verifier account. If that holds less than the
        // amount, it deposits the difference from its plain token balance first. The
        // deposit's result is not trusted (a USDC pause can report failure after the
        // Verifier credited the maker): the balance is read again on the next try.
        let esc = self.esc();
        let in_verifier = esc.intents_balance(&self.cfg.near_account, &At::Optimistic)?;
        if in_verifier < amount {
            let short = amount - in_verifier;
            let plain = esc.ft_balance(&self.cfg.near_account, &At::Optimistic)?;
            if plain < short {
                bail!("the maker holds {in_verifier} of {} in the Verifier and {plain} outside it; the swap needs {amount}", self.cfg.chains.token);
            }
            let (to, actions) = esc.deposit(short);
            let out = self.near.transact(&self.signer, &to, actions)?;
            self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "deposit".into(), hash: out.hash.clone() });
            self.log(s, format!("deposited {short} {} into the maker's Verifier account before the open, tx {} ({})", self.cfg.chains.token, out.hash, out.failure().unwrap_or("executed")));
            return Ok(Next::At(0));
        }
        let taker_pk = keys::unhex32(&s.taker_pk)?;
        let (to, actions) = self.esc().open(amount, &salt, &taker_pk, &ap.to_bytes(), &bp.to_bytes(), t0, t1, t2, w.reveal_secs as u64);
        let out = self.near.transact(&self.signer, &to, actions)?;
        if let Some(f) = out.failure() {
            s.t0 = None; // decide new deadlines on the next try
            bail!("open failed: {f}");
        }
        let v = self.esc().get_swap(&id, &At::Optimistic)?.ok_or_else(|| anyhow!("open executed but the escrow has no swap {}", hex::encode(id)))?;
        if let Some(what) = self.not_ours(s, &v)? {
            self.log(s, format!("ABORT: after open, the swap at id {} is not ours ({what} differs); no stipend sent", short(&hex::encode(id))));
            self.set_stage(s, MakerStage::Aborted(format!("swap id taken by a foreign swap ({what})")));
            return Ok(Next::Done);
        }
        s.open_tx = Some(out.hash.clone());
        self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "open".into(), hash: out.hash.clone() });
        self.log(s, format!("escrow OPEN id {} amount {amount} t0={t0} t1={t1} t2={t2} W={}, tx {}", hex::encode(id), w.reveal_secs, out.hash));
        self.send_stipend(s, now)
    }

    /// Compares a swap in the escrow with what this maker opens for `s`. `Some(field)`
    /// names the first field that differs.
    fn not_ours(&self, s: &MakerSwap, v: &SwapView) -> Result<Option<&'static str>> {
        let a_pub = keys::point_from_hex(&s.a_pub)?;
        let b_pub = keys::g() * keys::scalar_from_hex(&s.b)?;
        let (ap, bp) = if s.negated { (-a_pub, -b_pub) } else { (a_pub, b_pub) };
        let mut price: u128 = s.price.parse()?;
        if self.cfg.misbehave == Some(Misbehave::ShortOpen) {
            price -= 1;
        }
        let checks = [
            ("maker", v.maker == self.cfg.near_account),
            ("token", v.token == self.cfg.chains.token),
            ("amount", v.amount == price),
            ("taker key", v.taker_pk == s.taker_pk),
            ("A", v.a_pub == hex::encode(ap.to_bytes())),
            ("B", v.b_pub == hex::encode(bp.to_bytes())),
            ("reveal window", v.reveal_window == s.windows.reveal_secs as u64),
            ("t0", s.t0 == Some(v.t0)),
            ("t1", s.t1 == Some(v.t1)),
            ("t2", s.t2 == Some(v.t2)),
        ];
        Ok(checks.iter().find(|(_, ok)| !ok).map(|(what, _)| *what))
    }

    fn send_stipend(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        let stipend: u128 = s.stipend.parse()?;
        if self.cfg.misbehave == Some(Misbehave::NoStipend) {
            self.log(s, "[MISBEHAVE no-stipend] not sending the stipend".into());
        } else if s.stipend_tx.is_none() {
            let have = self.near.account(&s.taker_account, &At::Optimistic)?.map(|a| a.amount).unwrap_or(0);
            if have < stipend {
                let out = self.near.transact(&self.signer, &s.taker_account, vec![Action::Transfer { deposit: stipend }])?.ok().context("stipend transfer")?;
                self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "stipend".into(), hash: out.hash.clone() });
                self.log(s, format!("STIPEND of {stipend} yoctoNEAR sent to {}, tx {}", s.taker_account, out.hash));
                s.stipend_tx = Some(out.hash);
            } else {
                s.stipend_tx = Some("already funded".into());
            }
        }
        self.set_stage(s, MakerStage::Opened);
        Ok(Next::At(now + POLL_SECS))
    }

    /// Rules R3e, R3d and R3l decide a refund; this sends it. The decision is stored first
    /// (stage RefundSent, the record is saved at the end of the tick) and the refund goes
    /// out on the next tick, so b is never public while the store says otherwise.
    fn refund(&mut self, s: &mut MakerSwap, tnow: u64, why: &str) -> Result<Next> {
        if s.stage != MakerStage::RefundSent {
            s.refund_tx = None;
            s.refund_at = None;
            self.log(s, format!("{why}; refunding"));
            self.set_stage(s, MakerStage::RefundSent);
            return Ok(Next::At(0));
        }
        let again = s.refund_tx.is_some();
        if again && s.refund_at.map(|t| tnow < t + self.cfg.timing.inclusion_secs).unwrap_or(false) {
            // The previous refund may still be on its way: read again later, not at once
            // (incident 2026-11-14: "at once" made a tight loop of reads).
            return Ok(Next::At(self.wall_now + self.cfg.polling.settle_secs));
        }
        let id = s.id_bytes()?;
        let b = keys::scalar_to_bytes(&s.b_norm()?);
        let (to, actions) = self.esc().refund(&id, &b);
        let tx = self.near.sign(&self.signer, &to, actions)?;
        s.refund_tx = Some(tx.hash_str());
        s.refund_at = Some(tnow);
        let out = self.near.wait_executed(&tx, &self.signer.account_id)?;
        self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "refund".into(), hash: out.hash.clone() });
        let verb = if again { "refund not on chain; RE-SENT REFUND" } else { "REFUND" };
        self.log(s, format!("{verb} publishes b, tx {} ({}); waiting for it to be final", out.hash, out.failure().unwrap_or("executed")));
        Ok(Next::At(0))
    }

    fn watch_escrow(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        let id = s.id_bytes()?;
        let fin = self.near.block(&At::Final)?;
        let tnow = fin.secs();
        let at = At::Block(fin.hash.clone());
        let v = self.esc().get_swap(&id, &at)?.ok_or_else(|| anyhow!("escrow swap {} not found at a final block", short(&hex::encode(id))))?;
        let t = self.cfg.timing.clone();
        self.note_seen(s, &v, tnow, now);
        // H6: say so loudly when a swap of ours is near t1 or t2 and not settled. In
        // [t1, t2) the maker must refund an Open or Ready swap, or from t2 the taker can
        // claim through the punish path. Each kind of alert is repeated at most once per
        // `polling.quiet_secs` (incident 2026-11-14: once a second for hours).
        if matches!(v.state, SwapState::Open | SwapState::Ready) {
            let near_t2 = tnow + 2 * t.budget() >= v.t2;
            let near_t1 = tnow + 2 * t.budget() >= v.t1;
            if tnow >= v.t2 {
                let every = self.cfg.polling.stuck_secs.min(v.reveal_window / 3).max(self.cfg.polling.fast_secs);
                let m = format!(
                    "past t2 {} with the escrow {:?}: the escrow allows no refund now. Only the taker's key can act (commit_claim); after a lock the maker refunds when the reveal window ends, or sweeps after a claim. The maker reads this escrow every {every}s",
                    v.t2, v.state
                );
                self.alert(s, "past_t2", m, now, 3 * self.cfg.polling.quiet_secs);
            } else if near_t2 {
                self.alert(s, "near_t2", format!("t2 {} is near (NEAR time {tnow}) and the escrow is {:?}: refund now", v.t2, v.state), now, self.cfg.polling.quiet_secs);
            } else if near_t1 && s.stage != MakerStage::Ready {
                self.alert(s, "near_t1", format!("t1 {} is near (NEAR time {tnow}) and the escrow is {:?}", v.t1, v.state), now, self.cfg.polling.quiet_secs);
            }
        }
        match v.state {
            SwapState::Claimed => return self.on_claimed(s, &v, now),
            SwapState::Refunded => return self.on_refunded(s, &fin, now),
            SwapState::Locked => {
                // R3l: refund a lock only if a final block at or after lockedAt + W shows Locked.
                let deadline = v.locked_at.unwrap_or(0).saturating_add(v.reveal_window);
                if tnow >= deadline {
                    return self.refund(s, tnow, &format!("the taker locked but did not reveal by {deadline} (final block {})", fin.height));
                }
                return Ok(Next::At(now + POLL_SECS));
            }
            SwapState::Open | SwapState::Ready if tnow >= v.t1 => {
                // R3d: a final block with timestamp >= t1 shows Open or Ready, and the refund
                // can be final before t2.
                if s.stage == MakerStage::RefundSent && s.refund_tx.is_some() && tnow < v.t2 {
                    return self.refund(s, tnow, "deadline refund");
                }
                if !t.in_time(tnow, v.t2) {
                    let m = format!("L_M violated: back online at {tnow}, too close to t2 {} to refund safely; the taker may now claim (the trade completes if she funded)", v.t2);
                    self.quiet_log(s, "lm", m, now, 3 * self.cfg.polling.quiet_secs);
                    return Ok(Next::At(now + POLL_SECS));
                }
                return self.refund(s, tnow, &format!("final block {} (t={tnow}) is past t1 with state {:?}", fin.height, v.state));
            }
            _ => {}
        }
        if v.state == SwapState::Open && s.stage == MakerStage::RefundSent && tnow + self.cfg.timing.inclusion_secs < v.t0 {
            return self.refund(s, tnow, "early refund");
        }
        if v.state != SwapState::Open || s.stage != MakerStage::Opened {
            return Ok(Next::At(now + POLL_SECS));
        }
        // Open, maker in Opened: Ready once the joint note is there, or an early refund.
        let need = s.zats + wallet::zip317_fee(1, 1);
        let tip = self.wallet.height;
        let note = self.wallet.unspent(&s.rfq_id).into_iter().next();
        let enough = note.as_ref().map(|n| n.note.value().inner() >= need).unwrap_or(false);
        let confirmed = note.as_ref().map(|n| tip + 1 - n.height >= t.k_z).unwrap_or(false);
        if enough && confirmed && tnow + t.inclusion_secs < v.t0 {
            if self.cfg.misbehave == Some(Misbehave::NeverReady) {
                self.log_once(s, "[MISBEHAVE never-ready] joint note confirmed; not setting Ready".into());
                return Ok(Next::At(now + POLL_SECS));
            }
            // set_ready is public and would otherwise follow the funding by exactly k_Z
            // blocks; delay it too, but never past t0 − 2D.
            let n = note.unwrap();
            let deadline = now + v.t0.saturating_sub(tnow + 2 * t.inclusion_secs);
            if !self.cover_ready(s, "READY", n.height, Some(deadline), now) {
                return Ok(Next::At(now + POLL_SECS));
            }
            let tnow = self.near.block(&At::Final)?.secs();
            if tnow + t.inclusion_secs >= v.t0 {
                self.log(s, "cover delay reached t0 before Ready; not setting Ready (the swap will refund after t1)".into());
                return Ok(Next::At(now + POLL_SECS));
            }
            if let Some(what) = self.not_ours(s, &v)? {
                bail!("not setting Ready: the swap at our id differs in {what}");
            }
            let (to, actions) = self.esc().set_ready(&id);
            let out = self.near.transact(&self.signer, &to, actions)?;
            if let Some(f) = out.failure() {
                bail!("set_ready failed: {f}");
            }
            s.ready_tx = Some(out.hash.clone());
            self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "set_ready".into(), hash: out.hash.clone() });
            self.log(s, format!("joint note {} zats with {} confirmations (k_Z = {}); READY, tx {}", n.note.value().inner(), tip + 1 - n.height, t.k_z, out.hash));
            self.set_stage(s, MakerStage::Ready);
            return Ok(Next::At(now + POLL_SECS));
        }
        // R3e: refund early only in Open with no sufficient note, once the taker's R1b
        // funding window has closed, and while the refund can still land before t0.
        let window_closed = tnow + t.fund_secs >= v.t0;
        if self.zcash_stale {
            // H2: the funding may be there and not seen; R3d after t1 needs no Zcash data.
            return Ok(Next::At(now + POLL_SECS));
        }
        if window_closed && tnow + t.inclusion_secs < v.t0 && !enough {
            let why = match &note {
                Some(n) => format!("joint note underfunded ({} < {need}) before t0", n.note.value().inner()),
                None => "no funding before t0".into(),
            };
            return self.refund(s, tnow, &why);
        }
        Ok(Next::At(now + POLL_SECS))
    }

    /// Emits an alert of this kind for the swap, at most once per `every` seconds, and
    /// keeps it for `/health`.
    fn alert(&mut self, s: &mut MakerSwap, kind: &str, message: String, now: u64, every: u64) {
        let e = self.alerts.entry(s.rfq_id.clone()).or_insert_with(|| ActiveAlert { kind: kind.into(), message: message.clone(), since: now, last: now });
        if e.kind != kind {
            *e = ActiveAlert { kind: kind.into(), message: message.clone(), since: now, last: now };
        }
        e.message = message.clone();
        e.last = now;
        let mut held = 0;
        if self.quiet.pass(&format!("alert:{}:{kind}", s.rfq_id), now, every, &mut held) {
            let m = if held > 0 { format!("{message} (repeated {held} times since the last report)") } else { message };
            self.ev.push(Event::Alert { swap: s.rfq_id.clone(), message: m.clone() });
            self.log(s, format!("ALERT: {m}"));
        }
    }

    /// A log line of this kind, at most once per `every` seconds.
    fn quiet_log(&mut self, s: &mut MakerSwap, kind: &str, line: String, now: u64, every: u64) {
        let mut held = 0;
        if self.quiet.pass(&format!("log:{}:{kind}", s.rfq_id), now, every, &mut held) {
            self.log(s, line);
        }
    }

    fn note_seen(&mut self, s: &MakerSwap, v: &SwapView, tnow: u64, now: u64) {
        let p = self.polls.entry(s.rfq_id.clone()).or_default();
        p.seen = Some(Seen {
            state: v.state,
            tnow,
            at: now,
            t0: v.t0,
            t1: v.t1,
            t2: v.t2,
            reveal_end: (v.state == SwapState::Locked).then(|| v.locked_at.unwrap_or(0).saturating_add(v.reveal_window)),
        });
    }

    /// True when the confirmed joint note is in the wallet (the maker may set Ready).
    fn funded_locally(&self, s: &MakerSwap) -> bool {
        let need = s.zats + wallet::zip317_fee(1, 1);
        let tip = self.wallet.height;
        self.wallet.unspent(&s.rfq_id).into_iter().next().map(|n| n.note.value().inner() >= need && tip + 1 - n.height >= self.cfg.timing.k_z).unwrap_or(false)
    }

    /// The deadlines that matter for the swap in the state last seen (NEAR time).
    fn points(&self, s: &MakerSwap, seen: &Seen) -> Vec<(&'static str, u64)> {
        let t = &self.cfg.timing;
        match seen.state {
            SwapState::Open if s.stage == MakerStage::Opened => vec![
                ("funding window ends (R3e refund may start)", seen.t0.saturating_sub(t.fund_secs)),
                ("last moment for an early refund or set_ready (t0 - D)", seen.t0.saturating_sub(t.inclusion_secs)),
                ("t0", seen.t0),
                ("t1 (R3d refund window opens)", seen.t1),
                ("t2 - B_R (last safe refund)", seen.t2.saturating_sub(t.budget())),
                ("t2", seen.t2),
            ],
            SwapState::Open | SwapState::Ready => vec![
                ("t0 - D", seen.t0.saturating_sub(t.inclusion_secs)),
                ("t1 (R3d refund window opens)", seen.t1),
                ("t2 - B_R (last safe refund)", seen.t2.saturating_sub(t.budget())),
                ("t2", seen.t2),
            ],
            SwapState::Locked => seen.reveal_end.map(|e| vec![("reveal window ends (R3l refund)", e)]).unwrap_or_default(),
            _ => vec![],
        }
    }

    /// The next deadline of the swap after now: (what, wall-clock time).
    pub fn next_deadline_of(&self, s: &MakerSwap, now: u64) -> Option<(&'static str, u64)> {
        let seen = self.polls.get(&s.rfq_id)?.seen?;
        let tn = seen.est(now);
        self.points(s, &seen).into_iter().filter(|(_, p)| *p > tn).min_by_key(|(_, p)| *p).map(|(w, p)| (w, now + (p - tn)))
    }

    fn next_deadline(&self, id: &str, now: u64) -> Option<(&'static str, u64)> {
        self.swaps.get(id).and_then(|s| self.next_deadline_of(s, now))
    }

    /// True when the swap's next NEAR step protects funds: a refund that is decided or
    /// whose window is open or near, set_ready of a funded swap, or a swap not read yet
    /// since the start.
    fn is_critical(&self, s: &MakerSwap, now: u64) -> bool {
        if !matches!(s.stage, MakerStage::Opened | MakerStage::Ready | MakerStage::RefundSent) {
            return false;
        }
        if s.stage == MakerStage::RefundSent {
            return true;
        }
        let Some(p) = self.polls.get(&s.rfq_id) else { return true };
        let Some(seen) = p.seen else { return true };
        if p.funded_seen && s.stage == MakerStage::Opened {
            return true;
        }
        let tn = seen.est(now);
        let soon = 60 + 2 * self.cfg.polling.fast_secs;
        let t = &self.cfg.timing;
        match seen.state {
            SwapState::Open | SwapState::Ready if tn >= seen.t2 => false,
            SwapState::Open if s.stage == MakerStage::Opened && tn + soon >= seen.t0.saturating_sub(t.fund_secs) && tn < seen.t0 => true,
            SwapState::Open | SwapState::Ready => tn + soon >= seen.t1,
            SwapState::Locked => seen.reveal_end.map(|e| tn + soon >= e).unwrap_or(true),
            _ => false,
        }
    }

    /// Seconds until the next read of the swap (see `Polling`).
    fn poll_secs(&self, s: &MakerSwap, now: u64) -> u64 {
        let pol = self.cfg.polling;
        let Some(p) = self.polls.get(&s.rfq_id) else { return pol.fast_secs };
        let Some(seen) = p.seen else { return pol.fast_secs };
        let tn = seen.est(now);
        let slow = match (seen.state, &s.stage) {
            (SwapState::Open | SwapState::Ready, _) if tn >= seen.t2 => {
                // Stuck: only the taker can act. Read often enough to see a lock well
                // within its reveal window.
                pol.stuck_secs.min(self.swap_reveal(s) / 3).max(pol.fast_secs)
            }
            // A refund that could not land before t0 can only go again from t1.
            (SwapState::Open, MakerStage::RefundSent) if tn + self.cfg.timing.inclusion_secs >= seen.t0 && tn < seen.t1 => pol.open_secs,
            (_, MakerStage::RefundSent) => pol.settle_secs,
            (SwapState::Open, MakerStage::Opened) if p.funded_seen => pol.fast_secs,
            (SwapState::Open, MakerStage::Opened) => pol.open_secs,
            _ => pol.ready_secs,
        };
        let gap = self.points(s, &seen).into_iter().map(|(_, p)| p).filter(|p| *p > tn).map(|p| p - tn).min();
        match gap {
            Some(g) => (g + 1).clamp(pol.fast_secs.min(slow), slow),
            None => slow,
        }
    }

    fn swap_reveal(&self, s: &MakerSwap) -> u64 {
        s.windows.reveal_secs as u64
    }

    /// One NEAR step of a swap in `Opened`, `Ready`, `RefundSent` or `Sweeping`, if one is
    /// due: reads at the interval the deadlines call for (`Polling`), deadline-critical
    /// steps in the client's critical mode, a back-off after a failure, and an alert when a
    /// critical step has failed for `polling.alert_after_secs`.
    fn near_step(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        let id = s.rfq_id.clone();
        let pol = self.cfg.polling;
        let p = self.polls.get(&id).cloned().unwrap_or_default();
        let wake = s.stage == MakerStage::Opened && !p.funded_seen && !self.zcash_stale && self.funded_locally(s);
        if now < p.due && !wake {
            return Ok(Next::At(p.due));
        }
        let critical = self.is_critical(s, now) || wake;
        if self.near_trouble && !critical {
            // A deadline-critical step failed in this tick: leave the endpoints to it.
            return Ok(Next::At(now + pol.fast_secs));
        }
        let prev = self.near.set_critical(critical);
        let r = if s.stage == MakerStage::Sweeping { self.on_sweeping(s, now) } else { self.watch_escrow(s, now) };
        self.near.set_critical(prev);
        if wake {
            self.polls.entry(id.clone()).or_default().funded_seen = true;
        }
        match r {
            Ok(Next::Done) => {
                self.polls.remove(&id);
                self.alerts.remove(&id);
                Ok(Next::Done)
            }
            Ok(Next::At(t)) => {
                let iv = self.poll_secs(s, now);
                let p = self.polls.entry(id.clone()).or_default();
                if p.critical_fail_since.take().is_some() {
                    self.quiet.reset(&format!("alert:{id}:near_failing"));
                }
                p.fails = 0;
                // A step of several ticks (a refund decided, then sent) goes on at once;
                // otherwise the next read waits for the plan.
                p.due = if t <= now { now } else { t.max(now + iv) };
                if self.alerts.get(&id).map(|a| a.kind == "near_failing").unwrap_or(false) {
                    self.alerts.remove(&id);
                }
                Ok(Next::At(if t <= now { t } else { p.due }))
            }
            Err(e) => {
                let p = self.polls.entry(id.clone()).or_default();
                p.fails = p.fails.saturating_add(1);
                let mut wait = if critical { pol.fast_secs } else { (pol.fast_secs << (p.fails - 1).min(5)).min(pol.open_secs) };
                if let (false, Some(u)) = (critical, unavailable(&e)) {
                    let until = u.retry_at_ms.div_ceil(1000);
                    wait = wait.max(until.saturating_sub(now)).min(pol.open_secs);
                }
                p.due = now + wait.max(1);
                let due = p.due;
                self.ev.push(Event::Warning { swap: id.clone(), message: format!("{e:#}") });
                if critical {
                    self.near_trouble = true;
                }
                // Any NEAR step of an open swap that keeps failing is reported; a
                // deadline-critical one is named so.
                let since = *p.critical_fail_since.get_or_insert(now);
                if now >= since + pol.alert_after_secs {
                    let next = self.next_deadline_of(s, now).map(|(w, at)| format!("; next deadline: {w} in {}s", at.saturating_sub(now))).unwrap_or_default();
                    let what = if critical { "a deadline-critical NEAR step" } else { "the NEAR reads" };
                    let m = format!("{what} ({}) has failed for {}s{next}: {e:#}", s.stage.name(), now - since);
                    self.alert(s, "near_failing", m, now, pol.quiet_secs.min(300));
                }
                Ok(Next::At(due))
            }
        }
    }

    /// The maker's state for an operator (`GET /health`): when NEAR and Zcash last
    /// answered, the endpoints, open swaps by stage, the nearest deadline and the active
    /// alerts. No keys, no addresses, no amounts; swaps are named by the first 8 hex
    /// digits of their escrow id, which is public on NEAR.
    pub fn health(&self, now: u64) -> serde_json::Value {
        let mut stages: BTreeMap<&str, usize> = BTreeMap::new();
        let mut nearest: Option<(u64, &'static str, String)> = None;
        for s in self.swaps.values().filter(|s| !s.stage.is_terminal()) {
            *stages.entry(s.stage.name()).or_insert(0) += 1;
            if let Some((what, at)) = self.next_deadline_of(s, now)
                && nearest.as_ref().map(|(t, _, _)| at < *t).unwrap_or(true)
            {
                nearest = Some((at, what, s.escrow_id.as_deref().map(short).unwrap_or_default()));
            }
        }
        let name = |id: &str| self.swaps.get(id).and_then(|s| s.escrow_id.as_deref()).map(short).unwrap_or_default();
        let alerts: Vec<serde_json::Value> = self
            .alerts
            .iter()
            .map(|(id, a)| serde_json::json!({"escrow": name(id), "kind": a.kind, "message": a.message, "since": a.since, "last": a.last}))
            .collect();
        let endpoints: Vec<serde_json::Value> = self
            .near
            .health()
            .into_iter()
            .map(|e| serde_json::json!({
                "url": crate::near::rpc::public_url(&e.url),
                "fails_in_a_row": e.fails,
                "backoff_until": e.backoff_until_ms / 1000,
                "last_ok": e.last_ok_ms.map(|t| t / 1000),
                "last_error": e.last_error.map(|m| m.chars().take(200).collect::<String>()),
                "requests": e.requests,
                "rate_limited": e.rate_limited,
            }))
            .collect();
        // A swap past t2 with the escrow Open needs no action that the maker could take;
        // it is listed but does not make the maker unhealthy.
        let ok = self.alerts.values().all(|a| a.kind == "past_t2");
        serde_json::json!({
            "now": now,
            "ok": ok,
            "near": {"last_ok": self.near.last_ok_ms().map(|t| t / 1000), "requests": self.near.requests(), "endpoints": endpoints},
            "zcash": {"last_ok": self.last_zcash_ok, "height": self.wallet.height},
            "open_swaps": stages,
            "nearest_deadline": nearest.map(|(at, what, escrow)| serde_json::json!({"escrow": escrow, "what": what, "at": at, "in_secs": at.saturating_sub(now)})),
            "alerts": alerts,
        })
    }

    fn on_refunded(&mut self, s: &mut MakerSwap, fin: &crate::near::rpc::BlockInfo, now: u64) -> Result<Next> {
        // The refund is final (read at a final block). Its payout (mt_transfer to the
        // maker's Verifier account) normally ran in the refund call. If it is Due (too
        // little gas, or the Verifier refused it), send withdraw(id) and stay here until a
        // final block shows it Paid.
        let id = s.id_bytes()?;
        let v = self.esc().get_swap(&id, &At::Block(fin.hash.clone()))?.ok_or_else(|| anyhow!("escrow swap vanished"))?;
        if v.payout != escrow::Payout::Paid {
            let t = &self.cfg.timing;
            let due = s.withdraw_at.map(|w| fin.secs() >= w + t.inclusion_secs + t.finality_secs).unwrap_or(true);
            if v.payout == escrow::Payout::Due && due {
                let (to, actions) = self.esc().withdraw(&id);
                s.withdraw_at = Some(fin.secs());
                let out = self.near.transact(&self.signer, &to, actions)?;
                self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "withdraw".into(), hash: out.hash.clone() });
                self.log(s, format!("refund is final (block <= {}); withdraw(id) of the refund into the maker's Verifier account, tx {} ({})", fin.height, out.hash, out.failure().unwrap_or("executed")));
            }
            return Ok(Next::At(now + POLL_SECS));
        }
        self.log(s, format!("refund is final (block <= {}); {} paid back into the maker's Verifier account", fin.height, v.amount));
        if s.stage != MakerStage::RefundSent {
            // Someone else sent our refund (b was already public): fine, same result.
            self.log(s, "the escrow was refunded".into());
        }
        self.set_stage(s, MakerStage::Refunded);
        Ok(Next::Done)
    }

    fn on_claimed(&mut self, s: &mut MakerSwap, v: &SwapView, now: u64) -> Result<Next> {
        let a = keys::scalar_from_hex(v.a.as_deref().ok_or_else(|| anyhow!("Claimed without a"))?)?;
        let a_pub = keys::point_from_hex(&s.a_pub)?;
        let a_pub = if s.negated { -a_pub } else { a_pub };
        if keys::g() * a != a_pub {
            bail!("published a does not match A");
        }
        let b = s.b_norm()?;
        self.pay_taker(s)?;
        let Some(n) = self.wallet.unspent(&s.rfq_id).into_iter().next() else {
            if self.zcash_stale {
                // H2: the note may be there and not seen; never erase b on an old view.
                return Ok(Next::At(now + RETRY_SECS));
            }
            let tnow = self.near.block(&At::Final)?.secs();
            if v.t2 <= tnow && s.ready_tx.is_none() && s.pending.is_none() {
                self.set_stage(s, MakerStage::Aborted("claimed via the punish path without funding: L_M was violated".into()));
                self.log(s, "CLAIMED via the punish path, but no joint note exists: the maker was offline for all of [t1, t2) (assumption L_M violated) and lost the escrowed tokens".into());
                return Ok(Next::Done);
            }
            self.log_once(s, "CLAIMED; the joint note is not visible yet".into());
            return Ok(Next::At(now + POLL_SECS));
        };
        // The sweep has no deadline, so it waits for cover to decorrelate it from the claim.
        let tip = self.wallet.height;
        if !self.cover_ready(s, "SWEEP", tip, None, now) {
            return Ok(Next::At(now + POLL_SECS));
        }
        self.log(s, "CLAIMED: read a from the escrow".into());
        let tx = self.wallet.build_sweep(n, a + b, self.me.address())?;
        s.sweep_txid = Some(tx.txid.clone());
        s.sweep_height = None;
        Ok(self.pend(s, "sweep", &tx, MakerStage::Sweeping))
    }

    /// Backs up the taker's payout. The taker's key sends `withdraw(id)` itself from the
    /// stipend; if a final block still shows the payout Due `2 × (D + F)` after the maker
    /// first saw it Due, the maker sends the same call (anyone may; the tokens can only go
    /// to the taker). Returns true once the payout is Paid.
    fn pay_taker(&mut self, s: &mut MakerSwap) -> Result<bool> {
        let id = s.id_bytes()?;
        let fin = self.near.block(&At::Final)?;
        let v = self.esc().get_swap(&id, &At::Block(fin.hash.clone()))?.ok_or_else(|| anyhow!("escrow swap vanished"))?;
        if v.payout == escrow::Payout::Paid {
            return Ok(true);
        }
        if v.payout != escrow::Payout::Due {
            return Ok(false);
        }
        let t = &self.cfg.timing;
        let grace = 2 * (t.inclusion_secs + t.finality_secs);
        let since = *s.payout_due_at.get_or_insert(fin.secs());
        let retry = s.taker_withdraw_at.map(|w| fin.secs() >= w + t.inclusion_secs + t.finality_secs).unwrap_or(true);
        if fin.secs() >= since + grace && retry {
            let (to, actions) = self.esc().withdraw(&id);
            s.taker_withdraw_at = Some(fin.secs());
            let out = self.near.transact(&self.signer, &to, actions)?;
            self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "withdraw".into(), hash: out.hash.clone() });
            self.log(s, format!("the taker's payout is still Due after {grace}s; withdraw(id) for the taker, tx {} ({})", out.hash, out.failure().unwrap_or("executed")));
        }
        Ok(false)
    }

    fn on_sweeping(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        let paid = self.pay_taker(s)?;
        let k_z = self.cfg.timing.k_z;
        let tip = self.wallet.height;
        if let Some(h) = s.sweep_height {
            if tip + 1 - h >= k_z && paid {
                self.log(s, format!("sweep {} has {} confirmations (k_Z = {k_z}); done", s.sweep_txid.clone().unwrap_or_default(), tip + 1 - h));
                self.set_stage(s, MakerStage::Swept);
                return Ok(Next::Done);
            }
        } else if s.pending.is_none() {
            // The sweep expired unmined: sweep again.
            let id = s.id_bytes()?;
            let v = self.esc().get_swap(&id, &At::Final)?.ok_or_else(|| anyhow!("escrow vanished"))?;
            return self.on_claimed(s, &v, now);
        }
        Ok(Next::At(now + POLL_SECS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Chains, Cover, Timing, Windows},
        net::{Response, Transport},
        store::MemStore,
    };
    use std::rc::Rc;

    struct NoNet;
    impl Transport for NoNet {
        fn post(&self, _: &str, _: &[(&str, &str)], _: &[u8]) -> Result<Response> {
            Err(anyhow!("no network in this test"))
        }
    }

    fn maker() -> Maker {
        let cfg = MakerConfig {
            chains: Chains {
                zebra_rpc: Some("http://unused".into()),
                lightwalletd: None,
                birthday: None,
                network: None,
                nu7_height: Some(2),
                near_rpc: "http://unused".into(),
                escrow: "escrow.near".into(),
                token: "usdc.near".into(),
                intents: "intents.near".into(),
            },
            timing: Timing { inclusion_secs: 3, finality_secs: 3, reorg_budget: 2, margin_secs: 0, fund_secs: 30, k_z: 2 },
            near_account: "maker.near".into(),
            price_per_zec: 5_000_000,
            min_zats: 1,
            max_zats: 10u64.pow(9),
            premium_zats: 1,
            stipend: 1,
            windows: Windows { t0_secs: 120, t1_secs: 60, t2_secs: 60, reveal_secs: 45 },
            storage_min: 0,
            storage_topup: 0,
            cover: Cover::default(),
            misbehave: None,
            max_pending_quotes: 32,
            quote_ttl_secs: 3600,
            quote_send_secs: 300,
            max_quotes_per_tick: 1,
            premium_check: None,
            max_refusals_per_hour: 6,
            polling: Default::default(),
        };
        let mut store = MemStore::new();
        store.put(KEY_NEAR, NearKey::generate().secret_str().as_bytes()).unwrap();
        Maker::open(cfg, Rc::new(NoNet), &mut store).unwrap()
    }

    #[test]
    fn retry_path_refuses_a_look_alike_swap() {
        let m = maker();
        let (a, b) = (keys::random_scalar(), keys::random_scalar());
        let taker_pk = hex::encode([5u8; 32]);
        let mut s: MakerSwap = serde_json::from_value(serde_json::json!({
            "rfq_id": "00", "stage": "Opening", "zats": 1, "price": "5000000", "premium": 1, "stipend": "1",
            "taker_pk": taker_pk, "taker_account": taker_pk, "a_pub": hex::encode((keys::g() * a).to_bytes()),
            "commit_s_t": "", "b": hex::encode(keys::scalar_to_bytes(&b)), "s_m": "", "e_m": "", "e_t_pub": "",
            "quote_memo": "", "quote_hash": "", "windows": {"t0_secs": 120, "t1_secs": 60, "t2_secs": 60, "reveal_secs": 45},
            "rfq_height": 0, "accept_height": null, "negated": false, "joint_fvk": null, "salt": null, "escrow_id": null,
            "t0": 1, "t1": 2, "t2": 3, "pending_then": null, "pending": null, "quote_txid": null, "open_tx": null,
            "stipend_tx": null, "ready_tx": null, "refund_tx": null, "refund_at": null, "sweep_txid": null,
            "sweep_height": null, "taker_withdraw_at": null, "log": []
        }))
        .unwrap();
        let ours = |s: &MakerSwap| -> SwapView {
            serde_json::from_value(serde_json::json!({
                "id": "00", "maker": "maker.near", "token": "usdc.near", "amount": "5000000",
                "taker_pk": s.taker_pk, "taker_account": s.taker_account, "a_pub": s.a_pub,
                "b_pub": hex::encode((keys::g() * keys::scalar_from_hex(&s.b).unwrap()).to_bytes()),
                "t0": 1, "t1": 2, "t2": 3, "reveal_window": 45, "state": "Open",
                "locked_at": null, "commitment": null, "a": null, "b": null,
                "payout": "none", "recipient": null
            }))
            .unwrap()
        };
        assert_eq!(m.not_ours(&s, &ours(&s)).unwrap(), None);
        let mut v = ours(&s);
        v.token = "fake-token.near".into();
        assert_eq!(m.not_ours(&s, &v).unwrap(), Some("token"));
        let mut v = ours(&s);
        v.amount = 1;
        assert_eq!(m.not_ours(&s, &v).unwrap(), Some("amount"));
        let mut v = ours(&s);
        v.taker_pk = hex::encode([6u8; 32]);
        assert_eq!(m.not_ours(&s, &v).unwrap(), Some("taker key"));
        let mut v = ours(&s);
        v.t2 = 4;
        assert_eq!(m.not_ours(&s, &v).unwrap(), Some("t2"));
        let mut v = ours(&s);
        v.b_pub = s.a_pub.clone();
        assert_eq!(m.not_ours(&s, &v).unwrap(), Some("B"));
        // with negated shares the escrow holds -A and -B
        s.negated = true;
        assert_eq!(m.not_ours(&s, &ours(&s)).unwrap(), Some("A"));
    }

    // ------------------------------------------------------------------ NEAR budget
    // A fake NEAR chain behind two endpoints, a clock the test moves, and no Zcash
    // server (the wallet's sync fails, as on a bad day; R3d and R3l need no Zcash data).

    use std::cell::{Cell, RefCell};

    const EP_A: &str = "https://a.example";
    const EP_B: &str = "https://b.example";

    struct Chain {
        /// Wall clock (ms).
        clock: Cell<u64>,
        /// The swap as get_swap returns it.
        swap: RefCell<serde_json::Value>,
        /// The swap after the next send_tx (a refund).
        after_send: RefCell<Option<serde_json::Value>>,
        /// Endpoints that answer fastnear's -429.
        limited: RefCell<Vec<&'static str>>,
        /// (url, method) of every request.
        log: RefCell<Vec<(String, String)>>,
        /// (url, wall secs) of every send_tx that got through.
        sends: RefCell<Vec<(String, u64)>>,
    }

    impl Chain {
        fn now(&self) -> u64 {
            self.clock.get() / 1000
        }
        fn requests(&self) -> usize {
            self.log.borrow().len()
        }
        fn swap_state(&self, st: &str) {
            self.swap.borrow_mut()["state"] = serde_json::json!(st);
        }
    }

    impl Transport for Chain {
        fn post(&self, url: &str, _: &[(&str, &str)], body: &[u8]) -> Result<Response> {
            if url != EP_A && url != EP_B {
                return Err(anyhow!("no Zcash server in this test"));
            }
            let req: serde_json::Value = serde_json::from_slice(body).unwrap();
            let method = req["method"].as_str().unwrap().to_string();
            self.log.borrow_mut().push((url.to_string(), method.clone()));
            let ok = |v: serde_json::Value| Ok(Response { status: 200, body: serde_json::json!({"jsonrpc": "2.0", "id": "zecret", "result": v}).to_string().into_bytes(), headers: vec![] });
            if self.limited.borrow().contains(&url) {
                let e = serde_json::json!({"jsonrpc": "2.0", "id": "zecret", "error": {"code": -429, "message": "Rate limits exceeded"}});
                return Ok(Response { status: 200, body: e.to_string().into_bytes(), headers: vec![] });
            }
            let p = &req["params"];
            match method.as_str() {
                "block" => {
                    // Final blocks lag the wall clock by 2 s.
                    let secs = self.now() - 2;
                    ok(serde_json::json!({"header": {"height": secs, "hash": bs58::encode([7u8; 32]).into_string(), "timestamp": secs * 1_000_000_000}}))
                }
                "query" => match p["request_type"].as_str().unwrap() {
                    "call_function" => {
                        let bytes: Vec<u8> = self.swap.borrow().to_string().into_bytes();
                        ok(serde_json::json!({"result": bytes, "logs": []}))
                    }
                    "view_access_key" => ok(serde_json::json!({"nonce": 5, "permission": "FullAccess"})),
                    other => panic!("unexpected query {other}"),
                },
                "send_tx" | "tx" => {
                    if method == "send_tx" {
                        self.sends.borrow_mut().push((url.to_string(), self.now()));
                        if let Some(v) = self.after_send.borrow_mut().take() {
                            *self.swap.borrow_mut() = v;
                        }
                    }
                    ok(serde_json::json!({"final_execution_status": "FINAL", "status": {"SuccessValue": ""}, "transaction_outcome": {"outcome": {"logs": [], "status": {"SuccessReceiptId": "x"}, "executor_id": "maker.near"}}, "receipts_outcome": []}))
                }
                other => panic!("unexpected method {other}"),
            }
        }
        fn now_ms(&self) -> Option<u64> {
            Some(self.clock.get())
        }
    }

    const T0_SECS: u64 = 900;
    const T1_SECS: u64 = 600;
    const T2_SECS: u64 = 900;
    const W_SECS: u64 = 600;

    /// A maker with one swap in `stage`, opened at wall time `opened` (t0 = opened + 900,
    /// t1 = t0 + 600, t2 = t1 + 900, W = 600, as the testnet maker quotes).
    fn budget_maker(chain: &Rc<Chain>, stage: &str, opened: u64, polling: Option<crate::config::Polling>) -> (Maker, MemStore, String) {
        let (t0, t1) = (opened + T0_SECS, opened + T0_SECS + T1_SECS);
        let t2 = t1 + T2_SECS;
        let b = keys::random_scalar();
        let a = keys::random_scalar();
        let escrow_id = hex::encode([0x5eu8; 32]);
        let taker_pk = hex::encode([5u8; 32]);
        let rec = serde_json::json!({
            "rfq_id": "c2b9fa5067b8d2aded520570a2d1fe18", "stage": stage, "zats": 959000, "price": "287700", "premium": 1000, "stipend": "1",
            "taker_pk": taker_pk, "taker_account": taker_pk, "a_pub": hex::encode((keys::g() * a).to_bytes()),
            "commit_s_t": "", "b": hex::encode(keys::scalar_to_bytes(&b)), "s_m": "", "e_m": "", "e_t_pub": "",
            "quote_memo": "", "quote_hash": "", "windows": {"t0_secs": T0_SECS, "t1_secs": T1_SECS, "t2_secs": T2_SECS, "reveal_secs": W_SECS},
            "rfq_height": 0, "accept_height": 1, "negated": false, "joint_fvk": null, "salt": hex::encode([1u8; 32]), "escrow_id": escrow_id,
            "t0": t0, "t1": t1, "t2": t2, "pending_then": null, "pending": null, "quote_txid": null, "open_tx": "x",
            "stipend_tx": "x", "ready_tx": null, "refund_tx": if stage == "RefundSent" { serde_json::json!("old") } else { serde_json::Value::Null },
            "refund_at": if stage == "RefundSent" { serde_json::json!(t0 - 40) } else { serde_json::Value::Null },
            "sweep_txid": null, "sweep_height": null, "taker_withdraw_at": null, "log": [], "created_at": opened
        });
        *chain.swap.borrow_mut() = serde_json::json!({
            "id": escrow_id, "maker": "maker.near", "token": "usdc.near", "amount": "287700",
            "taker_pk": taker_pk, "taker_account": taker_pk, "a_pub": rec["a_pub"], "b_pub": hex::encode((keys::g() * b).to_bytes()),
            "t0": t0, "t1": t1, "t2": t2, "reveal_window": W_SECS, "state": if stage == "Ready" { "Ready" } else { "Open" },
            "locked_at": null, "commitment": null, "a": null, "b": null, "payout": "none", "recipient": null
        });
        let mut cfg = maker().cfg;
        cfg.chains.near_rpc = crate::config::RpcUrls::new(vec![EP_A.into(), EP_B.into()]).unwrap();
        cfg.timing = Timing { inclusion_secs: 30, finality_secs: 10, reorg_budget: 2, margin_secs: 0, fund_secs: 600, k_z: 3 };
        cfg.windows = Windows { t0_secs: T0_SECS as u32, t1_secs: T1_SECS as u32, t2_secs: T2_SECS as u32, reveal_secs: W_SECS as u32 };
        if let Some(p) = polling {
            cfg.polling = p;
        }
        let mut store = MemStore::new();
        store.put(KEY_NEAR, NearKey::generate().secret_str().as_bytes()).unwrap();
        put_json(&mut store, &format!("{PREFIX_SWAP}{}", rec["rfq_id"].as_str().unwrap()), &rec).unwrap();
        let m = Maker::open(cfg, chain.clone(), &mut store).unwrap();
        (m, store, rec["rfq_id"].as_str().unwrap().to_string())
    }

    fn chain_at(wall: u64) -> Rc<Chain> {
        Rc::new(Chain { clock: Cell::new(wall * 1000), swap: RefCell::new(serde_json::Value::Null), after_send: RefCell::new(None), limited: RefCell::new(vec![]), log: RefCell::new(vec![]), sends: RefCell::new(vec![]) })
    }

    /// Runs the maker's loop as `zecret maker` does (tick, then sleep until next, at most
    /// 3 s) from the chain's clock until `until`. Returns the requests sent in each tick
    /// and every event.
    fn run(m: &mut Maker, store: &mut MemStore, chain: &Chain, until: u64) -> (Vec<(u64, usize)>, Vec<Event>) {
        let mut per_tick = vec![];
        let mut events = vec![];
        while chain.now() < until {
            let now = chain.now();
            let before = chain.requests();
            let t = m.tick(now, store);
            per_tick.push((now, chain.requests() - before));
            events.extend(t.events);
            let wait = match t.next {
                Next::At(x) if x > now => (x - now).min(3),
                // "At once": the real loop goes on without a pause; the test moves 0.2 s.
                _ => 0,
            };
            chain.clock.set(chain.clock.get() + if wait == 0 { 200 } else { wait * 1000 });
        }
        (per_tick, events)
    }

    fn max_per_window(per_tick: &[(u64, usize)], secs: u64) -> usize {
        per_tick.iter().map(|(t0, _)| per_tick.iter().filter(|(t, _)| *t >= *t0 && *t < t0 + secs).map(|(_, n)| n).sum()).max().unwrap_or(0)
    }

    #[test]
    fn budget_opened_far_from_deadlines() {
        // Incident 2026-11-14: about 2 requests a second for one Opened swap.
        let start = 1_794_638_343;
        let chain = chain_at(start + 10);
        let (mut m, mut store, _) = budget_maker(&chain, "Opened", start, None);
        let (per_tick, _) = run(&mut m, &mut store, &chain, start + 10 + 300);
        let total: usize = per_tick.iter().map(|(_, n)| n).sum();
        assert!(per_tick.iter().all(|(_, n)| *n <= 2), "at most one final block and one get_swap per tick: {per_tick:?}");
        assert!(max_per_window(&per_tick, 60) <= 6, "at most 6 requests a minute, got {}", max_per_window(&per_tick, 60));
        assert!(total <= 24, "{total} requests in 5 minutes");
    }

    #[test]
    fn budget_ready_waiting_for_the_taker() {
        let start = 1_794_638_343;
        let chain = chain_at(start + 100);
        let (mut m, mut store, _) = budget_maker(&chain, "Ready", start, None);
        let (per_tick, _) = run(&mut m, &mut store, &chain, start + 100 + 300);
        assert!(per_tick.iter().all(|(_, n)| *n <= 2));
        assert!(max_per_window(&per_tick, 60) <= 14, "{}", max_per_window(&per_tick, 60));
    }

    #[test]
    fn opened_reads_just_after_the_funding_window_closes() {
        // The no-funding refund (R3e) may start at t0 - fund_secs: the maker reads then,
        // not up to 30 s later. Here the Zcash server is down (H2: no R3e on an old
        // view), so the test checks the read times.
        let start = 1_794_638_343;
        let chain = chain_at(start + 5);
        let (mut m, mut store, _) = budget_maker(&chain, "Opened", start, None);
        let window = start + T0_SECS - 600; // t0 - fund_secs
        let (per_tick, _) = run(&mut m, &mut store, &chain, window + 30);
        let reads: Vec<u64> = per_tick.iter().filter(|(_, n)| *n > 0).map(|(t, _)| *t).collect();
        assert!(reads.iter().any(|t| *t >= window && *t <= window + 2 + 3), "reads at {reads:?}, window opens at {window}");
    }

    #[test]
    fn budget_and_alerts_past_t2_with_the_escrow_open() {
        // The state of swap c2b9fa50 since 07:18:51: RefundSent, the escrow Open, past t2.
        let start = 1_794_638_343;
        let t2 = start + T0_SECS + T1_SECS + T2_SECS;
        let chain = chain_at(t2 + 60);
        let (mut m, mut store, id) = budget_maker(&chain, "RefundSent", start, None);
        let (per_tick, events) = run(&mut m, &mut store, &chain, t2 + 60 + 1800);
        let total: usize = per_tick.iter().map(|(_, n)| n).sum();
        // Every W/3 = 200 s at most: 2 requests each, 9 reads in 30 minutes plus the first.
        assert!(total <= 2 * 10, "{total} requests in 30 minutes past t2");
        let alerts: Vec<&Event> = events.iter().filter(|e| matches!(e, Event::Alert { .. })).collect();
        assert_eq!(alerts.len(), 1, "one alert, repeated only every 30 minutes: {alerts:?}");
        assert!(chain.sends.borrow().is_empty(), "no refund after t2: the escrow refuses it");
        // The Zcash server of this test is down: its warning comes once a minute. NEAR
        // gives no warning at all.
        let warnings: Vec<&String> = events.iter().filter_map(|e| match e { Event::Warning { message, .. } => Some(message), _ => None }).collect();
        assert!(warnings.iter().all(|w| w.starts_with("Zcash sync")), "{warnings:?}");
        assert!(warnings.len() <= 31, "{} warnings in 30 minutes", warnings.len());
        // The record does not grow every tick.
        let s = &m.swaps[&id];
        assert!(s.log.len() <= 3, "{:?}", s.log);
        // Health shows the stuck swap and its alert.
        let h = m.health(chain.now());
        assert_eq!(h["open_swaps"]["RefundSent"], 1);
        assert_eq!(h["alerts"][0]["kind"], "past_t2");
        assert_eq!(h["ok"], true, "a stuck swap is reported, but nothing can be done: the maker is healthy");
    }

    #[test]
    fn stuck_swap_refunds_once_the_taker_locks_and_the_reveal_window_ends() {
        // C: past t2 the maker can act again only after the taker's key locks (punish
        // path). If she does not reveal, R3l refunds at lockedAt + W.
        let start = 1_794_638_343;
        let t2 = start + T0_SECS + T1_SECS + T2_SECS;
        let chain = chain_at(t2 + 60);
        let (mut m, mut store, id) = budget_maker(&chain, "RefundSent", start, None);
        run(&mut m, &mut store, &chain, t2 + 100);
        let locked_at = chain.now() - 2;
        chain.swap_state("Locked");
        chain.swap.borrow_mut()["locked_at"] = serde_json::json!(locked_at);
        let mut refunded = chain.swap.borrow().clone();
        refunded["state"] = serde_json::json!("Refunded");
        refunded["payout"] = serde_json::json!("paid");
        *chain.after_send.borrow_mut() = Some(refunded);
        run(&mut m, &mut store, &chain, locked_at + W_SECS + 60);
        let sends = chain.sends.borrow().clone();
        assert_eq!(sends.len(), 1, "{sends:?}");
        assert!(sends[0].1 >= locked_at + W_SECS && sends[0].1 <= locked_at + W_SECS + 20, "refund at {} for W end {}", sends[0].1, locked_at + W_SECS);
        assert_eq!(m.swaps[&id].stage, MakerStage::Refunded);
    }

    fn r3d_maker(chain: &Rc<Chain>) -> (Maker, MemStore, String, u64, u64) {
        // NEAR time just before t1, the escrow Open, the maker in Opened (R3e missed).
        let t1_wall = 1_794_639_831;
        let start = t1_wall - T0_SECS - T1_SECS;
        chain.clock.set((t1_wall - 20) * 1000);
        let (m, store, id) = budget_maker(chain, "Opened", start, None);
        let mut refunded = chain.swap.borrow().clone();
        refunded["state"] = serde_json::json!("Refunded");
        refunded["payout"] = serde_json::json!("paid");
        *chain.after_send.borrow_mut() = Some(refunded);
        (m, store, id, t1_wall, t1_wall + T2_SECS)
    }

    #[test]
    fn refund_goes_out_through_the_second_endpoint_while_the_first_is_rate_limited() {
        let chain = chain_at(0);
        let (mut m, mut store, id, t1, t2) = r3d_maker(&chain);
        chain.limited.borrow_mut().push(EP_A);
        let (per_tick, _) = run(&mut m, &mut store, &chain, t1 + 60);
        let sends = chain.sends.borrow().clone();
        assert_eq!(sends.len(), 1, "{sends:?}");
        assert_eq!(sends[0].0, EP_B);
        assert!(sends[0].1 <= t1 + 2 + 3 + 3, "refund sent at {} (t1 {t1})", sends[0].1);
        assert!(sends[0].1 + 15 < t2);
        assert_eq!(m.swaps[&id].stage, MakerStage::Refunded);
        // The rate-limited endpoint was not hammered: its back-off holds.
        let to_a = chain.log.borrow().iter().filter(|(u, _)| u == EP_A).count();
        assert!(to_a <= 6, "{to_a} requests to the rate-limited endpoint");
        assert!(per_tick.iter().all(|(_, n)| *n <= 7), "{per_tick:?}");
    }

    #[test]
    fn with_both_endpoints_down_the_refund_goes_out_as_soon_as_one_returns() {
        let chain = chain_at(0);
        let (mut m, mut store, id, t1, t2) = r3d_maker(&chain);
        // One read before t1, then both endpoints down until t1 + 90.
        run(&mut m, &mut store, &chain, t1 - 15);
        chain.limited.borrow_mut().extend([EP_A, EP_B]);
        let (per_tick, events) = run(&mut m, &mut store, &chain, t1 + 90);
        assert!(chain.sends.borrow().is_empty());
        // No tight loop: at most one attempt (two endpoints) every fast_secs = 3 s.
        assert!(max_per_window(&per_tick, 60) <= 2 * 21, "{}", max_per_window(&per_tick, 60));
        let alert = events.iter().find_map(|e| match e {
            Event::Alert { message, .. } if message.contains("deadline-critical") => Some(message.clone()),
            _ => None,
        });
        let alert = alert.expect("an alert after 60 s of failing critical work");
        assert!(alert.contains("t2 - B_R"), "{alert}");
        assert_eq!(m.health(chain.now())["alerts"][0]["kind"], "near_failing");
        assert_eq!(m.health(chain.now())["ok"], false);
        // B comes back.
        let back = chain.now();
        chain.limited.borrow_mut().retain(|u| *u != EP_B);
        run(&mut m, &mut store, &chain, back + 30);
        let sends = chain.sends.borrow().clone();
        assert_eq!(sends.len(), 1, "{sends:?}");
        assert_eq!(sends[0].0, EP_B);
        assert!(sends[0].1 <= back + 3 + 3, "refund at {} after the endpoint came back at {back}", sends[0].1);
        assert!(sends[0].1 + 15 < t2);
        assert_eq!(m.swaps[&id].stage, MakerStage::Refunded);
        assert!(m.health(chain.now())["alerts"].as_array().unwrap().is_empty());
    }
}
