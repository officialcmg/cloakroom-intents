//! The taker: sells shielded ZEC for a token on NEAR, through one maker.
//!
//! The same code runs natively (`zecret taker`, tests) and in a browser Web Worker. It
//! never blocks on time: the caller calls [`Taker::tick`] with the current Unix time and
//! calls again when the returned [`Tick`] says. Network calls inside a tick are blocking
//! (see `net::Transport`); one shielded send takes about 25 s in a browser.
//!
//! Stages (`TakerStage`), with the wallet rules of the Cloakroom paper (Table 2):
//!
//! ```text
//! WaitingDeposit --RFQ memo broadcast------------------> RfqSent
//! RfqSent        --quote passes R1q; Accept + premium---> Accepted
//! Accepted       --open final, = quote (R1), stipend,
//!                  R1b; funding broadcast--------------> Funded
//! Funded         --Ready final, now + B_R < t1 (R2c),
//!                  or Open/Ready from t2; commit_claim--> Committed
//! Committed      --lock final, now + B_R < lockedAt + W
//!                  (R2r); claim(a)----------------------> Revealed
//! Revealed       --claim(a) signed and stored; R2r checked
//!                  again at the broadcast; claim final-> Claimed
//!                  (R2r fails before anything was sent --> Committed)
//! Claimed        --payout Due: the taker sends withdraw(id);
//!                  payout Paid (final block)------------> Completed
//! Claimed        --not Paid after payout_wait_secs------> PayoutPending --Paid--> Completed
//! any of Funded..Revealed --refund(b) seen-------------> Recovering --k_Z conf--> Recovered
//! ```
//!
//! Secrets (a, s_T, e_T, the joint viewing key) are erased in a terminal stage (R4). The
//! NEAR key is kept: it owns the payout inside `intents.near`.

use anyhow::{anyhow, bail, Context, Result};
use group::GroupEncoding;
use orchard::keys::{FullViewingKey, Scope};
use serde::{Deserialize, Serialize};

#[cfg(feature = "misbehave")]
use crate::config::Misbehave;
use crate::{
    config::{TakerConfig, Windows},
    engine::{self, drive_pending, short, Event, Next, PendingState, PendingZ, Tick},
    keys,
    msg::{self, Accept, Envelope, Quote, Refusal, Rfq},
    near::{escrow, At, Escrow, NearKey, NearRpc, Signer, SwapState},
    net::SharedTransport,
    store::{get_json, put_json, Store},
    wallet::{self, CoverWait, Wallet},
    Me,
};

/// Seconds between polls while waiting for the other party or a chain.
pub const POLL_SECS: u64 = 2;
/// Seconds before a retry after a transient error.
pub const RETRY_SECS: u64 = 5;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TakerStage {
    WaitingDeposit,
    RfqSent,
    Accepted,
    Funded,
    Committed,
    Revealed,
    Claimed,
    /// The claim is final, but the payout is not Paid after `payout_wait_secs`: the
    /// taker's own `withdraw(id)` calls did not pay (for example the Verifier refuses
    /// `mt_transfer`, or the taker's key lacks NEAR). The engine keeps retrying. Any NEAR
    /// account can send the same call (event `payout_pending`). Ends in `Completed`.
    PayoutPending,
    Completed,
    Recovering,
    Recovered,
    Aborted(String),
}

impl TakerStage {
    pub fn name(&self) -> &'static str {
        match self {
            TakerStage::WaitingDeposit => "WaitingDeposit",
            TakerStage::RfqSent => "RfqSent",
            TakerStage::Accepted => "Accepted",
            TakerStage::Funded => "Funded",
            TakerStage::Committed => "Committed",
            TakerStage::Revealed => "Revealed",
            TakerStage::Claimed => "Claimed",
            TakerStage::PayoutPending => "PayoutPending",
            TakerStage::Completed => "Completed",
            TakerStage::Recovering => "Recovering",
            TakerStage::Recovered => "Recovered",
            TakerStage::Aborted(_) => "Aborted",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, TakerStage::Completed | TakerStage::Recovered | TakerStage::Aborted(_))
    }
}

/// The accepted quote, as the taker keeps it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct QuoteRec {
    pub price: String,
    pub premium: u64,
    pub stipend: String,
    pub b_pub: String,
    pub e_m_pub: String,
    pub windows: Windows,
    pub height: u64,
}

impl QuoteRec {
    pub fn price(&self) -> u128 {
        self.price.parse().unwrap_or(0)
    }
    pub fn stipend(&self) -> u128 {
        self.stipend.parse().unwrap_or(0)
    }
}

/// The taker's record of one swap. It is written to the store whenever it changes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TakerSwap {
    pub rfq_id: String,
    pub stage: TakerStage,
    /// Zatoshis sold (the joint note's value, without its sweep fee).
    pub zats: u64,
    /// Taker share, un-normalized. Erased in a terminal stage.
    pub a: String,
    /// Taker seed. Erased in a terminal stage.
    pub s_t: String,
    /// Ephemeral DH secret. Erased in a terminal stage.
    pub e_t: String,
    pub e_t_pub: String,
    /// Seed of the taker's ed25519 NEAR key. Kept: the key owns the payout in Intents.
    pub near_sk: String,
    /// The taker's NEAR and Intents account: hex of the NEAR public key.
    pub taker_account: String,
    pub reply_index: u32,
    pub created_at: u64,
    /// Smallest price the taker accepts for this swap (token base units), from `start`.
    #[serde(default, with = "crate::config::u128_str")]
    pub min_price: u128,
    /// Wall-clock time of the last stage change (for timeouts).
    pub stage_at: u64,
    pub quote: Option<QuoteRec>,
    pub negated: bool,
    pub joint_fvk: Option<String>,
    pub joint_address: Option<String>,
    pub escrow_id: Option<String>,
    /// The stage to move to once `pending` is accepted by a node.
    pub pending_then: Option<TakerStage>,
    pub pending: Option<PendingZ>,
    pub rfq_txid: Option<String>,
    pub accept_txid: Option<String>,
    pub fund_txid: Option<String>,
    pub fund_expiry: Option<u64>,
    pub sweep_txid: Option<String>,
    pub sweep_height: Option<u64>,
    pub commit_tx: Option<String>,
    pub claim_tx: Option<String>,
    /// NEAR time of the last claim(a) broadcast. None while nothing has left the engine.
    pub claim_at: Option<u64>,
    /// Hex of the signed first claim(a) transaction, stored one tick before its broadcast
    /// (security review 2, F2-02). Cleared if R2r fails before the broadcast.
    #[serde(default)]
    pub claim_raw: Option<String>,
    pub withdraw_tx: Option<String>,
    pub withdraw_at: Option<u64>,
    pub log: Vec<String>,
}

impl TakerSwap {
    fn a_scalar(&self) -> Result<pasta_curves::pallas::Scalar> {
        keys::scalar_from_bytes(&keys::unhex32(&self.a)?)
    }
    /// The taker's share as the escrow sees it (normalized).
    fn a_norm(&self) -> Result<pasta_curves::pallas::Scalar> {
        Ok(keys::normalize(self.a_scalar()?, self.negated))
    }
    fn id_bytes(&self) -> Result<[u8; 32]> {
        keys::unhex32(self.escrow_id.as_deref().ok_or_else(|| anyhow!("no escrow id yet"))?)
    }
    fn near_key(&self) -> Result<NearKey> {
        Ok(NearKey::from_seed(keys::unhex32(&self.near_sk)?))
    }
    fn rfq_bytes(&self) -> Result<[u8; 16]> {
        hex::decode(&self.rfq_id)?.try_into().map_err(|_| anyhow!("rfq id length"))
    }
}

/// A quote found among the received memos: the quote, its plaintext, the maker's
/// ephemeral key and the height it was mined at.
type FoundQuote = (Quote, Vec<u8>, [u8; 32], u64);

pub const KEY_WALLET: &str = "wallet";
pub const PREFIX_SWAP: &str = "swap/";

/// Zatoshis the taker's wallet needs for a swap of `zats`: the joint note (with its sweep
/// fee), three transaction fees (request, acceptance, funding) and the largest premium.
pub fn required_deposit(zats: u64, max_premium: u64) -> u64 {
    zats + wallet::zip317_fee(1, 1) + 3 * wallet::zip317_fee(1, 2) + max_premium
}

pub struct Taker {
    pub cfg: TakerConfig,
    pub me: Me,
    pub wallet: Wallet,
    pub near: NearRpc,
    swap: Option<TakerSwap>,
    cover: Option<(String, CoverWait)>,
    ev: Vec<Event>,
    last_balance: Option<u64>,
    announced: Option<String>,
    /// The cover delay that has passed (swap and broadcast), until that broadcast is built.
    cover_passed: Option<String>,
    /// True in a tick whose Zcash sync failed: the wallet's view is old. Only the stages
    /// that need NEAR alone go on (security review 2, F2-02).
    zcash_stale: bool,
}

/// Stages that need only NEAR: a Zcash sync error does not stop them.
fn near_only(stage: &TakerStage) -> bool {
    matches!(stage, TakerStage::Funded | TakerStage::Committed | TakerStage::Revealed | TakerStage::Claimed | TakerStage::PayoutPending)
}

/// yoctoNEAR per gas unit at which NEAR checks that a signer can pay its prepaid gas
/// (`min_gas_purchase_price`; see `near::escrow`).
const PURCHASE_GAS_PRICE: u128 = escrow::GAS_PURCHASE_PRICE;

