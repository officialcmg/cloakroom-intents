//! A minimal Ironwood light wallet over Zebra JSON-RPC or lightwalletd (gRPC-web).
//!
//! From the Cloakroom client (`node/src/wallet.rs` at the `pre-hackathon` tag), with the
//! browser changes of `spike/browser-wallet` (listed in docs/experiments/browser-wallet.md):
//! no sockets, no sleeps and no clock inside; I/O goes through `net::Transport`. New here:
//! a transaction is built ([`Wallet::build`]) and broadcast ([`Wallet::broadcast`]) in two
//! steps, so a caller can store it in between; mined transaction ids are tracked; notes
//! spent by a transaction that is not mined yet are reserved.
//!
//! It scans every block, keeps the full Ironwood note commitment tree (fine for regtest
//! and small test chains), trial-decrypts with any number of viewing keys (the wallet's
//! own and per-swap joint keys), and builds v6 transactions through the PCZT roles.

use anyhow::{anyhow, bail, Context, Result};
use incrementalmerkletree::{Position, Retention};
use orchard::{
    circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey},
    keys::{FullViewingKey, OutgoingViewingKey, PreparedIncomingViewingKey, Scope, SpendAuthorizingKey},
    note_encryption::IronwoodDomain,
    tree::MerkleHashOrchard,
    Address, Note,
};
use pasta_curves::pallas;
use pczt::roles::{
    creator::Creator, io_finalizer::IoFinalizer, low_level_signer, prover::Prover, signer::Signer,
    tx_extractor::TransactionExtractor,
};
use serde_json::json;
use shardtree::{store::memory::MemoryShardStore, ShardTree};
use std::{collections::HashSet, convert::Infallible, sync::OnceLock};

use crate::zcash::{Lwd, Rpc};
use zcash_address::unified::{self, Container, Encoding};
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::{
    builder::{BuildConfig, Builder, BundlePadding},
    fees::zip317,
    Transaction,
};
use incrementalmerkletree::frontier::Frontier;
use zcash_note_encryption::try_compact_note_decryption;
use zcash_protocol::{
    consensus::{BlockHeight, BranchId, NetworkType, NetworkUpgrade, Parameters, MAIN_NETWORK, TEST_NETWORK},
    local_consensus::LocalNetwork,
    memo::MemoBytes,
    value::Zatoshis,
};

use crate::{keys, OsRng};

pub const MARGINAL_FEE: u64 = 5_000;
pub const GRACE_ACTIONS: u64 = 2;

pub fn zip317_fee(spends: usize, outputs: usize) -> u64 {
    MARGINAL_FEE * GRACE_ACTIONS.max(spends.max(outputs) as u64)
}

// ---------------------------------------------------------------- network + addresses

pub fn regtest_params(nu7: Option<u32>) -> LocalNetwork {
    let one = Some(BlockHeight::from_u32(1));
    LocalNetwork {
        overwinter: one,
        sapling: one,
        blossom: one,
        heartwood: one,
        canopy: one,
        nu5: one,
        nu6: one,
        nu6_1: one,
        nu6_2: one,
        nu6_3: one,
        nu7: nu7.map(BlockHeight::from_u32),
    }
}

/// Network used for address encoding: regtest unless set to testnet/mainnet at startup.
pub static NET: OnceLock<NetworkType> = OnceLock::new();

pub fn set_network(name: &str) -> Result<()> {
    let n = match name {
        "regtest" => NetworkType::Regtest,
        "test" | "testnet" => NetworkType::Test,
        "main" | "mainnet" => NetworkType::Main,
        _ => bail!("unknown network {name}"),
    };
    let _ = NET.set(n);
    Ok(())
}

pub fn encode_ua(addr: &Address) -> String {
    unified::Address::try_from_items(
        zcash_protocol::address::Revision::R0,
        vec![unified::Uitem::Data(unified::Receiver::Orchard(addr.to_raw_address_bytes()))],
    )
    .expect("valid UA")
    .encode(NET.get().unwrap_or(&NetworkType::Regtest))
}

pub fn decode_ua(s: &str) -> Result<Address> {
    let (_net, _rev, ua) = unified::Address::decode(s).map_err(|e| anyhow!("bad UA: {e:?}"))?;
    for item in ua.items() {
        if let unified::Receiver::Orchard(raw) = item {
            return Option::from(Address::from_raw_address_bytes(&raw)).ok_or_else(|| anyhow!("bad Orchard receiver"));
        }
    }
    bail!("UA has no Orchard-protocol receiver")
}

pub fn addr_from_raw(raw: &[u8; 43]) -> Result<Address> {
    Option::from(Address::from_raw_address_bytes(raw)).ok_or_else(|| anyhow!("bad raw address"))
}

/// NU7 on Zcash mainnet. The zcash_protocol release in use (0.11.0-pre.0) has no mainnet
/// height for NU7, so the wallet sets it. ZIP 218 names 3,543,405, and the chain agrees: the
/// coinbase transaction of block 3,543,404 carries consensus branch id 0x37a5165b (NU6.3)
/// and that of block 3,543,405 carries 0x77190ad9 (NU7). Read on 2026-11-14 through
/// `zcash-mainnet.chainsafe.dev` (GetBlockRange, GetTransaction); its GetLightdInfo also
/// reported branch id 77190ad9 at the tip. Remove this when zcash_protocol has the height.
pub const MAINNET_NU7_HEIGHT: u32 = 3_543_405;

/// Consensus parameters: a local regtest chain, the public testnet or mainnet.
#[derive(Clone, Copy, Debug)]
pub enum Net {
    Local(LocalNetwork),
    Test,
    Main,
}

impl Parameters for Net {
    fn network_type(&self) -> NetworkType {
        match self {
            Net::Local(p) => p.network_type(),
            Net::Test => NetworkType::Test,
            Net::Main => NetworkType::Main,
        }
    }
    fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
        match self {
            Net::Local(p) => p.activation_height(nu),
            Net::Test => TEST_NETWORK.activation_height(nu),
            Net::Main => match nu {
                NetworkUpgrade::Nu7 => MAIN_NETWORK.activation_height(nu).or(Some(BlockHeight::from_u32(MAINNET_NU7_HEIGHT))),
                _ => MAIN_NETWORK.activation_height(nu),
            },
        }
    }
}

pub enum Backend {
    Zebra(Rpc),
    Lwd(Lwd),
}

// ---------------------------------------------------------------- circuit keys

static PK: OnceLock<ProvingKey> = OnceLock::new();
static VK: OnceLock<VerifyingKey> = OnceLock::new();
fn pk() -> &'static ProvingKey {
    PK.get_or_init(|| ProvingKey::build(OrchardCircuitVersion::PostNu6_3))
}
/// Taken from the proving key: `VerifyingKey::build` would run keygen_vk a second time.
fn vk() -> &'static VerifyingKey {
    VK.get_or_init(|| pk().verifying_key())
}

/// Build the proving key now (it is otherwise built on the first send). About 20 s in a
/// browser; start it early.
pub fn prepare_proving_key() {
    let _ = vk();
}

/// Expiry delta of the transactions the builder makes (the zcash_primitives default): a
/// transaction not mined by `target height + EXPIRY_DELTA` can never be mined.
pub const EXPIRY_DELTA: u64 = 40;

