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
//! Opening  --open (ft_transfer_call) + stipend----------> Opened
//! Opened   --joint note >= amount, k_Z conf, before t0--> Ready
//! Opened/Ready --claim(a) seen: sweep with a+b----------> Sweeping --k_Z conf--> Swept
//! Opened   --no note by the end of R1b (R3e)------------> RefundSent
//! Opened/Ready --final block >= t1, now + B_R < t2 (R3d)> RefundSent
//! Locked   --final block >= lockedAt + W (R3l)----------> RefundSent
//! RefundSent --refund final; withdraw-------------------> Refunded
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
    msg::{self, Envelope, Quote, Rfq},
    near::{escrow, tx::Action, At, Escrow, NearKey, NearRpc, Signer, SwapState, SwapView},
    net::SharedTransport,
    store::{get_json, put_json, Store},
    wallet::{self, CoverWait, Received, Wallet},
    Me,
};

pub const POLL_SECS: u64 = 1;
pub const RETRY_SECS: u64 = 5;

pub const KEY_WALLET: &str = "wallet";
pub const KEY_NEAR: &str = "near_key";
pub const KEY_RFQ: &str = "rfq_key";
pub const KEY_SEEN: &str = "seen";
pub const PREFIX_SWAP: &str = "swap/";

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
    pub taker_withdraw_at: Option<u64>,
    pub log: Vec<String>,
    /// Wall-clock time the request was taken (F2-04: `Quoting` and `Quoted` expire after
    /// `quote_ttl_secs`). Records from before this field get the time of their next tick.
    #[serde(default)]
    pub created_at: Option<u64>,
}

impl MakerSwap {
    fn id_bytes(&self) -> Result<[u8; 32]> {
        keys::unhex32(self.escrow_id.as_deref().ok_or_else(|| anyhow!("no escrow id"))?)
    }
    fn b_norm(&self) -> Result<pasta_curves::pallas::Scalar> {
        Ok(keys::normalize(keys::scalar_from_hex(&self.b)?, self.negated))
    }
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
        let near = NearRpc::new(t, &cfg.chains.near_rpc);
        let seen = get_json(store, KEY_SEEN)?.unwrap_or_default();
        let mut swaps = BTreeMap::new();
        for k in store.keys(PREFIX_SWAP)? {
            let s: MakerSwap = get_json(store, &k)?.ok_or_else(|| anyhow!("record {k} vanished"))?;
            swaps.insert(s.rfq_id.clone(), s);
        }
        let signer = Signer::new(&cfg.near_account, key);
        let mut m = Maker { epoch: rk.epoch, rfq_sk: keys::scalar_from_hex(&rk.sk)?, cfg, me, wallet, near, signer, seen, swaps, covers: HashMap::new(), covers_passed: BTreeSet::new(), ev: vec![], zcash_stale: false, quote_budget: 0, pending_quotes: 0, quote_funds_low: false };
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

    /// The maker's signed directory entry, for takers.
    pub fn directory_entry(&self) -> MakerEntry {
        let rfq_pub = msg::dh_public(&self.rfq_sk);
        let ua = self.ua();
        let digest = keys::directory_digest(&ua, self.epoch, &rfq_pub, &self.cfg.near_account);
        MakerEntry {
            ua,
            epoch: self.epoch,
            rfq_pub: hex::encode(rfq_pub),
            near_account: self.cfg.near_account.clone(),
            near_pk: self.signer.key.public_key_str(),
            sig: hex::encode(self.signer.key.sign(&digest)),
        }
    }

