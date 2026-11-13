//! Chain check for the 2026-11-13 maker incident (docs/incidents/2026-11-13-maker-unknown-anchor.md).
//!
//! usage: chaincheck BIRTHDAY FROM TO [ROOT_HEX@UNIX_TIME ...]
//!        chaincheck watch SECONDS
//!        chaincheck wallet SECONDS
//!
//! `wallet` runs the core's light wallet (no keys) as a long-lived process: it syncs
//! every 5 s for SECONDS and prints the wallet's notes (rollbacks, rescans). At the end it
//! runs the repair check, which compares the wallet's tree with the server's.
//!
//! `watch` asks the server every 5 s for its tip and the last 10 blocks, for SECONDS,
//! and prints every block whose hash changed at a height it saw before (a reorganisation
//! seen live), and every time the tip went down.
//!
//! 1. Reads the compact blocks BIRTHDAY..=TO from the light-wallet server in ranges of
//!    1000, as the wallet does, and checks: the heights are contiguous, each prev_hash is
//!    the hash of the block before, and the Ironwood tree size after each block matches
//!    the block's chain metadata.
//! 2. Builds the Ironwood tree from the server's tree state before BIRTHDAY, and checks
//!    its root at a few heights against GetTreeState.
//! 3. For each ROOT (an anchor a node refused, with the time it was used), searches the
//!    heights whose block time is up to 15 minutes before that time for a tree that has
//!    this root, under these hypotheses about a block b in FROM..=TO: correct tree; block
//!    b skipped; block b applied twice; the first k actions of b applied twice; one
//!    action of b missing. It prints every match.
//! 4. Asks the server for its tip and the hash of a few blocks 12 times, to see whether
//!    different backends behind the host name answer differently.
use anyhow::{anyhow, bail, Result};
use incrementalmerkletree::{frontier::Frontier, Hashable};
use orchard::tree::MerkleHashOrchard;
use std::rc::Rc;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zecret_core::zcash::Lwd;

type F = Frontier<MerkleHashOrchard, 32>;

fn root_of(f: &F) -> [u8; 32] {
    f.value().map(|v| v.root(None)).unwrap_or_else(|| MerkleHashOrchard::empty_root(32.into())).to_bytes()
}