/// A node's answer to a broadcast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broadcast {
    Accepted,
    Refused(String),
}

impl Broadcast {
    pub fn ok(self) -> Result<()> {
        match self {
            Broadcast::Accepted => Ok(()),
            Broadcast::Refused(e) => bail!("transaction refused: {e}"),
        }
    }
}

/// A built, signed transaction that is not broadcast yet.
#[derive(Clone, Debug)]
pub struct BuiltTx {
    pub txid: String,
    pub raw: Vec<u8>,
    /// The last height at which it can be mined.
    pub expiry_height: u64,
}

// ---------------------------------------------------------------- scanning

struct KeyEntry {
    label: String,
    fvk: FullViewingKey,
    ivks: Vec<(Scope, PreparedIncomingViewingKey)>,
}

#[derive(Clone, Debug)]
pub struct Received {
    pub label: String,
    pub note: Note,
    pub pos: u64,
    pub height: u64,
    pub txid: String,
    pub memo: [u8; 512],
    pub nf: [u8; 32],
    pub spent_in: Option<String>,
}

pub enum SignWith {
    Key(SpendAuthorizingKey),
    /// Joint key: ask = a + b (already normalized).
    Joint(pallas::Scalar),
}

pub struct Wallet {
    pub backend: Backend,
    pub params: Net,
    /// Ironwood tree frontier just before the first scanned block (light-wallet mode).
    frontier: Frontier<MerkleHashOrchard, 32>,
    /// Global position of leaves[0].
    leaf_offset: u64,
    keys: Vec<KeyEntry>,
    leaves: Vec<MerkleHashOrchard>,
    pub received: Vec<Received>,
    /// nullifier -> (spending txid, block height)
    spent: std::collections::HashMap<[u8; 32], (String, u64)>,
    pub height: u64,
    /// Per block: number of "look-alike" transactions, i.e. fully shielded, Ironwood-only,
    /// exactly 2 actions -- the same shape as every Cloakroom transaction.
    pub cover: std::collections::BTreeMap<u64, u32>,
    /// Transaction ids to look for while scanning, and the height they were mined at.
    tracked: std::collections::HashMap<String, Option<u64>>,
    /// Nullifiers spent by built transactions that are not mined yet -> (expiry height,
    /// txid of the transaction that spends them).
    reserved: std::collections::HashMap<[u8; 32], (u64, String)>,
    /// The last scanned blocks, oldest first (at most `KEEP_BLOCKS`), with the first one
    /// possibly the block just before the birthday. They let the wallet check that each
    /// new block links to the one before, and roll back to a common block after a chain
    /// reorganisation.
    marks: std::collections::VecDeque<Mark>,
    /// Transactions a node accepted from this wallet.
    accepted: HashSet<String>,
    /// Lines for the engine's log (rollbacks, rescans, repairs); see [`Wallet::take_notes`].
    notes: Vec<String>,
    /// Tip height at the last rescan made by [`Wallet::repair`] (at most one per height).
    repaired_at: Option<u64>,
}

/// Blocks the wallet keeps for rollbacks. A reorganisation deeper than this makes it
/// rescan from the birthday.
pub const KEEP_BLOCKS: usize = 100;

/// The anchor of a built transaction is the tree of the block this many blocks below the
/// tip, or of the newest block that holds a spent note if that is higher. A block at the
/// tip is the one a reorganisation replaces most often.
pub const ANCHOR_DEPTH: u64 = 2;

/// One scanned block: its height, hash, and the number of leaves in `Wallet::leaves`
/// after it.
#[derive(Clone, Debug)]
struct Mark {
    height: u64,
    hash: Vec<u8>,
    leaves: usize,
}

/// Everything one block adds to the wallet. It is built first (with every network call
/// the block needs) and then applied at once, so a block is applied fully or not at all.
struct BlockDelta {
    height: u64,
    hash: Vec<u8>,
    leaves: Vec<MerkleHashOrchard>,
    received: Vec<Received>,
    spent: Vec<([u8; 32], String)>,
    lookalikes: u32,
    mined: Vec<String>,
    /// The Ironwood tree size after this block, when the server says it.
    tree_size: Option<u64>,
}

/// The scan state, kept aside while a repair rescans, and put back if the rescan fails.
struct Snapshot {
    frontier: Frontier<MerkleHashOrchard, 32>,
    leaf_offset: u64,
    leaves: Vec<MerkleHashOrchard>,
    received: Vec<Received>,
    spent: std::collections::HashMap<[u8; 32], (String, u64)>,
    height: u64,
    cover: std::collections::BTreeMap<u64, u32>,
    tracked: std::collections::HashMap<String, Option<u64>>,
    marks: std::collections::VecDeque<Mark>,
}

/// What the wallet saw when it compared a block with the chain it has scanned.
enum Link {
    Ok,
    /// The block does not link to the wallet's last block: the chain was reorganised.
    Fork,
}

/// Cover-traffic policy for a broadcast: wait U[min_secs, max_secs] AND until at least `k`
/// look-alike transactions have been mined since the trigger, but never past `deadline`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoverPolicy {
    pub min_secs: u64,
    pub max_secs: u64,
    pub k: u64,
}

/// Cover delay before a broadcast that follows a public trigger ("redraw" policy). Wait a fresh
/// U[min_secs, max_secs]; if fewer than `k` look-alikes have been mined since the trigger when
/// the wait ends, draw a new U and wait again. The broadcast therefore always happens at the
/// end of a random wait, never right after the k-th look-alike: snapping to the k-th arrival
/// lets a policy-aware observer guess the transaction with median probability about 1/3
/// (measurements/cover_posterior.py). Never waits past `deadline_unix`.
///
/// In `node/` this was a loop with `std::thread::sleep(2 s)` inside the wallet. That cannot
/// run in a browser (wasm32 has no thread sleep), so it is now a poll-style state machine:
/// the caller syncs the wallet, calls `poll` with the current time, and sleeps however its
/// platform sleeps (a timer in the browser, a thread sleep natively) until `poll` says done.
pub struct CoverWait {
    policy: CoverPolicy,
    trigger_height: u64,
    deadline_unix: Option<u64>,
    start: u64,
    wait_from: u64,
    delay: u64,
    draws: u32,
}

impl CoverWait {
    pub fn new(policy: CoverPolicy, trigger_height: u64, deadline_unix: Option<u64>, now_unix: u64) -> Self {
        let mut c = CoverWait { policy, trigger_height, deadline_unix, start: now_unix, wait_from: now_unix, delay: 0, draws: 1 };
        c.delay = c.draw();
        c
    }

    fn draw(&self) -> u64 {
        let span = self.policy.max_secs.saturating_sub(self.policy.min_secs);
        self.policy.min_secs + if span == 0 { 0 } else { (rand::Rng::next_u64(&mut { crate::OsRng })) % (span + 1) }
    }