impl Taker {
    /// Opens the taker's wallet (creating its key in the store on first use) and the
    /// swap in progress, if any.
    pub fn open(cfg: TakerConfig, t: SharedTransport, store: &mut dyn Store) -> Result<Taker> {
        cfg.timing.validate()?;
        let me = match store.get(KEY_WALLET)? {
            Some(b) => Me::from_hex(std::str::from_utf8(&b)?)?,
            None => {
                let me = Me::generate();
                store.put(KEY_WALLET, hex::encode(me.sk_bytes()).as_bytes())?;
                me
            }
        };
        let mut wallet = engine::wallet_for(&cfg.chains, t.clone())?;
        wallet.watch("wallet", &me.fvk, &[Scope::External, Scope::Internal]);
        let near = NearRpc::new(t, &cfg.chains.near_rpc);
        let mut swap = None;
        for k in store.keys(PREFIX_SWAP)? {
            let s: TakerSwap = get_json(store, &k)?.ok_or_else(|| anyhow!("record {k} vanished"))?;
            if !s.stage.is_terminal() {
                swap = Some(s);
            }
        }
        let mut tk = Taker { cfg, me, wallet, near, swap, cover: None, ev: vec![], last_balance: None, announced: None, cover_passed: None, zcash_stale: false };
        if let Some(s) = tk.swap.clone() {
            tk.watch_joint(&s)?;
            if let Some(p) = &s.pending {
                tk.wallet.track(&p.txid);
            }
            for t in [&s.fund_txid, &s.sweep_txid].into_iter().flatten() {
                tk.wallet.track(t);
            }
        }
        Ok(tk)
    }

    /// The temporary wallet's unified address (where the user pays).
    pub fn ua(&self) -> String {
        self.me.ua()
    }

    /// The swap in progress, or the last one this engine ran.
    pub fn swap(&self) -> Option<&TakerSwap> {
        self.swap.as_ref()
    }

    /// Spendable zatoshis in the temporary wallet as of the last sync.
    pub fn balance(&self) -> u64 {
        self.wallet.spendable("wallet")
    }