fn leaves_of(b: &CompactBlock) -> Result<Vec<MerkleHashOrchard>> {
    let mut v = vec![];
    for t in &b.vtx {
        for a in &t.ironwood_actions {
            let cmx = a.cmx().map_err(|e| anyhow!("cmx at {}: {e:?}", b.height))?;
            v.push(MerkleHashOrchard::from_cmx(&cmx));
        }
    }
    Ok(v)
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(String::as_str) == Some("wallet") {
        return long_wallet(a.get(1).and_then(|s| s.parse().ok()).unwrap_or(600));
    }
    if a.first().map(String::as_str) == Some("watch") {
        return watch(a.get(1).and_then(|s| s.parse().ok()).unwrap_or(600));
    }
    if a.len() < 3 {
        bail!("usage: chaincheck BIRTHDAY FROM TO [ROOT_HEX@UNIX_TIME ...]");
    }
    let (birthday, from, to): (u64, u64, u64) = (a[0].parse()?, a[1].parse()?, a[2].parse()?);
    let mut targets: Vec<([u8; 32], u64, String)> = vec![];
    for t in &a[3..] {
        let (h, time) = t.split_once('@').ok_or_else(|| anyhow!("ROOT_HEX@UNIX_TIME"))?;
        let b: [u8; 32] = hex::decode(h)?.try_into().map_err(|_| anyhow!("root length"))?;
        targets.push((b, time.parse()?, h[..8].to_string()));
        let mut r = b;
        r.reverse();
        targets.push((r, time.parse()?, format!("{} (reversed)", &h[..8])));
    }
    let l = Lwd::new(Rc::new(zecret_node::io::Http::new()?), "https://zcash-testnet.chainsafe.dev", birthday);

    let tip = l.tip()?;
    let to = to.min(tip);
    println!("server tip {tip}; checking up to {to}");
    // 1. blocks, continuity, tree sizes
    let start = std::time::Instant::now();
    let mut blocks: Vec<CompactBlock> = vec![];
    let mut lo = birthday;
    while lo <= to {
        let hi = (lo + 999).min(to);
        let got = l.blocks(lo, hi)?;
        if got.len() as u64 != hi - lo + 1 {
            println!("range {lo}..={hi}: asked {} blocks, got {}", hi - lo + 1, got.len());
        }
        blocks.extend(got);
        lo = hi + 1;
    }
    println!("fetched {} compact blocks {birthday}..={to} in {:.1}s", blocks.len(), start.elapsed().as_secs_f64());
    let base = l.tree_before(birthday - 1)?;
    let base_size = base.value().map(|f| u64::from(f.position()) + 1).unwrap_or(0);
    let mut size = base_size;
    let mut problems = 0;
    for (i, b) in blocks.iter().enumerate() {
        if b.height != birthday + i as u64 {
            println!("height gap: block {i} has height {}, expected {}", b.height, birthday + i as u64);
            problems += 1;
        }
        if i > 0 && b.prev_hash != blocks[i - 1].hash {
            println!("prev_hash mismatch at {}", b.height);
            problems += 1;
        }
        size += leaves_of(b)?.len() as u64;
        if let Some(m) = &b.chain_metadata {
            if m.ironwood_commitment_tree_size as u64 != size {
                println!("tree size at {}: counted {size}, chain metadata {}", b.height, m.ironwood_commitment_tree_size);
                problems += 1;
            }
        }
    }
    println!("continuity: {problems} problem(s); Ironwood tree size {base_size} before {birthday}, {size} after {to}");

    // 2. frontiers per height; check against GetTreeState
    let mut fr: Vec<F> = Vec::with_capacity(blocks.len());
    let mut f = base.clone();
    for b in &blocks {
        for leaf in leaves_of(b)? {
            f.append(leaf);
        }
        fr.push(f.clone());
    }
    let at = |h: u64| -> &F { &fr[(h - birthday) as usize] };
    for h in [from, (from + to) / 2, to] {
        let server = l.tree_before(h)?;
        println!("root at {h}: ours {} server {} {}", hex::encode(root_of(at(h))), hex::encode(root_of(&server)), if root_of(at(h)) == root_of(&server) { "same" } else { "DIFFERENT" });
    }
    for b in blocks.iter().filter(|b| b.height + 25 > to) {
        println!("block {} time {} hash {}", b.height, b.time, { let mut h = b.hash.clone(); h.reverse(); hex::encode(h) });
    }
    let actions: Vec<(u64, usize)> = blocks.iter().filter(|b| b.height >= from).map(|b| (b.height, leaves_of(b).map(|v| v.len()).unwrap_or(0))).filter(|x| x.1 > 0).collect();
    println!("blocks in {from}..={to} with Ironwood actions (height:actions): {}", actions.iter().map(|(h, n)| format!("{h}:{n}")).collect::<Vec<_>>().join(" "));

    // 3. hypotheses
    if !targets.is_empty() {
        let time_of = |h: u64| blocks[(h - birthday) as usize].time as u64;
        // candidate tip heights per target: block time in [t - 900, t]
        let cands: Vec<Vec<u64>> = targets.iter().map(|(_, t, _)| (from..=to).filter(|h| time_of(*h) <= *t && time_of(*h) + 900 >= *t).collect()).collect();
        for (i, (_, t, name)) in targets.iter().enumerate().step_by(2) {
            println!("target {name} at {t}: candidate tips {:?}", cands[i].first().zip(cands[i].last()));
        }
        let maxc = cands.iter().flat_map(|c| c.iter().copied()).max().unwrap_or(from);
        let any_c: std::collections::HashSet<u64> = cands.iter().flat_map(|c| c.iter().copied()).collect();
        let check = |label: &str, f: &F, h: u64| {
            if !any_c.contains(&h) {
                return;
            }
            let r = root_of(f);
            for (i, (root, _, name)) in targets.iter().enumerate() {
                if r == *root && cands[i].contains(&h) {
                    println!("MATCH {name}: {label}, tree at tip {h}");
                }
            }
        };
        // per block: the Ironwood leaves of each transaction
        let txs_of = |h: u64| -> Vec<Vec<MerkleHashOrchard>> {
            blocks[(h - birthday) as usize].vtx.iter().map(|t| t.ironwood_actions.iter().filter_map(|a| a.cmx().ok()).map(|c| MerkleHashOrchard::from_cmx(&c)).collect::<Vec<_>>()).filter(|v: &Vec<_>| !v.is_empty()).collect()
        };
        println!("height of FROM {from}: block time {}", time_of(from));
        // correct tree
        for h in from..=to {
            check("correct tree", at(h), h);
        }
        let mut n_hyp = 0u64;
        for b in from..=maxc {
            let lv = leaves_of(&blocks[(b - birthday) as usize])?;
            let mut hyps: Vec<(String, Vec<MerkleHashOrchard>)> = vec![];
            if !lv.is_empty() {
                hyps.push((format!("block {b} skipped"), vec![]));
                hyps.push((format!("block {b} applied twice"), lv.iter().chain(lv.iter()).cloned().collect()));
                for k in 1..lv.len() {
                    hyps.push((format!("first {k} of {} actions of block {b} applied twice", lv.len()), lv[..k].iter().chain(lv.iter()).cloned().collect()));
                }
                for j in 0..lv.len() {
                    hyps.push((format!("action {j} of {} of block {b} missing", lv.len()), lv.iter().enumerate().filter(|(i, _)| *i != j).map(|(_, x)| *x).collect()));
                }
            }
            // depth-1 reorganisation: the wallet scanned an orphan block at height b, whose
            // transactions are a subset of the transactions mined in b..=b+2, and never
            // scanned the block that replaced it.
            let pool: Vec<(u64, Vec<MerkleHashOrchard>)> = (b..=(b + 2).min(to)).flat_map(|h| txs_of(h).into_iter().map(move |t| (h, t))).collect();
            if pool.len() <= 10 {
                for mask in 0u32..(1 << pool.len()) {
                    let picked: Vec<usize> = (0..pool.len()).filter(|i| mask & (1 << i) != 0).collect();
                    let leaves: Vec<MerkleHashOrchard> = picked.iter().flat_map(|i| pool[*i].1.iter().cloned()).collect();
                    if leaves == lv {
                        continue; // that is the correct block
                    }
                    let desc: Vec<String> = picked.iter().map(|i| format!("tx of {} with {} action(s)", pool[*i].0, pool[*i].1.len())).collect();
                    hyps.push((format!("orphan block at {b} holding [{}], replacement never scanned", desc.join(", ")), leaves));
                }
            } else {
                println!("block {b}: {} transactions in b..=b+2, orphan search skipped", pool.len());
            }
            for (label, mine) in hyps {
                n_hyp += 1;
                let mut f = at(b - 1).clone();
                for leaf in mine {
                    f.append(leaf);
                }
                check(&label, &f, b);
                for h in b + 1..=maxc {
                    for leaf in leaves_of(&blocks[(h - birthday) as usize])? {
                        f.append(leaf);
                    }
                    check(&label, &f, h);
                }
            }
        }
        println!("{n_hyp} hypotheses tried");
        println!("hypothesis search done");
    }

    // 4. do the backends agree?
    for i in 0..12 {
        let tip = l.tip()?;
        let probe = l.blocks(tip, tip).map(|v| v.len()).map_err(|e| format!("{e:#}"));
        let next = l.blocks(tip + 1, tip + 1).map(|v| v.len()).map_err(|e| format!("{e:#}"));
        println!("probe {i}: tip {tip}; GetBlockRange(tip) -> {probe:?} block(s); GetBlockRange(tip+1) -> {next:?}");
    }
    Ok(())
}