    /// `Some(outcome)` when the broadcast may go now; `None` to wait and poll again.
    pub fn poll(&mut self, w: &Wallet, now: u64) -> Option<String> {
        let policy = self.policy;
        if policy.max_secs == 0 && policy.k == 0 {
            return Some("no cover delay configured".into());
        }
        let seen = w.lookalikes_since(self.trigger_height);
        if let Some(d) = self.deadline_unix
            && now >= d {
                return Some(format!("cover cut short by the protocol deadline after {}s ({} draw(s)): {seen} of {} look-alikes", now - self.start, self.draws, policy.k));
            }
        if now >= self.wait_from + self.delay {
            if seen >= policy.k {
                return Some(format!("cover: waited {}s over {} random draw(s), {seen} look-alike txs mined since height {}", now - self.start, self.draws, self.trigger_height));
            }
            // not enough cover yet: redraw instead of waiting for the k-th arrival
            self.wait_from = now;
            self.delay = self.draw().max(1);
            self.draws += 1;
        }
        None
    }
}

impl Wallet {
    pub fn new(backend: Backend, params: Net) -> Self {
        Wallet {
            backend,
            params,
            frontier: Frontier::empty(),
            leaf_offset: 0,
            keys: vec![],
            leaves: vec![],
            received: vec![],
            spent: Default::default(),
            height: 0,
            cover: Default::default(),
            tracked: Default::default(),
            reserved: Default::default(),
            marks: Default::default(),
            accepted: Default::default(),
            notes: vec![],
            repaired_at: None,
        }
    }

    /// Ironwood actions scanned so far (leaves of the note commitment tree kept in memory).
    pub fn actions_scanned(&self) -> usize {
        self.leaves.len()
    }

    /// Look-alike transactions mined in blocks (after, tip].
    pub fn lookalikes_since(&self, after: u64) -> u64 {
        self.cover.range(after + 1..).map(|(_, n)| *n as u64).sum()
    }

    /// Lines for the log: rollbacks after a reorganisation, rescans and repairs since the
    /// last call. The maker and the taker put them in their events.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Track a viewing key. Adding a key resets the scan so its history is found.
    pub fn watch(&mut self, label: &str, fvk: &FullViewingKey, scopes: &[Scope]) {
        if self.keys.iter().any(|k| k.label == label) {
            return;
        }
        let ivks = scopes.iter().map(|s| (*s, PreparedIncomingViewingKey::new(&fvk.to_ivk(*s)))).collect();
        self.keys.push(KeyEntry { label: label.to_string(), fvk: fvk.clone(), ivks });
        self.reset_scan();
    }

