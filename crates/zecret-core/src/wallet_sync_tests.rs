//! Light-wallet sync against a scripted gRPC-web server: chain reorganisations, short and
//! cut-off block streams, blocks with missing actions, and the repair after a node refuses
//! the wallet's anchor (incident 2026-11-13, docs/incidents/2026-11-13-maker-unknown-anchor.md).

use super::*;
use crate::net::{Response, Transport};
use prost::Message;
use std::{cell::RefCell, rc::Rc};
use zcash_client_backend::proto::{
    compact_formats::{ChainMetadata, CompactBlock, CompactOrchardAction, CompactTx},
    service::{BlockId, BlockRange, SendResponse, TreeState},
};

const BIRTHDAY: u64 = 1000;

/// A chain as a light-wallet server sees it: blocks BIRTHDAY.. and an empty Ironwood tree
/// before BIRTHDAY.
struct Chain {
    blocks: Vec<CompactBlock>,
    /// How the next GetBlockRange answers.
    cut: Cut,
    /// Answer of SendTransaction.
    send_error: Option<String>,
    calls: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Cut {
    None,
    /// Only the first N blocks, with grpc-status 0 (a stream that ended early).
    ShortOk(usize),
    /// The body ends in the middle of a frame.
    MidFrame,
}

fn hash_of(height: u64, branch: u8) -> Vec<u8> {
    let mut h = vec![0u8; 32];
    h[..8].copy_from_slice(&height.to_le_bytes());
    h[8] = branch;
    h[31] = 0xab;
    h
}

/// A valid Pallas base field element (top byte small), distinct per (height, branch, i).
fn elem(height: u64, branch: u8, i: u8, tag: u8) -> Vec<u8> {
    let mut b = vec![0u8; 32];
    b[..8].copy_from_slice(&height.to_le_bytes());
    b[8] = branch;
    b[9] = i;
    b[10] = tag;
    b
}

impl Chain {
    fn new() -> Self {
        Chain { blocks: vec![], cut: Cut::None, send_error: None, calls: vec![] }
    }

    fn tip(&self) -> u64 {
        BIRTHDAY - 1 + self.blocks.len() as u64
    }

    fn size_before(&self, height: u64) -> u32 {
        if height <= BIRTHDAY {
            return 0;
        }
        self.blocks[(height - 1 - BIRTHDAY) as usize].chain_metadata.unwrap().ironwood_commitment_tree_size
    }

    /// Adds a block on `branch` with `n` Ironwood actions (one transaction each); the
    /// first transaction gets txid `txid` if given.
    fn push(&mut self, branch: u8, n: u8, txid: Option<[u8; 32]>) {
        let height = self.tip() + 1;
        let prev = if height == BIRTHDAY { hash_of(BIRTHDAY - 1, 0) } else { self.blocks.last().unwrap().hash.clone() };
        let vtx = (0..n)
            .map(|i| CompactTx {
                index: i as u64,
                txid: match txid {
                    Some(t) if i == 0 => t.to_vec(),
                    _ => elem(height, branch, i, 9),
                },
                ironwood_actions: vec![CompactOrchardAction { nullifier: elem(height, branch, i, 1), cmx: elem(height, branch, i, 2), ephemeral_key: vec![], ciphertext: vec![] }],
                ..Default::default()
            })
            .collect();
        let size = self.size_before(height) + n as u32;
        self.blocks.push(CompactBlock {
            height,
            hash: hash_of(height, branch),
            prev_hash: prev,
            vtx,
            chain_metadata: Some(ChainMetadata { ironwood_commitment_tree_size: size, ..Default::default() }),
            ..Default::default()
        });
    }

    /// Replaces the last `depth` blocks with `depth + extra` blocks of `branch`.
    fn reorg(&mut self, depth: usize, extra: usize, branch: u8) {
        for _ in 0..depth {
            self.blocks.pop();
        }
        for _ in 0..depth + extra {
            self.push(branch, 2, None);
        }
    }

    fn frontier_at(&self, height: u64) -> Frontier<MerkleHashOrchard, 32> {
        let mut f = Frontier::empty();
        for b in self.blocks.iter().take_while(|b| b.height <= height) {
            for t in &b.vtx {
                for a in &t.ironwood_actions {
                    f.append(MerkleHashOrchard::from_bytes(&a.cmx.clone().try_into().unwrap()).unwrap());
                }
            }
        }
        f
    }