    fn esc(&self) -> Escrow<'_> {
        Escrow { rpc: &self.near, escrow: self.cfg.chains.escrow.clone(), token: self.cfg.chains.token.clone(), intents: self.cfg.chains.intents.clone() }
    }

    pub fn tick(&mut self, now: u64, store: &mut dyn Store) -> Tick {
        let next = match self.step(now, store) {
            Ok(n) => n,
            Err(e) => {
                self.ev.push(Event::Warning { swap: String::new(), message: format!("{e:#}") });
                Next::At(now + RETRY_SECS)
            }
        };
        Tick { next, events: std::mem::take(&mut self.ev) }
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
            Ok(()) => false,
            Err(e) => {
                self.ev.push(Event::Warning { swap: String::new(), message: format!("Zcash sync: {e:#}; NEAR duties go on with the last synced view") });
                true
            }
        };
        let quoting = |s: &MakerSwap| matches!(s.stage, MakerStage::Quoting | MakerStage::Quoted);
        let mut next = now + POLL_SECS * 2;
        // Deadline work first.
        let ids: Vec<String> = self.swaps.iter().filter(|(_, s)| !s.stage.is_terminal() && !quoting(s)).map(|(k, _)| k.clone()).collect();
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
        let waiting = ids.iter().any(|id| self.swaps[id].stage == MakerStage::Quoting);
        if waiting && !can_pay && !self.zcash_stale && !self.quote_funds_low {
            let have = self.wallet.spendable("wallet");
            self.ev.push(Event::Warning { swap: String::new(), message: format!("requests wait for a quote, but the wallet has {have} spendable zats, less than a quote fee ({fee})") });
        }
        if !self.zcash_stale {
            self.quote_funds_low = waiting && !can_pay;
        }
        self.advance_all(ids, now, store, &mut next)?;
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
        if self.pending_quotes >= self.cfg.max_pending_quotes {
            bail!("{} requests already wait for a quote or an acceptance (max_pending_quotes); request refused", self.pending_quotes);
        }
        let me_pub = msg::dh_public(&self.rfq_sk);
        let rfq = Rfq::from_bytes(&msg::open(b"rfq", &self.rfq_sk, &eph, &eph, &me_pub, ct)?)?;
        let rfq_hex = hex::encode(rfq.rfq_id);
        if self.swaps.contains_key(&rfq_hex) {
            return Ok(());
        }
        let c = self.cfg.clone();
        if rfq.zats < c.min_zats || rfq.zats > c.max_zats {
            bail!("amount {} outside {}..={}", rfq.zats, c.min_zats, c.max_zats);
        }
        let a_pub = keys::point_from_bytes(&rfq.a_pub)?;
        if !keys::pok_verify(&a_pub, &rfq.pok_a, &rfq.rfq_id, b"taker") {
            bail!("taker proof of knowledge invalid");
        }
        if rfq.taker_pk == [0u8; 32] || ed25519_dalek::VerifyingKey::from_bytes(&rfq.taker_pk).is_err() {
            bail!("taker NEAR key is not a valid ed25519 key");
        }
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
        };
        let tag = c.misbehave.map(|m| format!(" [MISBEHAVE {m:?}]")).unwrap_or_default();
        let line = format!("RFQ for {} zats decrypted and verified; quoting price {} (premium {} zats, stipend {} yocto){tag}", s.zats, s.price, s.premium, s.stipend);
        self.log(&mut s, line);
        self.ev.push(Event::Stage { swap: s.rfq_id.clone(), stage: s.stage.name().into() });
        self.pending_quotes += 1;
        self.save(&mut s, store)
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
            MakerStage::Opening => self.open_escrow(s, now),
            MakerStage::Opened | MakerStage::Ready | MakerStage::RefundSent => self.watch_escrow(s, now),
            MakerStage::Sweeping => self.on_sweeping(s, now),
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
            return Ok(Next::At(0)); // the previous refund may still be on its way
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
        // H6: say so loudly when a swap of ours is near t1 or t2 and not settled. In
        // [t1, t2) the maker must refund an Open or Ready swap, or from t2 the taker can
        // claim through the punish path.
        if matches!(v.state, SwapState::Open | SwapState::Ready) {
            let near_t2 = tnow + 2 * t.budget() >= v.t2;
            let near_t1 = tnow + 2 * t.budget() >= v.t1;
            let msg = if tnow >= v.t2 {
                Some(format!("past t2 {} with the escrow {:?}: only the taker can settle it now", v.t2, v.state))
            } else if near_t2 {
                Some(format!("t2 {} is near (NEAR time {tnow}) and the escrow is {:?}: refund now", v.t2, v.state))
            } else if near_t1 && s.stage != MakerStage::Ready {
                Some(format!("t1 {} is near (NEAR time {tnow}) and the escrow is {:?}", v.t1, v.state))
            } else {
                None
            };
            if let Some(m) = msg {
                let line = format!("ALERT: {m}");
                if s.log.last() != Some(&line) {
                    self.ev.push(Event::Alert { swap: s.rfq_id.clone(), message: m });
                    self.log(s, line);
                }
            }
        }
        match v.state {
            SwapState::Claimed => return self.on_claimed(s, &v, now),
            SwapState::Refunded => return self.on_refunded(s, &fin, now),
            SwapState::Locked => {
                // R3l: refund a lock only if a final block at or after lockedAt + W shows Locked.
                let deadline = v.locked_at.unwrap_or(0) + v.reveal_window;
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
                    self.log_once(s, format!("L_M violated: back online at {tnow}, too close to t2 {} to refund safely; the taker may now claim (the trade completes if she funded)", v.t2));
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

    fn on_refunded(&mut self, s: &mut MakerSwap, fin: &crate::near::rpc::BlockInfo, now: u64) -> Result<Next> {
        // The refund is final (read at a final block). Withdraw the maker's credit.
        let credit = self.esc().credit(&self.cfg.near_account, &At::Block(fin.hash.clone()))?;
        if credit.direct > 0 {
            let (to, actions) = self.esc().withdraw(&self.cfg.near_account);
            let out = self.near.transact(&self.signer, &to, actions)?.ok().context("withdraw")?;
            self.log(s, format!("refund is final (block <= {}); tokens withdrawn, tx {}", fin.height, out.hash));
        } else {
            self.log(s, format!("refund is final (block <= {}); nothing left to withdraw", fin.height));
        }
        if s.stage != MakerStage::RefundSent {
            // Someone else sent our refund (b was already public): fine, same result.
            self.log(s, "the escrow was refunded".into());
        }
        self.set_stage(s, MakerStage::Refunded);
        let _ = now;
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

    /// Pays the taker's credit into intents.near if nobody has yet. Anyone may call
    /// `withdraw`; the taker's stipend does not cover its gas, so the maker does it.
    /// Returns the credit still waiting.
    fn pay_taker(&mut self, s: &mut MakerSwap) -> Result<u128> {
        let fin = self.near.block(&At::Final)?;
        let credit = self.esc().credit(&s.taker_account, &At::Block(fin.hash.clone()))?;
        let due = s.taker_withdraw_at.map(|t| fin.secs() >= t + self.cfg.timing.inclusion_secs + self.cfg.timing.finality_secs).unwrap_or(true);
        if credit.intents > 0 && due {
            let (to, actions) = self.esc().withdraw(&s.taker_account);
            s.taker_withdraw_at = Some(fin.secs());
            let out = self.near.transact(&self.signer, &to, actions)?;
            self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "withdraw".into(), hash: out.hash.clone() });
            self.log(s, format!("withdraw for the taker into intents.near, tx {} ({})", out.hash, out.failure().unwrap_or("executed")));
        }
        Ok(credit.intents)
    }

    fn on_sweeping(&mut self, s: &mut MakerSwap, now: u64) -> Result<Next> {
        let waiting = self.pay_taker(s)?;
        let k_z = self.cfg.timing.k_z;
        let tip = self.wallet.height;
        if let Some(h) = s.sweep_height {
            if tip + 1 - h >= k_z && waiting == 0 {
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
            max_quotes_per_tick: 1,
            premium_check: None,
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
                "locked_at": null, "commitment": null, "a": null, "b": null
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
}