    /// Forget everything scanned: the next sync scans from the birthday (or genesis).
    fn reset_scan(&mut self) {
        self.leaves.clear();
        self.received.clear();
        self.spent.clear();
        self.height = 0;
        self.frontier = Frontier::empty();
        self.leaf_offset = 0;
        self.cover.clear();
        self.marks.clear();
        for h in self.tracked.values_mut() {
            *h = None;
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            frontier: self.frontier.clone(),
            leaf_offset: self.leaf_offset,
            leaves: self.leaves.clone(),
            received: self.received.clone(),
            spent: self.spent.clone(),
            height: self.height,
            cover: self.cover.clone(),
            tracked: self.tracked.clone(),
            marks: self.marks.clone(),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.frontier = s.frontier;
        self.leaf_offset = s.leaf_offset;
        self.leaves = s.leaves;
        self.received = s.received;
        self.spent = s.spent;
        self.height = s.height;
        self.cover = s.cover;
        self.tracked = s.tracked;
        self.marks = s.marks;
        self.after_scan();
    }

    /// Look for this transaction id while scanning (from the next rescan or new blocks on).
    /// A transaction mined before the current scan height is found only if it was tracked
    /// before that block was scanned; call `track` before broadcasting.
    pub fn track(&mut self, txid: &str) {
        self.tracked.entry(txid.to_string()).or_insert(None);
    }

    /// The height a tracked transaction was mined at, if it was.
    pub fn mined_height(&self, txid: &str) -> Option<u64> {
        self.tracked.get(txid).copied().flatten()
    }

    pub fn tip(&mut self) -> Result<u64> {
        match &mut self.backend {
            Backend::Zebra(rpc) => Ok(rpc.call("getblockcount", json!([]))?.as_u64().unwrap_or(0)),
            Backend::Lwd(l) => l.tip(),
        }
    }

    /// Broadcast a built transaction. `Err` means no answer from the node (retry);
    /// `Refused` is the node's answer. Sending the same transaction again is harmless: a
    /// node that already has it answers with an error that counts as `Accepted`.
    ///
    /// A refusal of a transaction that no node took before frees the notes it spends. A
    /// refusal for an unknown anchor also starts [`Wallet::repair`]: the wallet's tree no
    /// longer matches the node's chain, and the next build must not use it.
    pub fn broadcast(&mut self, tx: &BuiltTx) -> Result<Broadcast> {
        self.track(&tx.txid);
        let r = match &mut self.backend {
            Backend::Zebra(rpc) => rpc.call_raw("sendrawtransaction", json!([hex::encode(&tx.raw)]))?.map(|_| ()).map_err(|e| e.to_string()),
            Backend::Lwd(l) => l.send(tx.raw.clone())?,
        };
        Ok(match r {
            Ok(()) => {
                self.accepted.insert(tx.txid.clone());
                Broadcast::Accepted
            }
            Err(e) if is_already_known(&e) => {
                self.accepted.insert(tx.txid.clone());
                Broadcast::Accepted
            }
            Err(e) => {
                if !self.accepted.contains(&tx.txid) {
                    self.reserved.retain(|_, (_, txid)| *txid != tx.txid);
                }
                if is_unknown_anchor(&e) {
                    self.notes.push(format!("the node refused transaction {} for its anchor; checking the wallet's tree against the chain", short_id(&tx.txid)));
                    if let Err(err) = self.repair() {
                        self.notes.push(format!("wallet repair failed, will try again at the next refusal: {err:#}"));
                    }
                }
                Broadcast::Refused(e)
            }
        })
    }

    /// Checks the wallet's tree against the chain after a node refused its anchor, and
    /// rebuilds it if they differ:
    ///
    /// 1. Sync. This also finds a reorganisation whose new blocks are already there.
    /// 2. Compare the recent block hashes with the server's, and roll back to the newest
    ///    common block (a reorganisation that replaced the tip at the same height).
    /// 3. Compare the tree root with the server's tree at the same height (light-wallet
    ///    server). If they still differ (or the backend cannot tell, Zebra), rescan from
    ///    the birthday, at most once per tip height. The old state comes back if the
    ///    rescan fails.
    ///
    /// Ok(true) when the wallet changed its tree.
    pub fn repair(&mut self) -> Result<bool> {
        let before = (self.height, self.leaves.len());
        self.sync()?;
        let tip = self.height;
        if let Some(fork) = self.find_fork(tip)?
            && fork < tip
        {
            self.rollback_to(fork, "the server's recent blocks differ from the scanned ones");
            self.sync()?;
        }
        let healthy = match &self.backend {
            Backend::Lwd(l) => {
                let server = l.tree_before(self.height)?;
                let ours = self.root_after(self.leaves.len());
                let theirs = frontier_root(&server);
                if ours == theirs {
                    self.notes.push(format!("the wallet's tree at {} matches the server's (root {}); the node may not have that block yet", self.height, hex::encode(&theirs[..8])));
                }
                ours == theirs
            }
            Backend::Zebra(_) => false,
        };
        if !healthy {
            if self.repaired_at == Some(self.height) {
                self.notes.push(format!("the wallet already rescanned at height {}; not again before a new block", self.height));
                return Ok((self.height, self.leaves.len()) != before);
            }
            self.repaired_at = Some(self.height);
            let saved = self.snapshot();
            self.notes.push(format!("rescanning from the birthday to rebuild the tree (was at height {})", self.height));
            self.reset_scan();
            if let Err(e) = self.sync() {
                self.restore(saved);
                return Err(e.context("rescan"));
            }
            self.notes.push(format!("rescan done at height {}", self.height));
            return Ok(true);
        }
        Ok((self.height, self.leaves.len()) != before)
    }

    /// The root of the wallet's tree after its first `n` leaves.
    fn root_after(&self, n: usize) -> [u8; 32] {
        let mut f = self.frontier.clone();
        for leaf in &self.leaves[..n] {
            f.append(*leaf);
        }
        frontier_root(&f)
    }

    /// Height of the newest block the wallet scanned that is also in the server's chain,
    /// looking at the kept blocks up to `upto`. `None` when no kept block matches (or none
    /// is kept): only a rescan helps then.
    fn find_fork(&mut self, upto: u64) -> Result<Option<u64>> {
        let marks: Vec<Mark> = self.marks.iter().filter(|m| m.height <= upto).cloned().collect();
        let Some(first) = marks.first() else { return Ok(None) };
        let server: std::collections::HashMap<u64, Vec<u8>> = match &self.backend {
            Backend::Lwd(l) => {
                let mut map = std::collections::HashMap::new();
                // the first kept block may be the one before the birthday: its hash is
                // the prev_hash of the next one
                let blocks = l.blocks(first.height.max(1), upto)?;
                for b in &blocks {
                    map.insert(b.height, b.hash.clone());
                    if b.height > 0 && !b.prev_hash.is_empty() {
                        map.entry(b.height - 1).or_insert_with(|| b.prev_hash.clone());
                    }
                }
                map
            }
            Backend::Zebra(rpc) => {
                let mut map = std::collections::HashMap::new();
                for m in marks.iter().rev() {
                    let h = rpc.call("getblockhash", json!([m.height]))?;
                    let hash = hex::decode(h.as_str().unwrap_or_default()).unwrap_or_default();
                    let same = same_hash(&hash, &m.hash);
                    map.insert(m.height, hash);
                    if same {
                        break; // older blocks are below a common block
                    }
                }
                map
            }
        };
        Ok(marks.iter().rev().find(|m| server.get(&m.height).is_some_and(|h| same_hash(h, &m.hash))).map(|m| m.height))
    }

    /// Undo every block above `height` (a kept block): leaves, received notes, spends,
    /// look-alikes and mined heights.
    fn rollback_to(&mut self, height: u64, why: &str) {
        let Some(m) = self.marks.iter().find(|m| m.height == height).cloned() else {
            self.notes.push(format!("{why}; block {height} is not kept, rescanning from the birthday"));
            self.reset_scan();
            return;
        };
        let from = self.height;
        self.leaves.truncate(m.leaves);
        self.received.retain(|r| r.height <= height);
        self.spent.retain(|_, (_, h)| *h <= height);
        self.cover.retain(|h, _| *h <= height);
        for h in self.tracked.values_mut() {
            if h.is_some_and(|h| h > height) {
                *h = None;
            }
        }
        self.marks.retain(|k| k.height <= height);
        self.height = height;
        self.after_scan();
        self.notes.push(format!("{why}; rolled back from height {from} to {height} ({} block(s))", from - height));
    }

    /// Checks that a block at `self.height + 1` with this parent hash links to the last
    /// scanned block.
    fn link(&self, height: u64, prev_hash: &[u8]) -> Result<Link> {
        if height != self.height + 1 {
            bail!("the server sent block {height} where block {} was expected", self.height + 1);
        }
        match self.marks.back() {
            Some(m) if m.height == self.height && !m.hash.is_empty() && !prev_hash.is_empty() && !same_hash(&m.hash, prev_hash) => Ok(Link::Fork),
            _ => Ok(Link::Ok),
        }
    }

    /// Before a block whose tree size (from the chain) does not fit the wallet's tree:
    /// compare the wallet's tree size with the server's tree at the wallet's height. If
    /// they differ, the wallet's tree is wrong: it is reset to rescan from the birthday
    /// (Ok(false)). If they agree, the block itself is incomplete; [`Wallet::apply`] then
    /// refuses it.
    fn size_fits(&mut self, d: &BlockDelta) -> Result<bool> {
        let ours = self.leaf_offset + self.leaves.len() as u64;
        let Some(want) = d.tree_size else { return Ok(true) };
        if want == ours + d.leaves.len() as u64 {
            return Ok(true);
        }
        let server = match &self.backend {
            Backend::Lwd(l) => l.tree_before(self.height)?.value().map(|f| u64::from(f.position()) + 1).unwrap_or(0),
            Backend::Zebra(_) => return Ok(true),
        };
        if server != ours {
            self.notes.push(format!("the wallet's tree holds {ours} leaves at height {}, the server's {server}; rescanning from the birthday", self.height));
            self.reset_scan();
            return Ok(false);
        }
        Ok(true)
    }

    /// Applies one block whose every part is known. Checks the tree size first, so a block
    /// with missing or extra actions is never applied.
    fn apply(&mut self, d: BlockDelta) -> Result<()> {
        let size = self.leaf_offset + (self.leaves.len() + d.leaves.len()) as u64;
        if let Some(want) = d.tree_size
            && want != size {
                bail!("block {}: the Ironwood tree would hold {size} leaves, the chain says {want}; block not applied", d.height);
            }
        self.leaves.extend(d.leaves);
        self.received.extend(d.received);
        for (nf, txid) in d.spent {
            self.spent.insert(nf, (txid, d.height));
        }
        if d.lookalikes > 0 {
            *self.cover.entry(d.height).or_default() += d.lookalikes;
        }
        for txid in d.mined {
            if let Some(slot) = self.tracked.get_mut(&txid) {
                *slot = Some(d.height);
            }
        }
        self.marks.push_back(Mark { height: d.height, hash: d.hash, leaves: self.leaves.len() });
        while self.marks.len() > KEEP_BLOCKS + 1 {
            self.marks.pop_front();
        }
        self.height = d.height;
        Ok(())
    }

    /// On a block that does not link: find the newest common block and roll back to it
    /// (or rescan if none is kept).
    fn on_fork(&mut self, at: u64) -> Result<()> {
        let why = format!("block {at} does not link to the scanned block {} (chain reorganisation)", at - 1);
        match self.find_fork(self.height)? {
            Some(h) => self.rollback_to(h, &why),
            None => {
                self.notes.push(format!("{why}; no kept block is in the server's chain, rescanning from the birthday"));
                self.reset_scan();
            }
        }
        Ok(())
    }

    /// When the server's tip is at or below the wallet's height, checks that the wallet's
    /// block at that height is the server's (a reorganisation can replace the tip before a
    /// longer chain shows it). A server that is only behind is not a fork.
    fn check_tip(&mut self, tip: u64, tip_hash: &[u8]) -> Result<()> {
        if tip > self.height {
            return Ok(());
        }
        let Some(m) = self.marks.iter().find(|m| m.height == tip).cloned() else { return Ok(()) };
        if !tip_hash.is_empty() && same_hash(&m.hash, tip_hash) {
            return Ok(());
        }
        let server_hash = match &self.backend {
            Backend::Lwd(l) => l.blocks(tip, tip)?.first().map(|b| b.hash.clone()).unwrap_or_default(),
            Backend::Zebra(rpc) => hex::decode(rpc.call("getblockhash", json!([tip]))?.as_str().unwrap_or_default()).unwrap_or_default(),
        };
        if server_hash.is_empty() || same_hash(&server_hash, &m.hash) {
            return Ok(());
        }
        let why = format!("the server's block {tip} is not the scanned one (chain reorganisation)");
        match self.find_fork(tip)? {
            Some(h) => self.rollback_to(h, &why),
            None => {
                self.notes.push(format!("{why}; no kept block is in the server's chain, rescanning from the birthday"));
                self.reset_scan();
            }
        }
        Ok(())
    }

    /// Scans new blocks. Each block is applied fully or not at all; an error leaves the
    /// wallet at the last fully applied block (an older view, never a wrong one). Each
    /// block must link to the one before (hash) and give the Ironwood tree size the chain
    /// says; a block that does not link means a reorganisation: the wallet rolls back to
    /// the newest common block and scans the new chain.
    pub fn sync(&mut self) -> Result<()> {
        let r = if matches!(self.backend, Backend::Lwd(_)) { self.sync_lwd() } else { self.sync_zebra() };
        self.after_scan();
        r
    }

    fn sync_zebra(&mut self) -> Result<()> {
        let (tip, tip_hash) = {
            let Backend::Zebra(rpc) = &self.backend else { unreachable!() };
            let tip = rpc.call("getblockcount", json!([]))?.as_u64().unwrap_or(0);
            (tip, vec![])
        };
        self.check_tip(tip, &tip_hash)?;
        let mut forks = 0;
        while self.height < tip {
            let h = self.height + 1;
            let Backend::Zebra(rpc) = &self.backend else { unreachable!() };
            let blk = rpc.call("getblock", json!([h.to_string(), 1]))?;
            let hash = hex::decode(blk["hash"].as_str().unwrap_or_default()).unwrap_or_default();
            let prev = hex::decode(blk["previousblockhash"].as_str().unwrap_or_default()).unwrap_or_default();
            if let Link::Fork = self.link(h, &prev)? {
                forks += 1;
                if forks > 3 {
                    bail!("the chain keeps changing under the scan; try again later");
                }
                self.on_fork(h)?;
                continue;
            }
            let d = self.zebra_delta(h, hash, &blk)?;
            self.apply(d)?;
        }
        Ok(())
    }

    fn zebra_delta(&self, h: u64, hash: Vec<u8>, blk: &serde_json::Value) -> Result<BlockDelta> {
        let Backend::Zebra(rpc) = &self.backend else { unreachable!() };
        let branch = BranchId::for_height(&self.params, BlockHeight::from_u32(h as u32));
        let base = self.leaf_offset + self.leaves.len() as u64;
        let tree_size = blk["trees"]["ironwood"]["size"].as_u64();
        let mut d = BlockDelta { height: h, hash, leaves: vec![], received: vec![], spent: vec![], lookalikes: 0, mined: vec![], tree_size };
        for txid in blk["tx"].as_array().cloned().unwrap_or_default() {
            let txid = txid.as_str().unwrap_or_default().to_string();
            let raw = rpc.call("getrawtransaction", json!([txid, 0]))?;
            let bytes = hex::decode(raw.as_str().unwrap_or_default())?;
            let tx = Transaction::read(&bytes[..], branch).with_context(|| format!("parse tx {txid} at {h}"))?;
            d.mined.push(txid.clone());
            let lookalike = tx.transparent_bundle().is_none()
                && tx.sapling_bundle().is_none()
                && tx.orchard_bundle().is_none()
                && tx.ironwood_bundle().map(|b| b.actions().len() == 2).unwrap_or(false);
            if lookalike {
                d.lookalikes += 1;
            }
            if let Some(bundle) = tx.ironwood_bundle() {
                for act in bundle.actions() {
                    d.spent.push((act.nullifier().to_bytes(), txid.clone()));
                    let pos = base + d.leaves.len() as u64;
                    d.leaves.push(MerkleHashOrchard::from_cmx(act.cmx()));
                    for k in &self.keys {
                        for (_scope, ivk) in &k.ivks {
                            if let Some((note, _addr, memo)) = try_note_decryption(&IronwoodDomain::for_action(act), ivk, act) {
                                let nf = note.nullifier(&k.fvk).to_bytes();
                                d.received.push(Received { label: k.label.clone(), note, pos, height: h, txid: txid.clone(), memo, nf, spent_in: None });
                            }
                        }
                    }
                }
            }
        }
        Ok(d)
    }

    /// Light-wallet sync: start from the server's Ironwood tree state just before the
    /// birthday, scan compact blocks, and fetch full transactions only for our own notes
    /// (compact blocks carry no memo).
    fn sync_lwd(&mut self) -> Result<()> {
        // A rescan from the birthday (no kept block in the server's chain) starts over.
        for _ in 0..3 {
            if self.sync_lwd_once()? {
                return Ok(());
            }
        }
        bail!("the light-wallet server's chain keeps changing; try again later")
    }

    /// Ok(false): the wallet was reset to rescan from the birthday; call again.
    fn sync_lwd_once(&mut self) -> Result<bool> {
        if self.height == 0 {
            let Backend::Lwd(l) = &self.backend else { unreachable!() };
            let b = l.birthday;
            let (frontier, hash) = l.tree_and_hash(b - 1)?;
            self.frontier = frontier;
            self.leaf_offset = self.frontier.value().map(|f| u64::from(f.position()) + 1).unwrap_or(0);
            self.height = b - 1;
            self.marks.clear();
            self.marks.push_back(Mark { height: b - 1, hash, leaves: 0 });
        }
        let tip = {
            let Backend::Lwd(l) = &self.backend else { unreachable!() };
            l.tip_id()?
        };
        self.check_tip(tip.height, &tip.hash)?;
        if self.height == 0 {
            // the tip check found no common block: start again from the birthday
            return Ok(false);
        }
        let mut forks = 0;
        'outer: while self.height < tip.height {
            let lo = self.height + 1;
            let hi = (lo + 999).min(tip.height);
            let blocks = {
                let Backend::Lwd(l) = &self.backend else { unreachable!() };
                l.blocks(lo, hi)?
            };
            if blocks.is_empty() {
                bail!("the light-wallet server sent no block for {lo}..={hi}");
            }
            for blk in &blocks {
                if let Link::Fork = self.link(blk.height, &blk.prev_hash)? {
                    forks += 1;
                    if forks > 3 {
                        bail!("the chain keeps changing under the scan; try again later");
                    }
                    self.on_fork(blk.height)?;
                    if self.height == 0 {
                        return Ok(false);
                    }
                    continue 'outer;
                }
                let d = self.lwd_delta(blk)?;
                if !self.size_fits(&d)? {
                    return Ok(false);
                }
                self.apply(d)?;
            }
        }
        Ok(true)
    }

