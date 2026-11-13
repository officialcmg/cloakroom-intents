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
    consensus::{BlockHeight, BranchId, NetworkType, NetworkUpgrade, Parameters, TEST_NETWORK},
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

/// Consensus parameters: a local regtest chain or the public testnet.
#[derive(Clone, Copy, Debug)]
pub enum Net {
    Local(LocalNetwork),
    Test,
}

impl Parameters for Net {
    fn network_type(&self) -> NetworkType {
        match self {
            Net::Local(p) => p.network_type(),
            Net::Test => NetworkType::Test,
        }
    }
    fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
        match self {
            Net::Local(p) => p.activation_height(nu),
            Net::Test => TEST_NETWORK.activation_height(nu),
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
    /// Nullifiers spent by built transactions that are not mined yet -> expiry height.
    reserved: std::collections::HashMap<[u8; 32], u64>,
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
        if let Some(d) = self.deadline_unix {
            if now >= d {
                return Some(format!("cover cut short by the protocol deadline after {}s ({} draw(s)): {seen} of {} look-alikes", now - self.start, self.draws, policy.k));
            }
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

    /// Track a viewing key. Adding a key resets the scan so its history is found.
    pub fn watch(&mut self, label: &str, fvk: &FullViewingKey, scopes: &[Scope]) {
        if self.keys.iter().any(|k| k.label == label) {
            return;
        }
        let ivks = scopes.iter().map(|s| (*s, PreparedIncomingViewingKey::new(&fvk.to_ivk(*s)))).collect();
        self.keys.push(KeyEntry { label: label.to_string(), fvk: fvk.clone(), ivks });
        self.leaves.clear();
        self.received.clear();
        self.spent.clear();
        self.height = 0;
        self.frontier = Frontier::empty();
        self.leaf_offset = 0;
        self.cover.clear();
        for h in self.tracked.values_mut() {
            *h = None;
        }
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

    /// Broadcast a built transaction. Sending the same transaction again is harmless: a
    /// node that already has it (in its mempool or in a block) answers with an error that
    /// this function treats as success.
    pub fn broadcast(&mut self, tx: &BuiltTx) -> Result<()> {
        self.track(&tx.txid);
        let r = match &mut self.backend {
            Backend::Zebra(rpc) => rpc.call("sendrawtransaction", json!([hex::encode(&tx.raw)])).map(|_| ()),
            Backend::Lwd(l) => l.send(tx.raw.clone()),
        };
        match r {
            Err(e) if is_already_known(&format!("{e:#}")) => Ok(()),
            r => r,
        }
    }

    pub fn sync(&mut self) -> Result<()> {
        if matches!(self.backend, Backend::Lwd(_)) {
            return self.sync_lwd();
        }
        let tip = self.tip()?;
        let Backend::Zebra(rpc) = &self.backend else { unreachable!() };
        while self.height < tip {
            let h = self.height + 1;
            let blk = rpc.call("getblock", json!([h.to_string(), 1]))?;
            let branch = BranchId::for_height(&self.params, BlockHeight::from_u32(h as u32));
            for txid in blk["tx"].as_array().cloned().unwrap_or_default() {
                let txid = txid.as_str().unwrap_or_default().to_string();
                let raw = rpc.call("getrawtransaction", json!([txid, 0]))?;
                let bytes = hex::decode(raw.as_str().unwrap_or_default())?;
                let tx = Transaction::read(&bytes[..], branch).with_context(|| format!("parse tx {txid} at {h}"))?;
                if let Some(slot) = self.tracked.get_mut(&txid) {
                    *slot = Some(h);
                }
                let lookalike = tx.transparent_bundle().is_none()
                    && tx.sapling_bundle().is_none()
                    && tx.orchard_bundle().is_none()
                    && tx.ironwood_bundle().map(|b| b.actions().len() == 2).unwrap_or(false);
                if lookalike {
                    *self.cover.entry(h).or_default() += 1;
                }
                if let Some(bundle) = tx.ironwood_bundle() {
                    for act in bundle.actions() {
                        self.spent.insert(act.nullifier().to_bytes(), (txid.clone(), h));
                        let pos = self.leaves.len() as u64;
                        self.leaves.push(MerkleHashOrchard::from_cmx(act.cmx()));
                        for k in &self.keys {
                            for (_scope, ivk) in &k.ivks {
                                if let Some((note, _addr, memo)) =
                                    try_note_decryption(&IronwoodDomain::for_action(act), ivk, act)
                                {
                                    let nf = note.nullifier(&k.fvk).to_bytes();
                                    self.received.push(Received {
                                        label: k.label.clone(),
                                        note,
                                        pos,
                                        height: h,
                                        txid: txid.clone(),
                                        memo,
                                        nf,
                                        spent_in: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            self.height = h;
        }
        self.after_scan();
        Ok(())
    }

    /// Light-wallet sync: start from the server's Ironwood tree state just before the
    /// birthday, scan compact blocks, and fetch full transactions only for our own notes
    /// (compact blocks carry no memo).
    fn sync_lwd(&mut self) -> Result<()> {
        let Backend::Lwd(l) = &mut self.backend else { unreachable!() };
        if self.height == 0 {
            let b = l.birthday;
            self.frontier = l.tree_before(b - 1)?;
            self.leaf_offset = self.frontier.value().map(|f| u64::from(f.position()) + 1).unwrap_or(0);
            self.height = b - 1;
        }
        let tip = l.tip()?;
        while self.height < tip {
            let lo = self.height + 1;
            let hi = (lo + 999).min(tip);
            let blocks = l.blocks(lo, hi)?;
            for blk in &blocks {
                let branch = BranchId::for_height(&self.params, BlockHeight::from_u32(blk.height as u32));
                for ctx in &blk.vtx {
                    if ctx.vin.is_empty() && ctx.vout.is_empty() && ctx.spends.is_empty() && ctx.outputs.is_empty()
                        && ctx.actions.is_empty() && ctx.ironwood_actions.len() == 2
                    {
                        *self.cover.entry(blk.height).or_default() += 1;
                    }
                    let mut disp = ctx.txid.clone();
                    disp.reverse();
                    let txid = hex::encode(&disp);
                    if let Some(slot) = self.tracked.get_mut(&txid) {
                        *slot = Some(blk.height);
                    }
                    let mut full: Option<Transaction> = None;
                    for ca in &ctx.ironwood_actions {
                        let nf = ca.nf().map_err(|e| anyhow!("compact nf: {e:?}"))?.to_bytes();
                        self.spent.insert(nf, (txid.clone(), blk.height));
                        let cmx = ca.cmx().map_err(|e| anyhow!("compact cmx: {e:?}"))?;
                        let pos = self.leaf_offset + self.leaves.len() as u64;
                        self.leaves.push(MerkleHashOrchard::from_cmx(&cmx));
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
                                    self.received.push(Received { label: k.label.clone(), note, pos, height: blk.height, txid: txid.clone(), memo, nf, spent_in: None });
                                }
                            }
                        }
                    }
                }
            }
            self.height = hi;
        }
        self.after_scan();
        Ok(())
    }

    fn after_scan(&mut self) {
        for r in &mut self.received {
            r.spent_in = self.spent.get(&r.nf).map(|(t, _)| t.clone());
        }
        let (spent, tip) = (&self.spent, self.height);
        self.reserved.retain(|nf, expiry| !spent.contains_key(nf) && *expiry >= tip);
    }

    /// (txid, height) of the transaction that spent the first note received under `label`.
    pub fn spend_of(&self, label: &str) -> Option<(String, u64)> {
        self.received.iter().find(|r| r.label == label).and_then(|r| self.spent.get(&r.nf).cloned())
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
        // Tree with witnesses for the spent notes, anchored at the current tip.
        let mut tree: ShardTree<MemoryShardStore<MerkleHashOrchard, u32>, 32, 16> =
            ShardTree::new(MemoryShardStore::empty(), 100);
        let marked: HashSet<u64> = spends.iter().map(|(r, _)| r.pos).collect();
        if self.frontier.value().is_some() {
            tree.insert_frontier(self.frontier.clone(), Retention::Ephemeral).map_err(|e| anyhow!("insert frontier: {e:?}"))?;
        }
        for (i, leaf) in self.leaves.iter().enumerate() {
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
            b.add_ironwood_spend::<Infallible>(fvk, r.note.clone(), path.into())
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
            self.reserved.insert(r.nf, expiry_height);
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
        self.broadcast(&tx)?;
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
    pub fn pay(&mut self, label: &str, ask: &SpendAuthorizingKey, change: Address, outputs: Vec<(Address, u64, [u8; 512])>) -> Result<String> {
        let tx = self.build_pay(label, ask, change, outputs)?;
        self.broadcast(&tx)?;
        Ok(tx.txid)
    }

    /// Value of this wallet's spendable notes at `label`, without notes reserved by a
    /// transaction that is not mined yet.
    pub fn spendable(&self, label: &str) -> u64 {
        self.unspent(label).iter().filter(|r| !self.reserved.contains_key(&r.nf)).map(|r| r.note.value().inner()).sum()
    }
}

/// Node answers that mean "I already have this transaction".
fn is_already_known(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("already") && (m.contains("mempool") || m.contains("exists") || m.contains("known") || m.contains("in chain") || m.contains("in block"))
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