    fn root_at(&self, height: u64) -> [u8; 32] {
        frontier_root(&self.frontier_at(height))
    }
}

fn frame(flag: u8, p: &[u8]) -> Vec<u8> {
    let mut v = vec![flag];
    v.extend_from_slice(&(p.len() as u32).to_be_bytes());
    v.extend_from_slice(p);
    v
}

fn answer(msgs: &[Vec<u8>], status: u32, message: &str) -> Response {
    let mut body = vec![];
    for m in msgs {
        body.extend(frame(0, m));
    }
    body.extend(frame(0x80, format!("grpc-status:{status}\r\ngrpc-message:{message}\r\n").as_bytes()));
    Response { status: 200, body, headers: vec![] }
}

struct Server(Rc<RefCell<Chain>>);

impl Transport for Server {
    fn post(&self, url: &str, _: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        let mut c = self.0.borrow_mut();
        let method = url.rsplit('/').next().unwrap().to_string();
        c.calls.push(method.clone());
        let req = &body[5..];
        Ok(match method.as_str() {
            "GetLatestBlock" => {
                let tip = c.tip();
                let hash = c.blocks.last().map(|b| b.hash.clone()).unwrap_or_else(|| hash_of(tip, 0));
                answer(&[BlockId { height: tip, hash }.encode_to_vec()], 0, "")
            }
            "GetTreeState" => {
                let h = BlockId::decode(req)?.height;
                let tree = incrementalmerkletree::frontier::CommitmentTree::<MerkleHashOrchard, 32>::from_frontier(&c.frontier_at(h));
                let mut bytes = vec![];
                zcash_primitives::merkle_tree::write_commitment_tree(&tree, &mut bytes)?;
                let mut hash = if h < BIRTHDAY { hash_of(h, 0) } else { c.blocks[(h - BIRTHDAY) as usize].hash.clone() };
                hash.reverse();
                let ts = TreeState { network: "test".into(), height: h, hash: hex::encode(hash), ironwood_tree: if c.frontier_at(h).value().is_some() { hex::encode(bytes) } else { String::new() }, ..Default::default() };
                answer(&[ts.encode_to_vec()], 0, "")
            }
            "GetBlockRange" => {
                let r = BlockRange::decode(req)?;
                let (lo, hi) = (r.start.unwrap().height, r.end.unwrap().height);
                if hi > c.tip() {
                    return Ok(answer(&[], 11, &format!("GetBlock: block {hi} is newer than the latest block")));
                }
                let mut msgs: Vec<Vec<u8>> = (lo.max(BIRTHDAY)..=hi).map(|h| c.blocks[(h - BIRTHDAY) as usize].encode_to_vec()).collect();
                let cut = std::mem::replace(&mut c.cut, Cut::None);
                match cut {
                    Cut::None => answer(&msgs, 0, ""),
                    Cut::ShortOk(n) => {
                        msgs.truncate(n);
                        answer(&msgs, 0, "")
                    }
                    Cut::MidFrame => {
                        let mut r = answer(&msgs, 0, "");
                        let keep = frame(0, &msgs[0]).len() + 7;
                        r.body.truncate(keep);
                        r
                    }
                }
            }
            "SendTransaction" => {
                let e = c.send_error.clone();
                let r = match e {
                    Some(m) => SendResponse { error_code: -25, error_message: m },
                    None => SendResponse { error_code: 0, error_message: String::new() },
                };
                answer(&[r.encode_to_vec()], 0, "")
            }
            m => answer(&[], 12, &format!("{m} not scripted")),
        })
    }
}

fn setup(blocks: usize) -> (Rc<RefCell<Chain>>, Wallet) {
    let chain = Rc::new(RefCell::new(Chain::new()));
    for _ in 0..blocks {
        chain.borrow_mut().push(0, 2, None);
    }
    let w = Wallet::new(Backend::Lwd(Lwd::new(Rc::new(Server(chain.clone())), "http://lwd.test", BIRTHDAY)), Net::Test);
    (chain, w)
}

fn our_root(w: &Wallet) -> [u8; 32] {
    w.root_after(w.leaves.len())
}

fn assert_in_step(chain: &Rc<RefCell<Chain>>, w: &Wallet) {
    let c = chain.borrow();
    assert_eq!(w.height, c.tip(), "height");
    assert_eq!(our_root(w), c.root_at(c.tip()), "the wallet's tree differs from the chain's at {}", c.tip());
    assert_eq!(w.leaf_offset + w.leaves.len() as u64, c.size_before(c.tip() + 1) as u64);
}

#[test]
fn scans_and_follows_new_blocks() {
    let (chain, mut w) = setup(5);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
    chain.borrow_mut().push(0, 3, None);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
    assert!(w.take_notes().is_empty());
}

fn reorg_at_depth(depth: usize) {
    let (chain, mut w) = setup(8);
    let txid = [0x11u8; 32];
    let mut disp = txid;
    disp.reverse();
    let txid_hex = hex::encode(disp);
    // a tracked transaction mined in the block that will be orphaned
    chain.borrow_mut().push(0, 2, Some(txid));
    w.track(&txid_hex);
    for _ in 1..depth {
        chain.borrow_mut().push(0, 2, None);
    }
    w.sync().unwrap();
    let orphan_height = chain.borrow().tip() + 1 - depth as u64;
    assert_eq!(w.mined_height(&txid_hex), Some(orphan_height));
    let stale = our_root(&w);
    // the other branch replaces `depth` blocks and is one block longer
    chain.borrow_mut().reorg(depth, 1, 1);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
    assert_ne!(our_root(&w), stale);
    assert_eq!(w.mined_height(&txid_hex), None, "the orphaned block's transaction is no longer mined");
    let notes = w.take_notes().join("\n");
    assert!(notes.contains(&format!("rolled back from height {} to {} ({depth} block(s))", orphan_height + depth as u64 - 1, orphan_height - 1)), "{notes}");
}

#[test]
fn reorg_of_depth_1_rolls_back_one_block() {
    reorg_at_depth(1);
}

#[test]
fn reorg_of_depth_3_rolls_back_three_blocks() {
    reorg_at_depth(3);
}

#[test]
fn reorg_that_replaces_the_tip_at_the_same_height_is_seen_before_a_new_block() {
    let (chain, mut w) = setup(6);
    w.sync().unwrap();
    // the tip block is replaced; the chain is not longer
    chain.borrow_mut().reorg(1, 0, 2);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
    assert!(w.take_notes().join("\n").contains("is not the scanned one"));
}

#[test]
fn reorg_deeper_than_the_kept_blocks_rescans_from_the_birthday() {
    let (chain, mut w) = setup(KEEP_BLOCKS + 10);
    w.sync().unwrap();
    chain.borrow_mut().reorg(KEEP_BLOCKS + 5, 1, 1);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
    assert!(w.take_notes().join("\n").contains("rescanning from the birthday"));
}

#[test]
fn a_short_stream_is_not_a_skip() {
    let (chain, mut w) = setup(10);
    // the server answers 1000..=1009 with only 4 blocks and status 0
    chain.borrow_mut().cut = Cut::ShortOk(4);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
}

#[test]
fn a_stream_cut_mid_frame_changes_nothing() {
    let (chain, mut w) = setup(10);
    w.sync().unwrap();
    for _ in 0..5 {
        chain.borrow_mut().push(0, 2, None);
    }
    let (h, root) = (w.height, our_root(&w));
    chain.borrow_mut().cut = Cut::MidFrame;
    assert!(w.sync().is_err());
    assert_eq!((w.height, our_root(&w)), (h, root), "a failed sync leaves the wallet at its last full block");
    w.sync().unwrap();
    assert_in_step(&chain, &w);
}

#[test]
fn a_block_with_missing_actions_is_not_applied() {
    let (chain, mut w) = setup(4);
    w.sync().unwrap();
    chain.borrow_mut().push(0, 3, None);
    // the server drops one action of the new block but keeps the chain's tree size
    let saved = {
        let mut c = chain.borrow_mut();
        let b = c.blocks.last_mut().unwrap();
        b.vtx.pop()
    };
    let h = w.height;
    let e = w.sync().unwrap_err().to_string();
    assert!(e.contains("the chain says"), "{e}");
    assert_eq!(w.height, h);
    chain.borrow_mut().blocks.last_mut().unwrap().vtx.push(saved.unwrap());
    w.sync().unwrap();
    assert_in_step(&chain, &w);
}

fn refused_tx() -> BuiltTx {
    BuiltTx { txid: "97fd3cee14ccf63ceb30d0685f58752098189a62744a0d5dced8692a1cd7eb13".into(), raw: vec![1, 2, 3], expiry_height: 2000 }
}

const REFUSAL: &str = "failed to validate tx: WtxId(\"private\"), error: transaction did not pass consensus validation: could not contextually validate transaction on best chain: unknown Ironwood anchor: Root(\"34987c52\")";

#[test]
fn a_refused_anchor_repairs_a_corrupt_tree() {
    let (chain, mut w) = setup(12);
    w.sync().unwrap();
    w.corrupt_tree_for_test();
    assert_ne!(our_root(&w), chain.borrow().root_at(chain.borrow().tip()));
    // a reserved note of the refused transaction is freed
    w.reserved.insert([5u8; 32], (2000, refused_tx().txid));
    chain.borrow_mut().send_error = Some(REFUSAL.into());
    let r = w.broadcast(&refused_tx()).unwrap();
    assert!(matches!(r, Broadcast::Refused(ref e) if is_unknown_anchor(e)));
    assert_in_step(&chain, &w);
    assert!(w.reserved.is_empty(), "the refused transaction's notes are free again");
    let notes = w.take_notes().join("\n");
    assert!(notes.contains("refused transaction 97fd3cee for its anchor") && notes.contains("rescan done"), "{notes}");
    // a second refusal at the same height does not rescan again: the tree matches the
    // server's, so the node is only behind
    w.broadcast(&refused_tx()).unwrap();
    let notes = w.take_notes().join("\n");
    assert!(notes.contains("matches the server's") && !notes.contains("rescanning"), "{notes}");
}

#[test]
fn a_corrupt_tree_is_rebuilt_at_the_next_block() {
    let (chain, mut w) = setup(12);
    w.sync().unwrap();
    w.corrupt_tree_for_test();
    // the next block's tree size (chain metadata) does not fit the wallet's tree
    chain.borrow_mut().push(0, 2, None);
    w.sync().unwrap();
    assert_in_step(&chain, &w);
    assert!(w.take_notes().join("\n").contains("rescanning from the birthday"));
}

#[test]
fn a_refused_anchor_after_a_tip_reorg_rolls_back() {
    let (chain, mut w) = setup(6);
    w.sync().unwrap();
    // The tip is replaced at the same height between the build and the broadcast.
    chain.borrow_mut().reorg(1, 0, 3);
    chain.borrow_mut().send_error = Some(REFUSAL.into());
    w.broadcast(&refused_tx()).unwrap();
    assert_in_step(&chain, &w);
    let notes = w.take_notes().join("\n");
    assert!(notes.contains("rolled back"), "{notes}");
}

#[test]
fn other_refusals_free_the_notes_but_do_not_repair() {
    let (chain, mut w) = setup(3);
    w.sync().unwrap();
    w.reserved.insert([5u8; 32], (2000, refused_tx().txid));
    chain.borrow_mut().send_error = Some("bad-txns-inputs-spent".into());
    let n = chain.borrow().calls.len();
    w.broadcast(&refused_tx()).unwrap();
    assert!(w.reserved.is_empty());
    assert_eq!(chain.borrow().calls.len(), n + 1, "only the send");
    assert!(w.take_notes().is_empty());
}

#[test]
fn unknown_anchor_answers_are_recognised() {
    assert!(is_unknown_anchor(REFUSAL));
    assert!(is_unknown_anchor("bad-txns-orchard-anchor-invalid"));
    assert!(!is_unknown_anchor("bad-txns-inputs-spent"));
    assert!(!is_unknown_anchor("transaction is already in the mempool"));
}

#[test]
fn the_anchor_is_two_blocks_below_the_tip_unless_a_note_is_newer() {
    let (_chain, mut w) = setup(10);
    w.sync().unwrap();
    let tip = w.height;
    let at = |w: &Wallet, h: u64| w.marks.iter().find(|m| m.height == h).map(|m| m.leaves).unwrap();
    assert_eq!(at(&w, tip - ANCHOR_DEPTH), w.leaves.len() - 2 * ANCHOR_DEPTH as usize);
}