    fn lwd_delta(&self, blk: &zcash_client_backend::proto::compact_formats::CompactBlock) -> Result<BlockDelta> {
        let Backend::Lwd(l) = &self.backend else { unreachable!() };
        let branch = BranchId::for_height(&self.params, BlockHeight::from_u32(blk.height as u32));
        let base = self.leaf_offset + self.leaves.len() as u64;
        let tree_size = blk.chain_metadata.as_ref().map(|m| m.ironwood_commitment_tree_size as u64).filter(|n| *n > 0);
        let mut d = BlockDelta { height: blk.height, hash: blk.hash.clone(), leaves: vec![], received: vec![], spent: vec![], lookalikes: 0, mined: vec![], tree_size };
        for ctx in &blk.vtx {
            if ctx.vin.is_empty() && ctx.vout.is_empty() && ctx.spends.is_empty() && ctx.outputs.is_empty()
                && ctx.actions.is_empty() && ctx.ironwood_actions.len() == 2
            {
                d.lookalikes += 1;
            }
            let mut disp = ctx.txid.clone();
            disp.reverse();
            let txid = hex::encode(&disp);
            d.mined.push(txid.clone());
            let mut full: Option<Transaction> = None;
            for ca in &ctx.ironwood_actions {
                let nf = ca.nf().map_err(|e| anyhow!("compact nf: {e:?}"))?.to_bytes();
                d.spent.push((nf, txid.clone()));
                let cmx = ca.cmx().map_err(|e| anyhow!("compact cmx: {e:?}"))?;
                let pos = base + d.leaves.len() as u64;
                d.leaves.push(MerkleHashOrchard::from_cmx(&cmx));
                let Ok(compact) = orchard::note_encryption::CompactAction::try_from(ca) else { continue };
                let cdomain = IronwoodDomain::for_compact_action(&compact);
                for k in &self.keys {
                    for (_scope, ivk) in &k.ivks {
                        if try_compact_note_decryption(&cdomain, ivk, &compact).is_none() {
                            continue;
                        }
                        // ours: fetch the full transaction to recover the memo
                        if full.is_none() {
                            let raw = l.raw_tx(&ctx.txid)?;
                            full = Some(Transaction::read(&raw[..], branch).with_context(|| format!("parse {txid}"))?);
                        }
                        let tx = full.as_ref().unwrap();
                        let bundle = tx.ironwood_bundle().ok_or_else(|| anyhow!("no ironwood bundle in {txid}"))?;
                        let act = bundle.actions().iter().find(|a| a.cmx().to_bytes() == cmx.to_bytes()).ok_or_else(|| anyhow!("action not found"))?;
                        if let Some((note, _addr, memo)) = try_note_decryption(&IronwoodDomain::for_action(act), ivk, act) {
                            let nf = note.nullifier(&k.fvk).to_bytes();
                            d.received.push(Received { label: k.label.clone(), note, pos, height: blk.height, txid: txid.clone(), memo, nf, spent_in: None });
                        }
                    }
                }
            }
        }
        Ok(d)
    }