    fn esc(&self) -> Escrow<'_> {
        Escrow { rpc: &self.near, escrow: self.cfg.chains.escrow.clone(), token: self.cfg.chains.token.clone(), intents: self.cfg.chains.intents.clone() }
    }

    /// Checks the maker's directory entry: the ed25519 signature, and that the key is a
    /// full-access key of the maker's NEAR account at a final block. A taker that fails
    /// this sends nothing.
    pub fn verify_maker(&self) -> Result<()> {
        let m = &self.cfg.maker;
        let pk = crate::near::tx::parse_public_key(&m.near_pk)?;
        let sig: [u8; 64] = hex::decode(&m.sig)?.try_into().map_err(|_| anyhow!("directory signature must be 64 bytes"))?;
        let digest = m.digest()?;
        if !crate::near::tx::verify(&pk, &digest, &sig) {
            bail!("maker directory entry is not signed by {}", m.near_pk);
        }
        match self.near.access_key(&m.near_account, &m.near_pk, &At::Final)? {
            Some(k) if k.full_access => Ok(()),
            Some(_) => bail!("maker key {} is not a full-access key of {}", m.near_pk, m.near_account),
            None => bail!("maker key {} is not a key of {}", m.near_pk, m.near_account),
        }
    }

    /// Starts a swap of `zats` zatoshis: checks the maker's directory entry, creates the
    /// per-swap secrets and stores them. Nothing is sent until the next `tick`.
    pub fn start(&mut self, zats: u64, now: u64, store: &mut dyn Store) -> Result<&TakerSwap> {
        let min = self.cfg.min_price;
        self.start_with_min_price(zats, min, now, store)
    }

    /// `start`, with the smallest acceptable price (token base units) for this swap; a page
    /// sets it from the price it showed. It is stored in the swap record.
    pub fn start_with_min_price(&mut self, zats: u64, min_price: u128, now: u64, store: &mut dyn Store) -> Result<&TakerSwap> {
        if let Some(s) = &self.swap
            && !s.stage.is_terminal() {
                bail!("swap {} is still in progress", short(&s.rfq_id));
            }
        if zats == 0 {
            bail!("amount is zero");
        }
        self.verify_maker()?;
        // The signed limits (if the entry has them): a request outside them would only cost
        // the request fee and get a refusal or no answer.
        if let Some(l) = self.cfg.maker.limits {
            if zats < l.min_zats || zats > l.max_zats {
                bail!("amount {zats} zats is outside the maker's published limits {}..={} zats; nothing was sent", l.min_zats, l.max_zats);
            }
            if l.premium_zats > self.cfg.max_premium {
                bail!("the maker's published premium {} zats is above this swap's limit {}; nothing was sent", l.premium_zats, self.cfg.max_premium);
            }
        }
        let rfq_id: [u8; 16] = crate::random_bytes();
        let a = keys::random_scalar();
        let e_t = keys::random_scalar();
        let near = NearKey::generate();
        let s = TakerSwap {
            rfq_id: hex::encode(rfq_id),
            stage: TakerStage::WaitingDeposit,
            zats,
            a: hex::encode(keys::scalar_to_bytes(&a)),
            s_t: hex::encode(crate::random_bytes::<32>()),
            e_t: hex::encode(keys::scalar_to_bytes(&e_t)),
            e_t_pub: hex::encode(msg::dh_public(&e_t)),
            near_sk: hex::encode(near.seed()),
            taker_account: near.implicit_account(),
            reply_index: u32::from_le_bytes(rfq_id[..4].try_into().unwrap()) & 0x7fff_ffff,
            created_at: now,
            min_price,
            stage_at: now,
            quote: None,
            negated: false,
            joint_fvk: None,
            joint_address: None,
            escrow_id: None,
            pending_then: None,
            pending: None,
            rfq_txid: None,
            accept_txid: None,
            fund_txid: None,
            fund_expiry: None,
            sweep_txid: None,
            sweep_height: None,
            commit_tx: None,
            claim_tx: None,
            claim_at: None,
            claim_raw: None,
            withdraw_tx: None,
            withdraw_at: None,
            log: vec![],
        };
        put_json(store, &format!("{PREFIX_SWAP}{}", s.rfq_id), &s)?;
        self.ev.push(Event::Stage { swap: s.rfq_id.clone(), stage: s.stage.name().into() });
        self.swap = Some(s);
        Ok(self.swap.as_ref().unwrap())
    }

    /// Events produced outside `tick` (by `start`), to deliver with the next tick.
    pub fn take_events(&mut self) -> Vec<Event> {
        self.drain_wallet_notes();
        std::mem::take(&mut self.ev)
    }

    /// The wallet's notes (rollbacks, repairs) as log events of the current swap.
    fn drain_wallet_notes(&mut self) {
        let swap = self.swap.as_ref().map(|s| s.rfq_id.clone()).unwrap_or_default();
        for line in self.wallet.take_notes() {
            self.ev.push(Event::Log { swap: swap.clone(), line: format!("Zcash wallet: {line}") });
        }
    }

    /// One step of the state machine. Never fails: a transient error becomes a `Warning`
    /// event and a retry time.
    pub fn tick(&mut self, now: u64, store: &mut dyn Store) -> Tick {
        let r = self.step(now, store);
        let next = match r {
            Ok(n) => n,
            Err(e) => {
                let swap = self.swap.as_ref().map(|s| s.rfq_id.clone()).unwrap_or_default();
                self.ev.push(Event::Warning { swap, message: format!("{e:#}") });
                Next::At(now + RETRY_SECS)
            }
        };
        self.drain_wallet_notes();
        Tick { next, events: std::mem::take(&mut self.ev) }
    }

    fn step(&mut self, now: u64, store: &mut dyn Store) -> Result<Next> {
        // F2-02: a Zcash sync error stops only the stages that need the Zcash wallet. The
        // NEAR stages (Funded to PayoutPending) go on with the last synced view; a decision
        // that needs Zcash data (a recovery) waits for a good sync.
        self.zcash_stale = false;
        if let Err(e) = self.wallet.sync() {
            match &self.swap {
                Some(s) if near_only(&s.stage) => {
                    self.ev.push(Event::Warning { swap: s.rfq_id.clone(), message: format!("Zcash sync: {e:#}; the NEAR steps go on") });
                    self.zcash_stale = true;
                }
                _ => return Err(e.context("Zcash sync")),
            }
        }
        if !self.zcash_stale {
            let bal = self.balance();
            if self.last_balance != Some(bal) {
                self.last_balance = Some(bal);
                self.ev.push(Event::Balance { zats: bal });
            }
        }
        let Some(mut s) = self.swap.clone() else { return Ok(Next::Done) };
        if s.stage.is_terminal() {
            return Ok(Next::Done);
        }
        let before = s.clone();
        let r = self.advance(&mut s, now);
        if s != before {
            if s.stage.is_terminal() {
                // R4: erase per-swap secrets once nothing can still need them.
                s.a.clear();
                s.s_t.clear();
                s.e_t.clear();
                s.joint_fvk = None;
                self.ev.push(Event::Done { swap: s.rfq_id.clone(), outcome: s.stage.name().into() });
            }
            put_json(store, &format!("{PREFIX_SWAP}{}", s.rfq_id), &s)?;
            self.swap = Some(s.clone());
        }
        let next = r?;
        Ok(if s.stage.is_terminal() {
            Next::Done
        } else if self.zcash_stale {
            // Come back soon for the sync, but not before the NEAR step asked to.
            match next {
                Next::At(t) => Next::At(t.min(now + RETRY_SECS)),
                Next::Done => Next::Done,
            }
        } else {
            next
        })
    }

    fn log(&mut self, s: &mut TakerSwap, line: String) {
        self.ev.push(Event::Log { swap: s.rfq_id.clone(), line: line.clone() });
        s.log.push(line);
    }

    /// Logs a line unless it is the same as the last one (for conditions polled each tick).
    fn log_once(&mut self, s: &mut TakerSwap, line: String) {
        if s.log.last() != Some(&line) {
            self.log(s, line);
        }
    }

    fn set_stage(&mut self, s: &mut TakerSwap, stage: TakerStage, now: u64) {
        s.stage = stage;
        s.stage_at = now;
        self.ev.push(Event::Stage { swap: s.rfq_id.clone(), stage: s.stage.name().into() });
    }

    fn abort(&mut self, s: &mut TakerSwap, why: String, now: u64) -> Result<Next> {
        self.log(s, format!("ABORT: {why}"));
        self.set_stage(s, TakerStage::Aborted(why), now);
        Ok(Next::Done)
    }

    fn watch_joint(&mut self, s: &TakerSwap) -> Result<()> {
        if let Some(f) = &s.joint_fvk {
            let b: [u8; 96] = hex::decode(f)?.try_into().map_err(|_| anyhow!("fvk length"))?;
            let fvk = FullViewingKey::from_bytes(&b).ok_or_else(|| anyhow!("bad joint fvk"))?;
            self.wallet.watch(&s.rfq_id, &fvk, &[Scope::External]);
        }
        Ok(())
    }

    /// Cover delay before a broadcast that follows a public trigger. True once it may go.
    fn cover_ready(&mut self, s: &mut TakerSwap, what: &str, trigger_height: u64, deadline: Option<u64>, now: u64) -> bool {
        let key = format!("{}:{what}", s.rfq_id);
        if self.cover_passed.as_deref() == Some(key.as_str()) {
            return true;
        }
        if self.cover.as_ref().map(|(k, _)| k != &key).unwrap_or(true) {
            self.cover = Some((key.clone(), CoverWait::new(self.cfg.cover.into(), trigger_height, deadline, now)));
        }
        let (_, c) = self.cover.as_mut().unwrap();
        match c.poll(&self.wallet, now) {
            Some(outcome) => {
                self.cover = None;
                self.cover_passed = Some(key);
                if self.cfg.cover.k > 0 || self.cfg.cover.max_secs > 0 {
                    self.log(s, format!("{what}: {outcome}"));
                }
                true
            }
            None => false,
        }
    }

    /// Returns true once the proof for `what` has been announced in an earlier tick. The
    /// first call emits an `Activity` event and returns false: the caller returns at once,
    /// so the event reaches the page before the worker blocks on the proof.
    fn announced(&mut self, s: &TakerSwap, what: &str, label: &str) -> bool {
        let key = format!("{}:{what}", s.rfq_id);
        if self.announced.as_deref() == Some(key.as_str()) {
            self.announced = None;
            return true;
        }
        self.announced = Some(key);
        self.ev.push(Event::Activity { swap: s.rfq_id.clone(), kind: "proving".into(), label: label.into(), estimated_secs: 25 });
        false
    }

    fn signer(&self, s: &TakerSwap) -> Result<Signer> {
        Ok(Signer::new(&s.taker_account, s.near_key()?))
    }

    fn advance(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        // A built Zcash transaction goes out before anything else happens. With an old
        // Zcash view (sync failed) it waits; the NEAR stage below still runs.
        if let Some(mut p) = s.pending.clone().filter(|_| !self.zcash_stale) {
            let st = drive_pending(&mut self.wallet, &mut p)?;
            let first_accept = (p.broadcast || matches!(st, PendingState::Mined(_))) && s.pending_then.is_some();
            s.pending = Some(p.clone());
            if first_accept {
                self.ev.push(Event::ZcashTx { swap: s.rfq_id.clone(), what: p.what.clone(), txid: p.txid.clone() });
                if let Some(then) = s.pending_then.take() {
                    self.on_broadcast(s, &p, then, now);
                }
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
                    if s.pending_then.take().is_some() {
                        // Refused before any node took it: build it again on the next tick.
                        self.log(s, format!("{} transaction {} {why}; building it again", p.what, short(&p.txid)));
                        return Ok(Next::At(now + RETRY_SECS));
                    }
                    self.log(s, format!("{} transaction {} {why}", p.what, short(&p.txid)));
                    match p.what.as_str() {
                        "rfq" | "accept" => return self.abort(s, format!("{} message expired unmined", p.what), now),
                        "sweep" => s.sweep_txid = None,
                        _ => {} // a dead funding: handled by the recovery rules (R4)
                    }
                }
            }
            if s.pending.as_ref().map(|p| !p.broadcast).unwrap_or(false) {
                return Ok(Next::At(now + RETRY_SECS));
            }
        }
        match s.stage.clone() {
            TakerStage::WaitingDeposit => self.on_deposit(s, now),
            TakerStage::RfqSent => self.on_quote(s, now),
            TakerStage::Accepted => self.on_open(s, now),
            TakerStage::Funded => self.on_funded(s, now),
            TakerStage::Committed => self.on_committed(s, now),
            TakerStage::Revealed => self.on_revealed(s, now),
            TakerStage::Claimed | TakerStage::PayoutPending => self.on_claimed(s, now),
            TakerStage::Recovering => self.on_recovering(s, now),
            TakerStage::Completed | TakerStage::Recovered | TakerStage::Aborted(_) => Ok(Next::Done),
        }
    }

    fn on_broadcast(&mut self, s: &mut TakerSwap, p: &PendingZ, then: TakerStage, now: u64) {
        let line = match p.what.as_str() {
            "rfq" => {
                s.rfq_txid = Some(p.txid.clone());
                format!("encrypted RFQ for {} zats sent to the maker in shielded tx {}", s.zats, p.txid)
            }
            "accept" => {
                s.accept_txid = Some(p.txid.clone());
                format!(
                    "encrypted ACCEPT with {} zats premium sent in tx {}; joint address {}",
                    s.quote.as_ref().map(|q| q.premium).unwrap_or(0),
                    p.txid,
                    s.joint_address.clone().unwrap_or_default()
                )
            }
            "fund" => format!("FUNDED the joint address in fully shielded tx {}", p.txid),
            "sweep" => format!("swept the joint note back with (a+b), tx {}; waiting for {} confirmations", p.txid, self.cfg.timing.k_z),
            other => format!("{other} tx {}", p.txid),
        };
        self.log(s, line);
        self.set_stage(s, then, now);
    }

    fn pend(&mut self, s: &mut TakerSwap, what: &str, tx: &wallet::BuiltTx, then: TakerStage) -> Next {
        self.cover_passed = None;
        s.pending = Some(PendingZ::new(what, tx));
        s.pending_then = Some(then);
        // Stored before the first broadcast (the record is saved at the end of this tick);
        // the broadcast happens on the next tick.
        Next::At(0)
    }

    // ------------------------------------------------------------------ stages

    fn on_deposit(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let need = required_deposit(s.zats, self.cfg.max_premium);
        let have = self.balance();
        if have < need {
            self.log_once(s, format!("waiting for a deposit: {have} of {need} zats in {}", self.ua()));
            return Ok(Next::At(now + POLL_SECS));
        }
        let rfq_id = s.rfq_bytes()?;
        let a = s.a_scalar()?;
        let s_t = keys::unhex32(&s.s_t)?;
        let e_t = keys::scalar_from_bytes(&keys::unhex32(&s.e_t)?)?;
        let e_t_pub = keys::unhex32(&s.e_t_pub)?;
        let rfq = Rfq {
            rfq_id,
            zats: s.zats,
            a_pub: (keys::g() * a).to_bytes(),
            pok_a: keys::pok_prove(&a, &rfq_id, b"taker"),
            commit_s_t: keys::h32(b"commit-sT", &[&s_t]),
            taker_pk: s.near_key()?.public_bytes(),
            reply_addr: self.me.fvk.address_at(s.reply_index, Scope::External).to_raw_address_bytes(),
        };
        let m = &self.cfg.maker;
        let maker_pub = keys::unhex32(&m.rfq_pub)?;
        let ct = msg::seal(b"rfq", &e_t, &maker_pub, &e_t_pub, &maker_pub, &rfq.to_bytes())?;
        let env = Envelope::Rfq { epoch: m.epoch, eph: e_t_pub, ct };
        let to = wallet::decode_ua(&m.ua)?;
        if !self.announced(s, "rfq", "Building the proof for the request to the maker") {
            return Ok(Next::At(0));
        }
        let tx = self.wallet.build_pay("wallet", &self.me.ask, self.me.change(), vec![(to, 0, env.encode())])?;
        Ok(self.pend(s, "rfq", &tx, TakerStage::RfqSent))
    }

    /// The maker's quote for this swap among the received memos: the quote, its plaintext,
    /// the maker's ephemeral key and the height it was mined at.
    fn find_quote(&self, s: &TakerSwap) -> Result<Option<FoundQuote>> {
        let rfq_id = s.rfq_bytes()?;
        let e_t = keys::scalar_from_bytes(&keys::unhex32(&s.e_t)?)?;
        let e_t_pub = keys::unhex32(&s.e_t_pub)?;
        Ok(self.wallet.received.iter().filter(|r| r.label == "wallet").find_map(|r| match Envelope::decode(&r.memo) {
            Ok(Some(Envelope::Quote { rfq_id: id, eph, ct })) if id == rfq_id => {
                msg::open(b"quote", &e_t, &eph, &e_t_pub, &eph, &ct).ok().and_then(|b| Quote::from_bytes(&b).ok().map(|q| (q, b, eph, r.height)))
            }
            _ => None,
        }))
    }

    /// The maker's signed refusal of this request among the received memos, if any. A memo
    /// that does not open, or whose signature is not the maker's, is ignored.
    fn find_refusal(&self, s: &TakerSwap) -> Result<Option<Refusal>> {
        let rfq_id = s.rfq_bytes()?;
        let e_t = keys::scalar_from_bytes(&keys::unhex32(&s.e_t)?)?;
        let e_t_pub = keys::unhex32(&s.e_t_pub)?;
        let maker_pub = keys::unhex32(&self.cfg.maker.rfq_pub)?;
        let Ok(maker_pk) = crate::near::tx::parse_public_key(&self.cfg.maker.near_pk) else { return Ok(None) };
        Ok(self.wallet.received.iter().filter(|r| r.label == "wallet").find_map(|r| match Envelope::decode(&r.memo) {
            Ok(Some(Envelope::Refuse { rfq_id: id, ct })) if id == rfq_id => msg::open(b"refuse", &e_t, &maker_pub, &e_t_pub, &maker_pub, &ct)
                .ok()
                .and_then(|b| Refusal::from_bytes(&b).ok())
                .filter(|f| crate::near::tx::verify(&maker_pk, &f.sig_digest(&rfq_id), &f.sig)),
            _ => None,
        }))
    }

    /// Takes a verified quote into the record: the joint key, the escrow id and the quote
    /// itself, and watches the joint address.
    fn adopt_quote(&mut self, s: &mut TakerSwap, q: &Quote, e_m_pub: [u8; 32], quote_height: u64) -> Result<()> {
        let rfq_id = s.rfq_bytes()?;
        let a = s.a_scalar()?;
        let s_t = keys::unhex32(&s.s_t)?;
        let joint = keys::joint(&rfq_id, keys::g() * a, keys::point_from_bytes(&q.b_pub)?, &s_t, &q.s_m)?;
        let salt = keys::escrow_salt(&rfq_id, &s_t, &q.s_m);
        s.escrow_id = Some(hex::encode(escrow::swap_id(&self.cfg.maker.near_account, &salt)));
        s.negated = joint.negated;
        s.joint_fvk = Some(hex::encode(joint.fvk.to_bytes()));
        s.joint_address = Some(wallet::encode_ua(&joint.fvk.address_at(0u32, Scope::External)));
        s.quote = Some(QuoteRec {
            price: q.price.to_string(),
            premium: q.premium,
            stipend: q.stipend.to_string(),
            b_pub: hex::encode(q.b_pub),
            e_m_pub: hex::encode(e_m_pub),
            windows: Windows { t0_secs: q.t0_secs, t1_secs: q.t1_secs, t2_secs: q.t2_secs, reveal_secs: q.reveal_secs },
            height: quote_height,
        });
        self.watch_joint(s)?;
        self.log(s, format!("QUOTE decrypted and verified (price {}, premium {} zats, stipend {} yocto, maker key valid)", q.price, q.premium, q.stipend));
        self.ev.push(Event::Quote {
            swap: s.rfq_id.clone(),
            price: q.price.to_string(),
            premium: q.premium,
            stipend: q.stipend.to_string(),
            joint_address: s.joint_address.clone().unwrap_or_default(),
        });
        Ok(())
    }

    /// For a swap restored from a recovery file into a fresh store: moves the record forward
    /// to what the chains show, so that no message and no payment is sent twice. A recovery
    /// file is usually saved before the swap starts, so its record says `WaitingDeposit`
    /// while the request, the acceptance or the funding may already be on chain.
    ///
    /// Only this engine spends from the temporary wallet, so its mined sends are, in order:
    /// the request, the acceptance (the first send at or after the quote's height) and the
    /// funding (also visible as a note at the joint address). A quote that fails the checks
    /// is left for `tick`, which rejects it. From `Funded` on, `tick` reads the escrow and
    /// needs no help. A transaction still in the mempool is not visible here: the host
    /// should call this again until a few new blocks are mined before it calls `tick`.
    /// Returns true if the record changed (it is then stored).
    pub fn reconcile(&mut self, now: u64, store: &mut dyn Store) -> Result<bool> {
        let Some(mut s) = self.swap.clone() else { return Ok(false) };
        if s.stage.is_terminal() || s.pending.is_some() {
            return Ok(false);
        }
        self.wallet.sync().context("Zcash sync")?;
        let before = s.clone();
        let sends = self.wallet.sends("wallet");
        if s.stage == TakerStage::WaitingDeposit
            && let Some((txid, h)) = sends.first().cloned()
        {
            s.rfq_txid = Some(txid.clone());
            self.log(&mut s, format!("restored: the request was already sent in tx {txid} (height {h})"));
            self.set_stage(&mut s, TakerStage::RfqSent, now);
        }
        if s.stage == TakerStage::RfqSent
            && let Some((q, _body, e_m_pub, qh)) = self.find_quote(&s)?
            && self.reject_quote(&q, &s.rfq_bytes()?, s.min_price).is_none()
            && let Some((txid, h)) = sends.iter().find(|(_, h)| *h >= qh).cloned()
        {
            if s.joint_fvk.is_none() {
                self.adopt_quote(&mut s, &q, e_m_pub, qh)?;
                // The joint key is new to the wallet: scan again to find a funding note.
                self.wallet.sync().context("Zcash sync")?;
            }
            s.accept_txid = Some(txid.clone());
            self.log(&mut s, format!("restored: the acceptance was already sent in tx {txid} (height {h})"));
            self.set_stage(&mut s, TakerStage::Accepted, now);
        }
        if s.stage == TakerStage::Accepted
            && let Some(n) = self.wallet.received.iter().find(|r| r.label == s.rfq_id).cloned()
        {
            s.fund_txid = Some(n.txid.clone());
            self.wallet.track(&n.txid);
            self.log(&mut s, format!("restored: the joint address was already funded in tx {} (height {})", n.txid, n.height));
            self.set_stage(&mut s, TakerStage::Funded, now);
        }
        if s == before {
            return Ok(false);
        }
        put_json(store, &format!("{PREFIX_SWAP}{}", s.rfq_id), &s)?;
        self.swap = Some(s);
        Ok(true)
    }

    fn on_quote(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let rfq_id = s.rfq_bytes()?;
        let e_t = keys::scalar_from_bytes(&keys::unhex32(&s.e_t)?)?;
        let e_t_pub = keys::unhex32(&s.e_t_pub)?;
        let found = self.find_quote(s)?;
        let Some((q, body, e_m_pub, quote_height)) = found else {
            if let Some(f) = self.find_refusal(s)? {
                let why = match f.code {
                    msg::RefuseCode::OverLimit => format!("the amount {} zats is above the maker's limit of {} zats", s.zats, f.max_zats),
                    msg::RefuseCode::UnderMinimum => format!("the amount {} zats is below the maker's minimum of {} zats", s.zats, f.min_zats),
                    msg::RefuseCode::Busy => "the maker has too many requests waiting; try again later".to_string(),
                    msg::RefuseCode::Unsupported => "the maker does not take this request".to_string(),
                };
                self.ev.push(Event::Refused { swap: s.rfq_id.clone(), code: f.code.name().into(), min_zats: f.min_zats, max_zats: f.max_zats });
                return self.abort(s, format!("MAKER REFUSED ({}): {why}; nothing was funded", f.code.name()), now);
            }
            if now > s.stage_at.saturating_add(self.cfg.quote_timeout_secs) {
                return self.abort(s, "no quote before the timeout; nothing funded".into(), now);
            }
            return Ok(Next::At(now + POLL_SECS));
        };
        if let Some(why) = self.reject_quote(&q, &rfq_id, s.min_price) {
            return self.abort(s, format!("QUOTE REJECTED: {why}; nothing was funded"), now);
        }
        let need = s.zats + wallet::zip317_fee(1, 1) + 2 * wallet::zip317_fee(1, 2) + q.premium;
        if self.balance() < need {
            return self.abort(s, format!("QUOTE REJECTED: the wallet holds {} zats, the swap needs {need}; nothing was funded", self.balance()), now);
        }
        // Joint key and the acceptance. Data from the quote is kept before the acceptance
        // goes out (the record is stored with the pending transaction).
        if s.joint_fvk.is_none() {
            self.adopt_quote(s, &q, e_m_pub, quote_height)?;
            return Ok(Next::At(0));
        }
        if !self.cover_ready(s, "ACCEPT", quote_height, None, now) {
            return Ok(Next::At(now + POLL_SECS));
        }
        let s_t = keys::unhex32(&s.s_t)?;
        let quote_hash = keys::h32(b"quote", &[&rfq_id, &body]);
        let ct = msg::seal(b"accept", &e_t, &e_m_pub, &e_t_pub, &e_m_pub, &Accept { s_t, quote_hash }.to_bytes())?;
        let env = Envelope::Accept { rfq_id, ct };
        let to = wallet::decode_ua(&self.cfg.maker.ua)?;
        if !self.announced(s, "accept", "Building the proof for the acceptance and the premium") {
            return Ok(Next::At(0));
        }
        let tx = self.wallet.build_pay("wallet", &self.me.ask, self.me.change(), vec![(to, q.premium, env.encode())])?;
        Ok(self.pend(s, "accept", &tx, TakerStage::Accepted))
    }

    /// Rule R1q and the identity checks. `Some(reason)` rejects the quote.
    fn reject_quote(&self, q: &Quote, rfq_id: &[u8; 16], min_price: u128) -> Option<String> {
        let c = &self.cfg;
        let t = &c.timing;
        // H1: a configured maker key that does not parse rejects every quote (fail closed).
        let maker_pk = match crate::near::tx::parse_public_key(&c.maker.near_pk) {
            Ok(k) => k,
            Err(e) => return Some(format!("the configured maker key does not parse ({e:#})")),
        };
        let b_pub = match keys::point_from_bytes(&q.b_pub) {
            Ok(p) => p,
            Err(_) => return Some("maker key B is not a valid point".into()),
        };
        let horizon = q.t0_secs as u64 + q.t1_secs as u64 + q.t2_secs as u64;
        if !keys::pok_verify(&b_pub, &q.pok_b, rfq_id, b"maker") {
            Some("maker proof of knowledge INVALID (possible rogue-key attack)".into())
        } else if q.escrow != c.chains.escrow {
            Some(format!("quote names an unknown escrow {}", q.escrow))
        } else if q.maker != c.maker.near_account {
            Some(format!("quote names an unexpected maker {}", q.maker))
        } else if !crate::near::tx::verify(&maker_pk, &q.sig_digest(rfq_id), &q.sig) {
            Some("quote is not signed by the maker's NEAR key".into())
        } else if q.price < min_price {
            Some(format!("price {} below the taker's minimum {min_price}", q.price))
        } else if q.price == 0 {
            Some("price is zero".into())
        } else if q.premium > c.max_premium {
            Some(format!("premium {} above the taker's limit {}", q.premium, c.max_premium))
        } else if q.stipend < c.min_stipend {
            Some(format!("stipend {} below the taker's minimum {}", q.stipend, c.min_stipend))
        } else if q.reveal_secs as u64 <= t.finality_secs.saturating_add(t.budget()) {
            Some(format!("reveal window {}s too short for finality (R1q needs W > F + B_R = {}s)", q.reveal_secs, t.finality_secs.saturating_add(t.budget())))
        } else if horizon > c.max_horizon_secs {
            Some(format!("deadline horizon t2 - open = {horizon}s above the taker's limit of {}s", c.max_horizon_secs))
        } else if q.t0_secs as u64 <= t.finality_secs.saturating_add(t.fund_secs).saturating_add(c.cover.max_secs.saturating_mul(3)) {
            // The funding waits for open's finality and then for a cover delay; if t0 cannot
            // absorb the delay's tail (3 draws), the wait would be cut at a public deadline.
            Some(format!("funding window t0 - open = {}s too short for finality, funding and the cover delay", q.t0_secs))
        } else if q.t1_secs as u64 <= t.finality_secs.saturating_add(t.budget()) {
            Some(format!("claim window t1 - t0 = {}s too short (needs > F + B_R)", q.t1_secs))
        } else {
            None
        }
    }

    fn on_open(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let id = s.id_bytes()?;
        let q = s.quote.clone().ok_or_else(|| anyhow!("no quote"))?;
        let Some(v) = self.esc().get_swap(&id, &At::Final)? else {
            if now > s.stage_at.saturating_add(self.cfg.quote_timeout_secs) {
                return self.abort(s, "the maker never opened the escrow; nothing funded".into(), now);
            }
            return Ok(Next::At(now + POLL_SECS));
        };
        // R1: every stored parameter must equal what was quoted and requested.
        let a = s.a_scalar()?;
        let (mut a_pub, mut b_pub) = (keys::g() * a, keys::point_from_hex(&q.b_pub)?);
        if s.negated {
            a_pub = -a_pub;
            b_pub = -b_pub;
        }
        let w = q.windows;
        let fin = self.near.block(&At::Final)?;
        let tnow = fin.secs();
        // F2-03: the windows must EQUAL the quote (the maker writes exactly these), and t0
        // can be no later than the quoted funding window from now.
        let checks = [
            ("maker", v.maker == self.cfg.maker.near_account),
            ("token", v.token == self.cfg.chains.token),
            ("amount", v.amount == q.price()),
            ("taker key", v.taker_pk == hex::encode(s.near_key()?.public_bytes()) && v.taker_account == s.taker_account),
            ("A", v.a_pub == hex::encode(a_pub.to_bytes())),
            ("B", v.b_pub == hex::encode(b_pub.to_bytes())),
            ("funding window", v.t0 <= tnow.saturating_add(w.t0_secs as u64)),
            ("claim window", v.t1.checked_sub(v.t0) == Some(w.t1_secs as u64)),
            ("punish window", v.t2.checked_sub(v.t1) == Some(w.t2_secs as u64)),
            ("reveal window", v.reveal_window == w.reveal_secs as u64),
            ("state", v.state == SwapState::Open),
        ];
        if let Some((what, _)) = checks.iter().find(|(_, ok)| !ok) {
            return self.abort(s, format!("escrow {what} does not match the quote; nothing funded"), now);
        }
        // The escrow must not be able to change under us, and must pay into the expected
        // Intents account (security review 1, NV-01).
        if let Err(e) = escrow::check_escrow(&self.near, &self.cfg.chains.escrow, &self.cfg.escrow_code_hash, &self.cfg.chains.intents, &At::Block(fin.hash.clone())) {
            return self.abort(s, format!("escrow contract check failed: {e:#}; nothing funded"), now);
        }
        // The tokens of this swap must be in the escrow's Verifier account and not be
        // promised to another swap: at this same final block, the escrow's Verifier
        // balance covers everything it owes in the token. A payout of another swap can
        // make the balance drop one or two blocks before the owed sum does, so a failed
        // check waits and reads again; only a check that still fails near the end of the
        // funding window aborts.
        if let Err(e) = escrow::check_backing(&self.near, &self.cfg.chains.escrow, &self.cfg.chains.token, &self.cfg.chains.intents, &At::Block(fin.hash.clone())) {
            if tnow.saturating_add(self.cfg.timing.fund_secs).saturating_add(self.cfg.timing.budget()) >= v.t0 {
                return self.abort(s, format!("escrow backing check failed: {e:#}; nothing funded"), now);
            }
            self.log_once(s, format!("escrow backing check failed at block {}: {e:#}; reading again", fin.height));
            return Ok(Next::At(now + POLL_SECS));
        }
        if v.t2 > tnow.saturating_add(self.cfg.max_horizon_secs) {
            return self.abort(s, format!("escrow t2 {} is beyond the taker's horizon; nothing funded", v.t2), now);
        }
        // R1b: fund only if the maker can still confirm the note and set Ready before t0.
        if tnow.saturating_add(self.cfg.timing.fund_secs) >= v.t0 {
            return self.abort(s, format!("(R1b) NEAR time {tnow} + fund window {}s is not before t0 {}; not funding", self.cfg.timing.fund_secs, v.t0), now);
        }
        // The stipend pays for commit_claim and claim. It must be there before the ZEC moves.
        let bal = self.near.account(&s.taker_account, &At::Final)?.map(|a| a.amount).unwrap_or(0);
        if bal < q.stipend() {
            self.log_once(s, format!("escrow open and final; waiting for the stipend ({bal} of {} yoctoNEAR)", q.stipend()));
            return Ok(Next::At(now + POLL_SECS));
        }
        if s.fund_txid.is_none() {
            self.log(s, format!("escrow {} verified ({} checks) at a final block; stipend of {bal} yoctoNEAR arrived", short(&hex::encode(id)), checks.len()));
            self.ev.push(Event::EscrowVerified { swap: s.rfq_id.clone(), escrow_id: hex::encode(id), t0: v.t0, t1: v.t1, t2: v.t2 });
        }
        #[cfg(feature = "misbehave")]
        if self.cfg.misbehave == Some(Misbehave::FreeClaim) {
            self.log(s, "[MISBEHAVE free-claim] escrow verified; NOT funding; will try to claim after t0".into());
            self.set_stage(s, TakerStage::Funded, now);
            return Ok(Next::At(now));
        }
        // A note at the joint address means the funding is already out (a record restored
        // from an early recovery file, or a funding mined after a refused rebroadcast).
        // Never fund twice.
        if let Some(n) = self.wallet.received.iter().find(|r| r.label == s.rfq_id).cloned() {
            s.fund_txid = Some(n.txid.clone());
            self.wallet.track(&n.txid);
            self.log(s, format!("the joint address already holds the funding (tx {})", n.txid));
            self.set_stage(s, TakerStage::Funded, now);
            return Ok(Next::At(now));
        }
        // Decorrelate the funding from the public open, inside R1b's deadline.
        let deadline = now.saturating_add(v.t0.saturating_sub(tnow.saturating_add(self.cfg.timing.fund_secs)));
        let tip = self.wallet.height;
        if !self.cover_ready(s, "FUND", tip, Some(deadline), now) {
            return Ok(Next::At(now + POLL_SECS));
        }
        let joint = wallet::decode_ua(s.joint_address.as_deref().ok_or_else(|| anyhow!("no joint address"))?)?;
        #[cfg_attr(not(feature = "misbehave"), allow(unused_mut))]
        let mut amount = s.zats + wallet::zip317_fee(1, 1);
        #[cfg(feature = "misbehave")]
        if self.cfg.misbehave == Some(Misbehave::Underfund) {
            amount /= 2;
        }
        let memo = Envelope::Fund { rfq_id: s.rfq_bytes()? }.encode();
        if !self.announced(s, "fund", "Building the proof for the payment to the shared address") {
            return Ok(Next::At(0));
        }
        let tx = self.wallet.build_pay("wallet", &self.me.ask, self.me.change(), vec![(joint, amount, memo)])?;
        s.fund_txid = Some(tx.txid.clone());
        s.fund_expiry = Some(tx.expiry_height);
        Ok(self.pend(s, "fund", &tx, TakerStage::Funded))
    }

    fn commit(&mut self, s: &mut TakerSwap) -> Result<crate::near::TxOutcome> {
        let id = s.id_bytes()?;
        let a = keys::scalar_to_bytes(&s.a_norm()?);
        let c = escrow::commitment(&id, &a);
        let msg = escrow::commit_message(&self.cfg.chains.escrow, &id, &c);
        let sig = s.near_key()?.sign(&msg);
        let (to, actions) = self.esc().commit_claim(&id, &c, &sig);
        let signer = self.signer(s)?;
        let out = self.near.transact(&signer, &to, actions)?;
        s.commit_tx = Some(out.hash.clone());
        Ok(out)
    }

    fn on_funded(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let id = s.id_bytes()?;
        let fin = self.near.block(&At::Final)?;
        let tnow = fin.secs();
        let v = self.esc().get_swap(&id, &At::Block(fin.hash.clone()))?.ok_or_else(|| anyhow!("escrow vanished"))?;
        let t = self.cfg.timing.clone();
        // Test-only behaviours exist only in a build with the cargo feature `misbehave` (H5).
        #[cfg(feature = "misbehave")]
        let mis = self.cfg.misbehave;
        match v.state {
            SwapState::Refunded => self.recover(s, &v, now),
            SwapState::Locked | SwapState::Claimed => {
                self.log(s, "escrow already locked; resuming".into());
                self.set_stage(s, TakerStage::Committed, now);
                Ok(Next::At(now))
            }
            #[cfg(feature = "misbehave")]
            SwapState::Ready if t.in_time(tnow, v.t1) && mis != Some(Misbehave::LateCommit) => self.commit_ready(s, now),
            #[cfg(not(feature = "misbehave"))]
            SwapState::Ready if t.in_time(tnow, v.t1) => self.commit_ready(s, now),
            SwapState::Open | SwapState::Ready if tnow >= v.t2 => {
                // Punish path: the maker neither set Ready in time nor refunded in [t1, t2).
                let out = self.commit(s)?;
                if let Some(f) = out.failure() {
                    self.log(s, format!("punish-path commit_claim failed: {f}"));
                    return Ok(Next::At(now + RETRY_SECS));
                }
                self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "commit_claim".into(), hash: out.hash.clone() });
                #[cfg(feature = "misbehave")]
                let tag = if mis == Some(Misbehave::FreeClaim) { " [MISBEHAVE free-claim, never funded]" } else { "" };
                #[cfg(not(feature = "misbehave"))]
                let tag = "";
                self.log(s, format!("past t2 with escrow {:?}: LOCKED via the punish path, tx {}{tag}", v.state, out.hash));
                self.set_stage(s, TakerStage::Committed, now);
                Ok(Next::At(now + POLL_SECS))
            }
            #[cfg(feature = "misbehave")]
            SwapState::Open if mis == Some(Misbehave::FreeClaim) && tnow >= v.t0 && !s.log.iter().any(|l| l.contains("REJECTED")) => {
                // The attack, before t2: lock without Ready. The escrow must refuse it.
                let out = self.commit(s)?;
                match out.failure() {
                    Some(f) => {
                        let f = f.to_string();
                        self.log(s, format!("[MISBEHAVE free-claim] commit_claim without Ready before t2 REJECTED by the escrow: {f}; waiting for t2"));
                        Ok(Next::At(now + POLL_SECS))
                    }
                    None => bail!("free claim lock was ACCEPTED before t2 (tx {}): escrow invariant broken", out.hash),
                }
            }
            #[cfg(feature = "misbehave")]
            SwapState::Ready | SwapState::Open if mis == Some(Misbehave::LateCommit) && tnow >= v.t1 && !s.log.iter().any(|l| l.contains("REVERTED")) => {
                // Ignore the deadline on purpose: the late commit must fail and reveal nothing.
                let out = self.commit(s)?;
                match out.failure() {
                    Some(f) => {
                        let f = f.to_string();
                        self.log(s, format!("[MISBEHAVE late-commit] commit_claim after t1 REVERTED, publishing only a hash: {f}"));
                        Ok(Next::At(now + POLL_SECS))
                    }
                    None => bail!("late commit ACCEPTED (tx {}): escrow invariant broken", out.hash),
                }
            }
            SwapState::Ready | SwapState::Open if !t.in_time(tnow, v.t1) => {
                if !s.log.iter().any(|l| l.starts_with("R2c")) {
                    self.log(s, format!("R2c: claim window closed (NEAR time {tnow}, t1 {}, budget {}s); not committing; waiting for the maker's refund or for t2 {}", v.t1, t.budget(), v.t2));
                }
                Ok(Next::At(now + POLL_SECS))
            }
            _ => Ok(Next::At(now + POLL_SECS)),
        }
    }

    /// R2c holds (Ready is final and now + B_R < t1): lock the escrow with commit_claim.
    fn commit_ready(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        // R2c: Ready is final and now + B_R < t1.
        let out = self.commit(s)?;
        if let Some(f) = out.failure() {
            self.log(s, format!("commit_claim failed: {f}"));
            return Ok(Next::At(now + RETRY_SECS));
        }
        self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "commit_claim".into(), hash: out.hash.clone() });
        self.log(s, format!("escrow READY (final); LOCKED with commit_claim signed by the taker key (publishes only the commitment), tx {}", out.hash));
        self.set_stage(s, TakerStage::Committed, now);
        Ok(Next::At(now + POLL_SECS))
    }

    /// Signs claim(a) and stores it in the record (`claim_raw`, `claim_tx`). Nothing is sent:
    /// the broadcast comes on a later tick, after the record is durable (F2-02).
    fn sign_claim(&mut self, s: &mut TakerSwap) -> Result<()> {
        let id = s.id_bytes()?;
        let a = keys::scalar_to_bytes(&s.a_norm()?);
        let (to, actions) = self.esc().claim(&id, &a);
        let signer = self.signer(s)?;
        let tx = self.near.sign(&signer, &to, actions)?;
        s.claim_tx = Some(tx.hash_str());
        s.claim_raw = Some(hex::encode(&tx.bytes));
        Ok(())
    }

    /// Broadcasts the stored claim(a) and waits for its execution.
    fn broadcast_claim(&mut self, s: &mut TakerSwap, tnow: u64) -> Result<crate::near::TxOutcome> {
        let raw = hex::decode(s.claim_raw.as_deref().ok_or_else(|| anyhow!("no signed claim stored"))?)?;
        let hash_str = s.claim_tx.clone().ok_or_else(|| anyhow!("no claim hash stored"))?;
        let hash: [u8; 32] = bs58::decode(&hash_str).into_vec()?.try_into().map_err(|_| anyhow!("claim hash length"))?;
        let tx = crate::near::tx::SignedTx { bytes: raw, hash };
        s.claim_at = Some(tnow);
        self.near.wait_executed(&tx, &s.taker_account)
    }

    /// A claim(a) after the first: a new transaction (a is already public).
    fn resend_claim(&mut self, s: &mut TakerSwap, tnow: u64) -> Result<crate::near::TxOutcome> {
        let id = s.id_bytes()?;
        let a = keys::scalar_to_bytes(&s.a_norm()?);
        let (to, actions) = self.esc().claim(&id, &a);
        let signer = self.signer(s)?;
        let tx = self.near.sign(&signer, &to, actions)?;
        s.claim_tx = Some(tx.hash_str());
        s.claim_at = Some(tnow);
        self.near.wait_executed(&tx, &s.taker_account)
    }

    fn on_committed(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let id = s.id_bytes()?;
        let fin = self.near.block(&At::Final)?;
        let tnow = fin.secs();
        let v = self.esc().get_swap(&id, &At::Block(fin.hash.clone()))?.ok_or_else(|| anyhow!("escrow vanished"))?;
        match v.state {
            SwapState::Locked => {
                #[cfg(feature = "misbehave")]
                if self.cfg.misbehave == Some(crate::config::Misbehave::NoReveal) {
                    self.log_once(s, "[MISBEHAVE no-reveal] lock is final; not revealing a".into());
                    return Ok(Next::At(now + POLL_SECS));
                }
                // F2-03: saturating, so a huge W cannot wrap or panic.
                let deadline = v.locked_at.unwrap_or(0).saturating_add(v.reveal_window);
                // R2r: reveal only once the lock is final and now + B_R < lockedAt + W.
                if !self.cfg.timing.in_time(tnow, deadline) {
                    self.log_once(s, format!("R2r: reveal deadline {deadline} too close (NEAR time {tnow}); NOT revealing a; waiting for the maker's refund"));
                    return Ok(Next::At(now + POLL_SECS));
                }
                // The decision is stored (stage Revealed) before claim(a) is signed and
                // stored, and that before it goes out: a restart never forgets that a may
                // be public. R2r is checked again at the broadcast.
                self.log(s, format!("lock final (block {}); revealing a before the deadline {deadline}", fin.height));
                self.set_stage(s, TakerStage::Revealed, now);
                Ok(Next::At(0))
            }
            SwapState::Claimed => {
                self.set_stage(s, TakerStage::Claimed, now);
                Ok(Next::At(now))
            }
            SwapState::Refunded => self.recover(s, &v, now),
            SwapState::Open | SwapState::Ready => {
                let head = self.esc().get_swap(&id, &At::Optimistic)?.map(|h| h.state);
                if head == Some(SwapState::Locked) {
                    return Ok(Next::At(now + POLL_SECS)); // the lock is not final yet
                }
                self.log(s, "commit not on chain; back to committing".into());
                self.set_stage(s, TakerStage::Funded, now);
                Ok(Next::At(now + POLL_SECS))
            }
        }
    }

    /// The first claim(a) goes out in two steps (security review 2, F2-02): one tick signs
    /// it and stores it; a later tick broadcasts the stored bytes. R2r is checked again
    /// right before each step. If it fails before anything was sent (`claim_at` is None),
    /// the engine does not reveal: it goes back to `Committed`, and the maker's refund of
    /// the lock (R3l) gives b. After the first broadcast, stay until the claim is final,
    /// and send it again while the window is open.
    fn on_revealed(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let id = s.id_bytes()?;
        let fin = self.near.block(&At::Final)?;
        let tnow = fin.secs();
        let v = self.esc().get_swap(&id, &At::Block(fin.hash.clone()))?.ok_or_else(|| anyhow!("escrow vanished"))?;
        let deadline = v.locked_at.unwrap_or(0).saturating_add(v.reveal_window);
        match v.state {
            SwapState::Claimed => {
                self.log(s, format!("claim is final (block {})", fin.height));
                self.set_stage(s, TakerStage::Claimed, now);
                Ok(Next::At(now))
            }
            SwapState::Refunded => self.recover(s, &v, now),
            SwapState::Locked if s.claim_at.is_none() => {
                if !self.cfg.timing.in_time(tnow, deadline) {
                    s.claim_raw = None;
                    s.claim_tx = None;
                    self.log(s, format!("R2r at the hand-off: reveal deadline {deadline} too close (NEAR time {tnow}); claim(a) NOT sent; a stays secret; back to Committed, waiting for the maker's refund"));
                    self.set_stage(s, TakerStage::Committed, now);
                    return Ok(Next::At(now + POLL_SECS));
                }
                if s.claim_raw.is_none() {
                    self.sign_claim(s)?;
                    self.log(s, format!("claim(a) signed and stored, tx {}; it goes out on the next step", s.claim_tx.clone().unwrap_or_default()));
                    return Ok(Next::At(0));
                }
                let out = self.broadcast_claim(s, tnow)?;
                self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "claim".into(), hash: out.hash.clone() });
                match out.failure() {
                    Some(f) => self.log(s, format!("REVEALED a, but claim failed: {f}; will re-send while the window is open")),
                    None => self.log(s, format!("REVEALED a with claim, tx {}; waiting for the claim to be final", out.hash)),
                }
                Ok(Next::At(now + POLL_SECS))
            }
            SwapState::Locked => {
                let head = self.esc().get_swap(&id, &At::Optimistic)?.map(|h| h.state);
                let pending = s.claim_at.map(|t| tnow < t.saturating_add(self.cfg.timing.inclusion_secs)).unwrap_or(false);
                if head == Some(SwapState::Locked) && !pending && tnow < deadline {
                    let out = self.resend_claim(s, tnow)?;
                    self.log(s, format!("claim not on chain; RE-SENT claim(a), tx {} ({})", out.hash, out.failure().unwrap_or("ok")));
                }
                Ok(Next::At(now + POLL_SECS))
            }
            _ => Ok(Next::At(now + POLL_SECS)),
        }
    }

    /// The claim is final. The payout is one `mt_transfer` inside the Verifier, sent by
    /// `withdraw(id)` (25 Tgas). The taker's key sends it from the stipend as soon as a
    /// final block shows the payout Due, and again after a failed try. The maker may send
    /// the same call; the escrow starts at most one payout at a time and pays once. The
    /// swap completes when a final block shows the payout Paid. If it is not Paid after
    /// `payout_wait_secs`, the stage becomes `PayoutPending` (the engine keeps trying).
    fn on_claimed(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        // R4: the claim is final, so the taker's share and seeds are no longer needed.
        if !s.a.is_empty() {
            s.a.clear();
            s.s_t.clear();
            s.e_t.clear();
            s.claim_raw = None;
        }
        let id = s.id_bytes()?;
        let fin = self.near.block(&At::Final)?;
        let at = At::Block(fin.hash.clone());
        let esc = self.esc();
        let v = esc.get_swap(&id, &at)?.ok_or_else(|| anyhow!("escrow swap vanished"))?;
        if v.payout == escrow::Payout::Paid {
            let held = esc.intents_balance(&s.taker_account, &at)?;
            self.log(s, format!("PAYOUT: {} of {} paid into the Verifier, owned by {} (holds {held} now)", v.amount, self.cfg.chains.token, s.taker_account));
            self.ev.push(Event::Payout { swap: s.rfq_id.clone(), account: s.taker_account.clone(), token: self.cfg.chains.token.clone(), amount: v.amount.to_string() });
            self.set_stage(s, TakerStage::Completed, now);
            return Ok(Next::Done);
        }
        let t = &self.cfg.timing;
        let due = s.withdraw_at.map(|w| fin.secs() >= w.saturating_add(t.inclusion_secs).saturating_add(t.finality_secs)).unwrap_or(true);
        if v.payout == escrow::Payout::Due && due {
            #[cfg(feature = "misbehave")]
            let skip = self.cfg.misbehave == Some(Misbehave::NoWithdraw);
            #[cfg(not(feature = "misbehave"))]
            let skip = false;
            // NEAR accepts the call only if the key holds the attached gas at the
            // purchase price (plus about 1 Tgas of fees).
            let need = (escrow::GAS_WITHDRAW + crate::near::TGAS) as u128 * PURCHASE_GAS_PRICE;
            let have = self.near.account(&s.taker_account, &at)?.map(|a| a.amount).unwrap_or(0);
            s.withdraw_at = Some(fin.secs());
            if skip {
                self.log_once(s, "[MISBEHAVE no-withdraw] payout Due; not sending withdraw".into());
            } else if have >= need {
                let (to, actions) = esc.withdraw(&id);
                let signer = self.signer(s)?;
                match self.near.transact(&signer, &to, actions) {
                    Ok(out) => {
                        self.ev.push(Event::NearTx { swap: s.rfq_id.clone(), what: "withdraw".into(), hash: out.hash.clone() });
                        s.withdraw_tx = Some(out.hash.clone());
                        self.log(s, format!("withdraw(id) sent by the taker, tx {} ({})", out.hash, out.failure().unwrap_or("executed")));
                    }
                    Err(e) => self.log(s, format!("the taker could not send withdraw ({e:#}); trying again")),
                }
            } else {
                self.log_once(s, format!("the taker's key holds {have} yoctoNEAR, less than the {need} that withdraw needs; waiting for another account to send it"));
            }
        }
        if s.stage == TakerStage::Claimed && now >= s.stage_at.saturating_add(self.cfg.payout_wait_secs) {
            let args = serde_json::json!({"id": hex::encode(id)}).to_string();
            let gas = escrow::GAS_WITHDRAW;
            self.log(
                s,
                format!(
                    "claimed; payout pending: not paid after {}s (payout {:?}). The engine keeps sending withdraw. Any NEAR account can send it too: call {} withdraw {args} with {} Tgas and no deposit (the caller needs about {} NEAR free for the gas check). The tokens can only go to {}",
                    self.cfg.payout_wait_secs,
                    v.payout,
                    self.cfg.chains.escrow,
                    gas / crate::near::TGAS,
                    ((gas + crate::near::TGAS) as u128 * PURCHASE_GAS_PRICE) as f64 / crate::near::ONE_NEAR as f64,
                    s.taker_account
                ),
            );
            self.ev.push(Event::PayoutPending {
                swap: s.rfq_id.clone(),
                receiver: self.cfg.chains.escrow.clone(),
                method: "withdraw".into(),
                args,
                gas: gas.to_string(),
                deposit: "0".into(),
                credit: v.amount.to_string(),
            });
            self.set_stage(s, TakerStage::PayoutPending, now);
            return Ok(Next::At(now + POLL_SECS));
        }
        self.log_once(s, format!("claim final; waiting for the payout ({:?})", v.payout));
        Ok(Next::At(now + POLL_SECS))
    }

    /// Joint notes that still need a sweep: unspent, and worth more than the sweep fee
    /// (a smaller note cannot pay its own sweep). The taker's own funding comes first.
    fn joint_notes_to_sweep(&self, s: &TakerSwap) -> Vec<wallet::Received> {
        let fee = wallet::zip317_fee(1, 1);
        let mut v: Vec<wallet::Received> = self.wallet.unspent(&s.rfq_id).into_iter().filter(|n| n.note.value().inner() > fee).collect();
        v.sort_by_key(|n| (Some(&n.txid) != s.fund_txid.as_ref(), std::cmp::Reverse(n.note.value().inner())));
        v
    }

    /// R4: a funding this engine broadcast may still be mined while it is not visible and
    /// its expiry height + k_Z has not passed. `a` must be kept until then.
    fn funding_may_still_come(&self, s: &TakerSwap) -> Option<u64> {
        let (txid, exp) = (s.fund_txid.as_ref()?, s.fund_expiry?);
        let dead_after = exp.saturating_add(self.cfg.timing.k_z);
        (self.wallet.mined_height(txid).is_none() && self.wallet.height <= dead_after).then_some(dead_after)
    }

    /// The maker refunded: sweep every joint note above the fee back with a + b, the
    /// taker's own funding first, and keep `a` while a funding may still arrive (security
    /// review 2, F2-01). A note that the maker sent (H10) is swept too; whatever
    /// `fund_txid` names, the stage ends only when no note above the fee is left.
    fn recover(&mut self, s: &mut TakerSwap, v: &crate::near::SwapView, now: u64) -> Result<Next> {
        if self.zcash_stale {
            bail!("the maker refunded, but the Zcash view is old (sync failed); the recovery waits for a good sync");
        }
        let b = keys::scalar_from_hex(v.b.as_deref().ok_or_else(|| anyhow!("Refunded without b"))?)?;
        let q = s.quote.clone().ok_or_else(|| anyhow!("no quote"))?;
        let b_pub = keys::point_from_hex(&q.b_pub)?;
        let b_pub = if s.negated { -b_pub } else { b_pub };
        if keys::g() * b != b_pub {
            bail!("published b does not match B");
        }
        let a = s.a_norm()?;
        let Some(n) = self.joint_notes_to_sweep(s).into_iter().next() else {
            if let Some(dead_after) = self.funding_may_still_come(s) {
                self.log_once(s, format!("maker REFUNDED; our funding is not visible yet: keeping a until it is mined or expires (height > {dead_after})"));
                return Ok(Next::At(now + POLL_SECS));
            }
            let line = match (&s.fund_txid, s.fund_expiry) {
                (Some(t), _) if self.wallet.mined_height(t).is_some() => "maker REFUNDED; no joint note above the fee is left unspent".to_string(),
                (Some(_), Some(exp)) => format!("maker REFUNDED; our funding expired unmined (height > {}), nothing more to recover", exp.saturating_add(self.cfg.timing.k_z)),
                _ => "maker REFUNDED; no joint note above the fee, nothing to recover".to_string(),
            };
            self.log(s, line);
            self.set_stage(s, TakerStage::Recovered, now);
            return Ok(Next::Done);
        };
        let tip = self.wallet.height;
        if !self.cover_ready(s, "RECOVERY SWEEP", tip, None, now) {
            return Ok(Next::At(now + POLL_SECS));
        }
        if !self.announced(s, "sweep", "Building the proof that takes the ZEC back from the shared address") {
            return Ok(Next::At(0));
        }
        self.log(s, format!("maker REFUNDED: read b from the escrow; sweeping the joint note of {} zats from tx {}", n.note.value().inner(), n.txid));
        let tx = self.wallet.build_sweep(n, a + b, self.me.address())?;
        s.sweep_txid = Some(tx.txid.clone());
        s.sweep_height = None;
        Ok(self.pend(s, "sweep", &tx, TakerStage::Recovering))
    }

    /// Keep a (and the joint key) until each recovery sweep has k_Z confirmations, no
    /// joint note above the fee is left, and no funding of ours can still arrive (R4).
    fn on_recovering(&mut self, s: &mut TakerSwap, now: u64) -> Result<Next> {
        let k_z = self.cfg.timing.k_z;
        let tip = self.wallet.height;
        if let Some(h) = s.sweep_height {
            let conf = (tip + 1).saturating_sub(h);
            if conf < k_z {
                return Ok(Next::At(now + POLL_SECS));
            }
            let left = self.joint_notes_to_sweep(s);
            let waiting = self.funding_may_still_come(s);
            if left.is_empty() && waiting.is_none() {
                self.log(s, format!("recovery sweep {} has {conf} confirmations (k_Z = {k_z}); no joint note above the fee is left; done", s.sweep_txid.clone().unwrap_or_default()));
                self.set_stage(s, TakerStage::Recovered, now);
                return Ok(Next::Done);
            }
            // F2-01: another note is at the joint address (or our funding may still come).
            self.log(
                s,
                format!(
                    "recovery sweep {} has {conf} confirmations, but {} joint note(s) above the fee are left{}; keeping a and sweeping again",
                    s.sweep_txid.clone().unwrap_or_default(),
                    left.len(),
                    if waiting.is_some() { " and our funding is not mined yet" } else { "" }
                ),
            );
            s.sweep_height = None;
            s.sweep_txid = None;
        }
        if s.pending.is_none() {
            // No sweep in flight (the last one is confirmed or expired unmined): look again.
            let id = s.id_bytes()?;
            let v = self.esc().get_swap(&id, &At::Final)?.ok_or_else(|| anyhow!("escrow vanished"))?;
            return self.recover(s, &v, now);
        }
        Ok(Next::At(now + POLL_SECS))
    }

    /// Sends what is left in the temporary wallet to `to` (a unified address), minus the
    /// fee. Use it after a swap ends. Returns the txid.
    pub fn return_leftover(&mut self, to: &str) -> Result<String> {
        self.wallet.sync()?;
        let to = wallet::decode_ua(to)?;
        let have = self.balance();
        let fee = wallet::zip317_fee(1, 2);
        if have <= fee {
            bail!("nothing to return: {have} zats");
        }
        self.wallet.pay("wallet", &self.me.ask, self.me.change(), vec![(to, have - fee, wallet::empty_memo())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Chains, Cover, MakerEntry, Timing},
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

    fn setup() -> (Taker, NearKey) {
        let maker_key = NearKey::from_seed([3u8; 32]);
        let rfq_pub = hex::encode(msg::dh_public(&keys::random_scalar()));
        let ua = Me::generate().ua();
        let digest = keys::directory_digest(&ua, 1, &keys::unhex32(&rfq_pub).unwrap(), "maker.near");
        let cfg = TakerConfig {
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
            timing: Timing { inclusion_secs: 30, finality_secs: 10, reorg_budget: 2, margin_secs: 0, fund_secs: 120, k_z: 2 },
            maker: MakerEntry { ua, epoch: 1, rfq_pub, near_account: "maker.near".into(), near_pk: maker_key.public_key_str(), sig: hex::encode(maker_key.sign(&digest)), limits: None },
            escrow_code_hash: "unused".into(),
            min_price: 1000,
            max_premium: 50_000,
            min_stipend: 3 * crate::near::ONE_NEAR / 100,
            max_horizon_secs: 48 * 3600,
            quote_timeout_secs: 600,
            cover: Cover::default(),
            misbehave: None,
            payout_wait_secs: 300,
        };
        let mut store = MemStore::new();
        (Taker::open(cfg, Rc::new(NoNet), &mut store).unwrap(), maker_key)
    }

    fn good_quote(rfq_id: &[u8; 16], key: &NearKey) -> Quote {
        let b = keys::random_scalar();
        let mut q = Quote {
            price: 5000,
            premium: 40_000,
            stipend: 3 * crate::near::ONE_NEAR / 100,
            b_pub: (keys::g() * b).to_bytes(),
            pok_b: keys::pok_prove(&b, rfq_id, b"maker"),
            s_m: [1; 32],
            t0_secs: 600,
            t1_secs: 600,
            t2_secs: 600,
            reveal_secs: 600,
            escrow: "escrow.near".into(),
            maker: "maker.near".into(),
            sig: [0; 64],
        };
        q.sig = key.sign(&q.sig_digest(rfq_id));
        q
    }

    #[test]
    fn quote_rules_r1q_and_identity() {
        let (tk, key) = setup();
        let id = [9u8; 16];
        assert_eq!(tk.reject_quote(&good_quote(&id, &key), &id, 1000), None);
        let resign = |mut q: Quote| {
            q.sig = key.sign(&q.sig_digest(&id));
            q
        };
        let cases: Vec<(Quote, &str)> = vec![
            // R1q: W must exceed F + B_R = 10 + 110
            (resign(Quote { reveal_secs: 120, ..good_quote(&id, &key) }), "reveal window"),
            // R1q: t2 - open must stay within the horizon
            (resign(Quote { t2_secs: u32::MAX, ..good_quote(&id, &key) }), "deadline horizon"),
            (resign(Quote { t0_secs: 130, ..good_quote(&id, &key) }), "funding window"),
            (resign(Quote { t1_secs: 120, ..good_quote(&id, &key) }), "claim window"),
            (resign(Quote { price: 999, ..good_quote(&id, &key) }), "below the taker's minimum"),
            (resign(Quote { premium: 50_001, ..good_quote(&id, &key) }), "premium"),
            (resign(Quote { stipend: 1, ..good_quote(&id, &key) }), "stipend"),
            (resign(Quote { escrow: "evil.near".into(), ..good_quote(&id, &key) }), "unknown escrow"),
            (resign(Quote { maker: "evil.near".into(), ..good_quote(&id, &key) }), "unexpected maker"),
            // a quote signed by another key
            (Quote { sig: NearKey::generate().sign(&good_quote(&id, &key).sig_digest(&id)), ..good_quote(&id, &key) }, "not signed"),
            // a proof of knowledge for another request id (replay)
            (resign(Quote { pok_b: good_quote(&[8u8; 16], &key).pok_b, ..good_quote(&id, &key) }), "proof of knowledge"),
        ];
        for (q, want) in cases {
            let got = tk.reject_quote(&q, &id, 1000).unwrap_or_default();
            assert!(got.contains(want), "expected {want:?}, got {got:?}");
        }
    }

    #[test]
    fn start_refuses_a_forged_directory_entry_before_any_network_call() {
        let (mut tk, _) = setup();
        tk.cfg.maker.sig = hex::encode(NearKey::generate().sign(b"x"));
        let mut store = MemStore::new();
        let e = tk.start(1000, 0, &mut store).unwrap_err().to_string();
        assert!(e.contains("not signed by"), "{e}");
        assert!(store.keys(PREFIX_SWAP).unwrap().is_empty(), "nothing stored");
        assert!(tk.swap().is_none());
    }

    /// H1: a configured maker key that does not parse rejects every quote.
    #[test]
    fn quote_check_fails_closed_on_a_bad_maker_key() {
        let (mut tk, key) = setup();
        let id = [9u8; 16];
        let q = good_quote(&id, &key);
        assert_eq!(tk.reject_quote(&q, &id, 1000), None);
        tk.cfg.maker.near_pk = "ed25519:not-a-key".into();
        let why = tk.reject_quote(&q, &id, 1000).expect("must reject");
        assert!(why.contains("does not parse"), "{why}");
    }

    /// H6: a request built again has the same proof of knowledge, so the same plaintext
    /// under the same AEAD key and nonce; other inputs give other proofs.
    #[test]
    fn rebuilt_request_has_the_same_plaintext() {
        let a = keys::random_scalar();
        let id = [7u8; 16];
        let (p1, p2) = (keys::pok_prove(&a, &id, b"taker"), keys::pok_prove(&a, &id, b"taker"));
        assert_eq!(p1, p2);
        assert!(keys::pok_verify(&(keys::g() * a), &p1, &id, b"taker"));
        assert_ne!(p1, keys::pok_prove(&a, &[8u8; 16], b"taker"));
        assert_ne!(p1, keys::pok_prove(&a, &id, b"maker"));
        assert_ne!(p1[..32], keys::pok_prove(&keys::random_scalar(), &id, b"taker")[..32], "nonce depends on the secret");
    }

    #[test]
    fn deposit_covers_note_fees_and_premium() {
        assert_eq!(required_deposit(100_000_000, 50_000), 100_000_000 + 10_000 + 30_000 + 50_000);
    }
}
