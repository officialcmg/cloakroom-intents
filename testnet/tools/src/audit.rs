//! Audit of Zcash testnet transactions through the gRPC-web light-wallet server, with
//! the core's own client: height and block time (from compact blocks), version, and
//! what each pool shows. usage: audit FROM_HEIGHT TXID...
use anyhow::{anyhow, Result};
use std::{collections::HashMap, rc::Rc};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, TEST_NETWORK};
use zecret_core::zcash::Lwd;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let from: u64 = a[0].parse()?;
    let want: Vec<String> = a[1..].to_vec();
    let l = Lwd::new(Rc::new(zecret_node::io::Http::new()?), "https://zcash-testnet.chainsafe.dev", from);
    let tip = l.tip()?;
    let mut found: HashMap<String, (u64, u32)> = HashMap::new();
    let mut times = vec![];
    for b in l.blocks(from, tip)? {
        times.push((b.height, b.time));
        for t in &b.vtx {
            let mut d = t.txid.clone();
            d.reverse();
            found.insert(hex::encode(d), (b.height, b.time));
        }
    }
    println!("tip {tip}; blocks {from}..={tip}");
    for w in times.windows(2) { print!("{}:+{}s ", w[1].0, w[1].1 as i64 - w[0].1 as i64); }
    println!();
    println!("txid | height | block time (unix) | version | transparent in/out | sapling spends/outputs | orchard actions | ironwood actions | ironwood value balance (zats)");
    for t in &want {
        let mut h = hex::decode(t)?;
        h.reverse();
        let raw = l.raw_tx(&h)?;
        let (ht, bt) = found.get(t).copied().ok_or_else(|| anyhow!("{t} not found in compact blocks {from}..={tip}"))?;
        let branch = BranchId::for_height(&TEST_NETWORK, BlockHeight::from_u32(ht as u32));
        let tx = Transaction::read(&raw[..], branch)?;
        let (tin, tout) = tx.transparent_bundle().map(|b| (b.vin.len(), b.vout.len())).unwrap_or((0, 0));
        let (ss, so) = tx.sapling_bundle().map(|b| (b.shielded_spends().len(), b.shielded_outputs().len())).unwrap_or((0, 0));
        let oa = tx.orchard_bundle().map(|b| b.actions().len()).unwrap_or(0);
        let (ia, ivb) = tx.ironwood_bundle().map(|b| (b.actions().len(), i64::from(*b.value_balance()))).unwrap_or((0, 0));
        let txid_ok = tx.txid().to_string() == *t;
        println!("{t} | {ht} | {bt} | v{} | {tin}/{tout} | {ss}/{so} | {oa} | {ia} | {ivb} | txid recomputed: {txid_ok} | {} bytes", u32::from(tx.version().header()) & 0x7fffffff, raw.len());
    }
    Ok(())
}