    /// Test hook: adds a leaf that is not in the chain right after the block
    /// `ANCHOR_DEPTH + 1` below the wallet's height, as an orphaned block scanned without
    /// its hash changing would. Later notes move one position up. The wallet's anchors are
    /// then unknown to every node until [`Wallet::repair`] runs (or the next block's tree
    /// size shows the difference). For tests and the testnet proof of the repair
    /// (`zecret pay --corrupt-tree`, `misbehave` builds only).
    #[cfg(any(test, feature = "misbehave"))]
    pub fn corrupt_tree_for_test(&mut self) {
        let leaf = MerkleHashOrchard::from_bytes(&[7u8; 32].map(|b| b & 0x3f)).unwrap();
        let below = self.height.saturating_sub(ANCHOR_DEPTH + 1);
        let at = self.marks.iter().rev().find(|m| m.height <= below).map(|m| m.leaves).unwrap_or(0);
        self.leaves.insert(at, leaf);
        for m in self.marks.iter_mut().filter(|m| m.height > below) {
            m.leaves += 1;
        }
        let pos = self.leaf_offset + at as u64;
        for r in self.received.iter_mut().filter(|r| r.pos >= pos) {
            r.pos += 1;
        }
    }

    fn after_scan(&mut self) {
        for r in &mut self.received {
            r.spent_in = self.spent.get(&r.nf).map(|(t, _)| t.clone());
        }
        let (spent, tip) = (&self.spent, self.height);
        self.reserved.retain(|nf, (expiry, _)| !spent.contains_key(nf) && *expiry >= tip);
    }

    /// (txid, height) of the transaction that spent the first note received under `label`.
    pub fn spend_of(&self, label: &str) -> Option<(String, u64)> {
        self.received.iter().find(|r| r.label == label).and_then(|r| self.spent.get(&r.nf).cloned())
    }