fn watch(secs: u64) -> Result<()> {
    let l = Lwd::new(Rc::new(zecret_node::io::Http::new()?), "https://zcash-testnet.chainsafe.dev", 1);
    let start = std::time::Instant::now();
    let mut seen: std::collections::BTreeMap<u64, Vec<u8>> = Default::default();
    let (mut polls, mut errors, mut reorgs, mut last_tip) = (0u64, 0u64, 0u64, 0u64);
    while start.elapsed().as_secs() < secs {
        polls += 1;
        let r = (|| -> Result<()> {
            let tip = l.tip()?;
            if tip < last_tip {
                println!("{} tip went down from {last_tip} to {tip}", now());
            }
            last_tip = tip;
            for b in l.blocks(tip.saturating_sub(9), tip)? {
                if let Some(old) = seen.get(&b.height) {
                    if *old != b.hash {
                        reorgs += 1;
                        println!("{} REORG at {}: hash {} -> {} (tip {tip})", now(), b.height, hx(old), hx(&b.hash));
                    }
                }
                seen.insert(b.height, b.hash.clone());
            }
            Ok(())
        })();
        if let Err(e) = r {
            errors += 1;
            println!("{} error: {e:#}", now());
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    println!("watched {}s: {polls} polls, {errors} errors, heights {:?}..{:?}, {reorgs} replaced block(s)", secs, seen.keys().next(), seen.keys().last());
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn hx(h: &[u8]) -> String {
    let mut v = h.to_vec();
    v.reverse();
    hex::encode(&v[..8.min(v.len())])
}

fn long_wallet(secs: u64) -> Result<()> {
    use zecret_core::wallet::{Backend, Net, Wallet};
    let t: zecret_core::net::SharedTransport = Rc::new(zecret_node::io::Http::new()?);
    let probe = Lwd::new(t.clone(), "https://zcash-testnet.chainsafe.dev", 1);
    let birthday = probe.tip()? - 300;
    let mut w = Wallet::new(Backend::Lwd(Lwd::new(t, "https://zcash-testnet.chainsafe.dev", birthday)), Net::Test);
    let start = std::time::Instant::now();
    let (mut syncs, mut errors) = (0u64, 0u64);
    while start.elapsed().as_secs() < secs {
        syncs += 1;
        if let Err(e) = w.sync() {
            errors += 1;
            println!("{} sync error: {e:#}", now());
        }
        for n in w.take_notes() {
            println!("{} wallet: {n}", now());
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    let changed = w.repair()?;
    for n in w.take_notes() {
        println!("{} wallet: {n}", now());
    }
    println!("long-lived wallet from {birthday}: {syncs} syncs, {errors} errors, at height {}, {} actions; final check changed the tree: {changed}", w.height, w.actions_scanned());
    Ok(())
}