    /// The mined transactions that spent notes received under `label`, with their heights,
    /// oldest first. For a wallet that only one engine spends from, these are its own sends.
    pub fn sends(&self, label: &str) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = self.received.iter().filter(|r| r.label == label).filter_map(|r| self.spent.get(&r.nf).cloned()).collect();
        v.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
        v.dedup();
        v
    }

    pub fn unspent(&self, label: &str) -> Vec<Received> {
        self.received.iter().filter(|r| r.label == label && r.spent_in.is_none()).cloned().collect()
    }

    pub fn balance(&self, label: &str) -> u64 {
        self.unspent(label).iter().map(|r| r.note.value().inner()).sum()
    }

    fn fvk_of(&self, label: &str) -> Result<FullViewingKey> {
        self.keys.iter().find(|k| k.label == label).map(|k| k.fvk.clone()).ok_or_else(|| anyhow!("unknown key {label}"))
    }

    /// Build, prove and sign a v6 transaction (about 25 s of proving in a browser). The
    /// spent notes are reserved until the transaction is mined or expires. Call
    /// [`Wallet::broadcast`] to send it.
    pub fn build(
        &mut self,
        spends: Vec<(Received, SignWith)>,
        outputs: Vec<(Address, u64, [u8; 512], Option<OutgoingViewingKey>)>,
    ) -> Result<BuiltTx> {
        self.sync()?;
        let tip = self.height;
        // The anchor: the tree after block `ANCHOR_DEPTH` below the tip, or after the
        // newest block that holds a spent note. Consensus accepts the root of any earlier
        // block of the chain; a deeper anchor survives a reorganisation of the tip.
        let newest_note = spends.iter().map(|(r, _)| r.height).max().unwrap_or(0);
        let anchor_height = tip.saturating_sub(ANCHOR_DEPTH).max(newest_note);
        let n_leaves = self.marks.iter().find(|m| m.height == anchor_height).map(|m| m.leaves).unwrap_or(self.leaves.len());
        // Tree with witnesses for the spent notes, anchored at that block.
        let mut tree: ShardTree<MemoryShardStore<MerkleHashOrchard, u32>, 32, 16> =
            ShardTree::new(MemoryShardStore::empty(), 100);
        let marked: HashSet<u64> = spends.iter().map(|(r, _)| r.pos).collect();
        if let Some(p) = marked.iter().find(|p| **p >= self.leaf_offset + n_leaves as u64) {
            bail!("note at position {p} is not in the tree at the anchor (height {anchor_height})");
        }
        if self.frontier.value().is_some() {
            tree.insert_frontier(self.frontier.clone(), Retention::Ephemeral).map_err(|e| anyhow!("insert frontier: {e:?}"))?;
        }
        for (i, leaf) in self.leaves[..n_leaves].iter().enumerate() {
            let pos = self.leaf_offset + i as u64;
            let ret = if marked.contains(&pos) { Retention::Marked } else { Retention::Ephemeral };
            tree.append(*leaf, ret).map_err(|e| anyhow!("tree append: {e:?}"))?;
        }
        tree.checkpoint(0).map_err(|e| anyhow!("{e:?}"))?;
        let root = tree.root_at_checkpoint_id(&0).map_err(|e| anyhow!("{e:?}"))?.ok_or_else(|| anyhow!("empty tree"))?;
        let anchor: orchard::Anchor = root.into();

        let target = BlockHeight::from_u32((tip + 1) as u32);
        let mut b = Builder::new(
            self.params,
            target,
            BuildConfig::Standard {
                sapling_anchor: None,
                orchard_anchor: None,
                ironwood_anchor: Some(anchor),
                orchard_padding: BundlePadding::DEFAULT,
                ironwood_padding: BundlePadding::DEFAULT,
            },
        );
        for (r, _) in &spends {
            let path = tree
                .witness_at_checkpoint_id(Position::from(r.pos), &0)
                .map_err(|e| anyhow!("{e:?}"))?
                .ok_or_else(|| anyhow!("no witness for position {}", r.pos))?;
            let fvk = self.fvk_of(&r.label)?;
            b.add_ironwood_spend::<Infallible>(fvk, r.note, path.into())
                .map_err(|e| anyhow!("add_ironwood_spend: {e:?}"))?;
        }
        for (to, value, memo, ovk) in &outputs {
            b.add_ironwood_output::<Infallible>(ovk.clone(), *to, Zatoshis::from_u64(*value).unwrap(), MemoBytes::from_bytes(memo).unwrap())
                .map_err(|e| anyhow!("add_ironwood_output: {e:?}"))?;
        }
        let res = b
            .build_for_pczt(OsRng, &zip317::FeeRule::standard())
            .map_err(|e| anyhow!("build_for_pczt: {e:?}"))?;
        let spend_idx: Vec<usize> = (0..spends.len())
            .map(|i| res.ironwood_meta.spend_action_index(i).ok_or_else(|| anyhow!("spend index")))
            .collect::<Result<_>>()?;

        let pczt = Creator::build_from_parts(res.pczt_parts).ok_or_else(|| anyhow!("Creator failed"))?;
        let pczt = IoFinalizer::new(pczt).finalize_io(OsRng).map_err(|e| anyhow!("IoFinalizer: {e:?}"))?;
        let pczt = Prover::new(pczt).create_ironwood_proof(OsRng, pk()).map_err(|e| anyhow!("Prover: {e:?}"))?.finish();

        // Read the spend randomizers (alpha) for external signing.
        let mut alphas: Vec<Option<pallas::Scalar>> = vec![None; spend_idx.len()];
        let pczt = low_level_signer::Signer::new(pczt)
            .sign_ironwood_with(|_p, bundle, _m| -> std::result::Result<(), LlErr> {
                for (j, idx) in spend_idx.iter().enumerate() {
                    alphas[j] = *bundle.actions()[*idx].spend().alpha();
                }
                Ok(())
            })
            .map_err(|e| anyhow!("low-level signer: {e:?}"))?
            .finish();

        let mut signer = Signer::new(pczt).map_err(|e| anyhow!("Signer: {e:?}"))?;
        let sighash = signer.shielded_sighash();
        for (j, (_, how)) in spends.iter().enumerate() {
            match how {
                SignWith::Key(ask) => signer.sign_ironwood(OsRng, spend_idx[j], ask).map_err(|e| anyhow!("sign: {e:?}"))?,
                SignWith::Joint(ask) => {
                    let alpha = alphas[j].ok_or_else(|| anyhow!("alpha missing"))?;
                    signer
                        .apply_ironwood_signature(spend_idx[j], keys::sign_spend(ask, &alpha, &sighash))
                        .map_err(|e| anyhow!("joint signature rejected: {e:?}"))?
                }
            }
        }
        let pczt = signer.finish();
        let tx = TransactionExtractor::new(pczt).with_orchard(vk()).extract(OsRng).map_err(|e| anyhow!("extract: {e:?}"))?;
        let mut bytes = vec![];
        tx.write(&mut bytes)?;
        let txid = tx.txid().to_string();
        let expiry_height = u32::from(tx.expiry_height()) as u64;
        for (r, _) in &spends {
            self.reserved.insert(r.nf, (expiry_height, txid.clone()));
        }
        self.track(&txid);
        Ok(BuiltTx { txid, raw: bytes, expiry_height })
    }

    /// Build and broadcast in one call (tools and tests).
    pub fn send(
        &mut self,
        spends: Vec<(Received, SignWith)>,
        outputs: Vec<(Address, u64, [u8; 512], Option<OutgoingViewingKey>)>,
    ) -> Result<String> {
        let tx = self.build(spends, outputs)?;
        self.broadcast(&tx)?.ok()?;
        Ok(tx.txid)
    }

    /// A transaction that sends this wallet's whole value at `label` (one note) to `to`,
    /// minus the fee: the sweep of a joint note.
    pub fn build_sweep(&mut self, note: Received, ask: pallas::Scalar, to: Address) -> Result<BuiltTx> {
        let v = note.note.value().inner();
        let fee = zip317_fee(1, 1);
        if v <= fee {
            bail!("note of {v} zats does not cover the fee {fee}");
        }
        self.build(vec![(note, SignWith::Joint(ask))], vec![(to, v - fee, empty_memo(), None)])
    }

    /// Pay `outputs` from this wallet's own notes (label), returning change to `change`.
    /// Builds the transaction; [`Wallet::broadcast`] sends it.
    pub fn build_pay(
        &mut self,
        label: &str,
        ask: &SpendAuthorizingKey,
        change: Address,
        outputs: Vec<(Address, u64, [u8; 512])>,
    ) -> Result<BuiltTx> {
        self.sync()?;
        let mut notes: Vec<Received> = self.unspent(label).into_iter().filter(|r| !self.reserved.contains_key(&r.nf)).collect();
        notes.sort_by_key(|r| std::cmp::Reverse(r.note.value().inner()));
        let want: u64 = outputs.iter().map(|o| o.1).sum();
        let mut chosen = vec![];
        let mut total = 0u64;
        for n in notes {
            if total >= want + zip317_fee(chosen.len() + 1, outputs.len() + 1) {
                break;
            }
            total += n.note.value().inner();
            chosen.push(n);
        }
        let fee = zip317_fee(chosen.len(), outputs.len() + 1);
        if total < want + fee {
            bail!("insufficient funds: have {total}, need {}", want + fee);
        }
        // Every Cloakroom transaction must have exactly 2 actions (the measured anonymity class):
        // Orchard pads to max(spends, outputs), so with 2 outputs at most 2 input notes are allowed.
        if chosen.len() > 2 {
            bail!("needs {} input notes; consolidate the wallet first so a swap transaction keeps the 2-action shape", chosen.len());
        }
        // ovk = None on every output: a later compromise of this wallet's viewing keys must not reveal
        // the destination or value of swap messages and funding (round-2 review M6).
        let mut outs: Vec<_> = outputs.into_iter().map(|(a, v, m)| (a, v, m, None)).collect();
        outs.push((change, total - want - fee, empty_memo(), None));
        let spends = chosen.into_iter().map(|r| (r, SignWith::Key(ask.clone()))).collect();
        self.build(spends, outs)
    }

    /// Build and broadcast a payment in one call (tools and tests).
    /// A refusal for an unknown anchor repairs the wallet (in [`Wallet::broadcast`]); the
    /// payment is then built once more on the repaired tree.
    pub fn pay(&mut self, label: &str, ask: &SpendAuthorizingKey, change: Address, outputs: Vec<(Address, u64, [u8; 512])>) -> Result<String> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let tx = self.build_pay(label, ask, change, outputs.clone())?;
            match self.broadcast(&tx)? {
                Broadcast::Accepted => return Ok(tx.txid),
                Broadcast::Refused(e) if attempt < 2 && is_unknown_anchor(&e) => continue,
                Broadcast::Refused(e) => bail!("transaction refused: {e}"),
            }
        }
    }

    /// Value of this wallet's spendable notes at `label`, without notes reserved by a
    /// transaction that is not mined yet.
    pub fn spendable(&self, label: &str) -> u64 {
        self.unspent(label).iter().filter(|r| !self.reserved.contains_key(&r.nf)).map(|r| r.note.value().inner()).sum()
    }
}

/// Node answers that mean "the anchor of this transaction is not a tree of my chain":
/// Zebra says "unknown Ironwood anchor" (also for Orchard and Sapling); zcashd says
/// "...-anchor..." in its reject reasons.
pub fn is_unknown_anchor(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("anchor") && (m.contains("unknown") || m.contains("invalid") || m.contains("not found") || m.contains("missing") || m.contains("bad-"))
}

fn short_id(s: &str) -> &str {
    &s[..s.len().min(8)]
}

fn frontier_root(f: &Frontier<MerkleHashOrchard, 32>) -> [u8; 32] {
    use incrementalmerkletree::Hashable;
    f.value().map(|v| v.root(None)).unwrap_or_else(|| MerkleHashOrchard::empty_root(32.into())).to_bytes()
}

/// The same block hash, in either byte order (servers differ in the order they use).
fn same_hash(a: &[u8], b: &[u8]) -> bool {
    a == b || (a.len() == b.len() && a.iter().eq(b.iter().rev()))
}

/// Node answers that mean "I already have this transaction".
fn is_already_known(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    // Zebra 7.0.0-rc.0 answers a mined transaction with "... transaction was committed to
    // the best chain" (seen on regtest); mempool duplicates and other nodes say "already".
    m.contains("committed to the best chain")
        || (m.contains("already") && (m.contains("mempool") || m.contains("exists") || m.contains("known") || m.contains("in chain") || m.contains("in block")))
}

pub fn empty_memo() -> [u8; 512] {
    let mut m = [0u8; 512];
    m[0] = 0xF6;
    m
}

#[derive(Debug)]
#[allow(dead_code)]
struct LlErr(String);
impl From<low_level_signer::OrchardParseError> for LlErr {
    fn from(e: low_level_signer::OrchardParseError) -> Self {
        LlErr(format!("{e:?}"))
    }
}

#[cfg(test)]
#[path = "wallet_sync_tests.rs"]
mod sync_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Response, Transport};
    use std::rc::Rc;

    struct NoNet;
    impl Transport for NoNet {
        fn post(&self, _: &str, _: &[(&str, &str)], _: &[u8]) -> Result<Response> {
            Err(anyhow!("no network in this test"))
        }
    }

    fn wallet() -> Wallet {
        Wallet::new(Backend::Zebra(Rpc::new(Rc::new(NoNet), "http://unused")), Net::Local(regtest_params(Some(2))))
    }

    #[test]
    fn cover_wait_polls_without_sleeping() {
        let w = wallet();
        // no policy: go at once
        let mut c = CoverWait::new(CoverPolicy::default(), 0, None, 100);
        assert_eq!(c.poll(&w, 100).as_deref(), Some("no cover delay configured"));
        // fixed 10 s wait, k = 0: not before 110, then go
        let p = CoverPolicy { min_secs: 10, max_secs: 10, k: 0 };
        let mut c = CoverWait::new(p, 0, None, 100);
        assert!(c.poll(&w, 105).is_none());
        assert!(c.poll(&w, 110).unwrap().starts_with("cover: waited 10s over 1"));
        // k = 1 never met (no look-alikes): redraws until the deadline cuts it short
        let p = CoverPolicy { min_secs: 1, max_secs: 3, k: 1 };
        let mut c = CoverWait::new(p, 0, Some(150), 100);
        let mut now = 100;
        let out = loop {
            if let Some(o) = c.poll(&w, now) {
                break o;
            }
            now += 1;
        };
        assert_eq!(now, 150);
        assert!(out.starts_with("cover cut short by the protocol deadline after 50s"), "{out}");
    }

    #[test]
    fn already_known_answers_count_as_accepted() {
        assert!(is_already_known("transaction is already in the mempool"));
        assert!(is_already_known("{\"code\":-27,\"message\":\"transaction already exists in state\"}"));
        assert!(is_already_known("any transaction with the same effects will be rejected from the mempool until a chain reset: transaction was committed to the best chain"));
        assert!(!is_already_known("bad-txns-inputs-spent"));
        assert!(!is_already_known("already spent"));
    }

    #[test]
    fn tracked_transactions_and_fees() {
        let mut w = wallet();
        assert_eq!(w.mined_height("ab"), None);
        w.track("ab");
        assert_eq!(w.mined_height("ab"), None);
        assert_eq!(zip317_fee(1, 1), 10_000);
        assert_eq!(zip317_fee(3, 2), 15_000);
    }

    #[test]
    fn mainnet_parameters_have_nu7_at_the_height_the_chain_shows() {
        let h = BlockHeight::from_u32;
        // Measured on mainnet (MAINNET_NU7_HEIGHT): branch 0x37a5165b before, 0x77190ad9 from it.
        assert_eq!(u32::from(BranchId::for_height(&Net::Main, h(MAINNET_NU7_HEIGHT - 1))), 0x37a5_165b);
        assert_eq!(u32::from(BranchId::for_height(&Net::Main, h(MAINNET_NU7_HEIGHT))), 0x7719_0ad9);
        assert_eq!(BranchId::for_height(&Net::Main, h(3_570_955)), BranchId::Nu7);
        assert_eq!(Net::Main.network_type(), NetworkType::Main);
        // The earlier upgrades are the library's mainnet heights.
        for nu in [NetworkUpgrade::Sapling, NetworkUpgrade::Nu5, NetworkUpgrade::Nu6_3] {
            assert_eq!(Net::Main.activation_height(nu), MAIN_NETWORK.activation_height(nu));
        }
        // Testnet is unchanged.
        assert_eq!(Net::Test.activation_height(NetworkUpgrade::Nu7), TEST_NETWORK.activation_height(NetworkUpgrade::Nu7));
        assert_eq!(BranchId::for_height(&Net::Test, h(4_618_020)), BranchId::Nu7);
    }
}
