//! memocheck: independent check of a mined transaction, without the wallet code.
//! It reads the transaction from Zebra's JSON-RPC, reports its shape, and trial-decrypts
//! its Ironwood actions with the recipient's key (a `node/` wallet.json) using orchard
//! directly. Exit code 1 if the transaction is not fully shielded, not mined, or the
//! recipient cannot decrypt the expected value and memo.
//!
//!   memocheck --zebra URL --nu7 2 --txid T --dir RECIPIENT_DIR [--zats N] [--memo-hex H]

use anyhow::{anyhow, bail, Result};
use orchard::{
    keys::{FullViewingKey, PreparedIncomingViewingKey, Scope, SpendingKey},
    note_encryption::IronwoodDomain,
};
use serde_json::{json, Value};
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::{
    consensus::{BlockHeight, BranchId},
    local_consensus::LocalNetwork,
};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn rpc(url: &str, method: &str, params: Value) -> Result<Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let v = reqwest::blocking::Client::new().post(url).header("content-type", "application/json").body(body.to_string()).send()?.text()?;
    let v: Value = serde_json::from_str(&v)?;
    if !v["error"].is_null() {
        bail!("{method}: {}", v["error"]);
    }
    Ok(v["result"].clone())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let url = arg(&args, "--zebra").unwrap_or("http://127.0.0.1:18232".into());
    let nu7: Option<u32> = arg(&args, "--nu7").map(|s| s.parse()).transpose()?;
    let txid = arg(&args, "--txid").ok_or_else(|| anyhow!("--txid"))?;
    let dir = arg(&args, "--dir").ok_or_else(|| anyhow!("--dir"))?;
    let want_zats: Option<u64> = arg(&args, "--zats").map(|s| s.parse()).transpose()?;
    let want_memo = arg(&args, "--memo-hex");

    let verbose = rpc(&url, "getrawtransaction", json!([txid, 1]))?;
    let height = verbose["height"].as_u64().ok_or_else(|| anyhow!("transaction {txid} is not mined (no height)"))?;
    let confirmations = verbose["confirmations"].as_u64().unwrap_or(0);
    let raw = hex::decode(verbose["hex"].as_str().ok_or_else(|| anyhow!("no hex"))?)?;
    let one = Some(BlockHeight::from_u32(1));
    let params = LocalNetwork {
        overwinter: one, sapling: one, blossom: one, heartwood: one, canopy: one, nu5: one, nu6: one, nu6_1: one, nu6_2: one, nu6_3: one,
        nu7: nu7.map(BlockHeight::from_u32),
    };
    let tx = Transaction::read(&raw[..], BranchId::for_height(&params, BlockHeight::from_u32(height as u32)))?;
    let (tin, tout) = tx.transparent_bundle().map(|b| (b.vin.len(), b.vout.len())).unwrap_or((0, 0));
    let (ss, so) = tx.sapling_bundle().map(|b| (b.shielded_spends().len(), b.shielded_outputs().len())).unwrap_or((0, 0));
    let oa = tx.orchard_bundle().map(|b| b.actions().len()).unwrap_or(0);
    let bundle = tx.ironwood_bundle().ok_or_else(|| anyhow!("no Ironwood bundle"))?;
    let ia = bundle.actions().len();
    let vb = i64::from(*bundle.value_balance());
    println!(
        "tx {txid}: mined at height {height} ({confirmations} confirmations), v{}, transparent in/out {tin}/{tout}, sapling spends/outputs {ss}/{so}, orchard actions {oa}, ironwood actions {ia}, ironwood value balance {vb} zats (= fee), {} bytes",
        u32::from(tx.version().header()) & 0x7fffffff,
        raw.len()
    );
    let fully_shielded = tin == 0 && tout == 0 && ss == 0 && so == 0 && oa == 0 && ia > 0;
    println!("fully shielded (no transparent, Sapling or Orchard parts): {fully_shielded}");

    let w: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/wallet.json"))?)?;
    let b: [u8; 32] = hex::decode(w["sk"].as_str().unwrap_or_default())?.try_into().map_err(|_| anyhow!("sk"))?;
    let sk = Option::<SpendingKey>::from(SpendingKey::from_bytes(b)).ok_or_else(|| anyhow!("sk"))?;
    let fvk = FullViewingKey::from(&sk);
    let mut found = vec![];
    for act in bundle.actions() {
        for scope in [Scope::External, Scope::Internal] {
            let ivk = PreparedIncomingViewingKey::new(&fvk.to_ivk(scope));
            if let Some((note, _addr, memo)) = try_note_decryption(&IronwoodDomain::for_action(act), &ivk, act) {
                found.push((note.value().inner(), memo));
            }
        }
    }
    let mut ok = fully_shielded;
    if found.is_empty() {
        println!("recipient {dir}: cannot decrypt any action");
        ok = false;
    }
    for (v, memo) in &found {
        let text_end = memo.iter().position(|b| *b == 0).unwrap_or(512);
        println!(
            "recipient {dir}: decrypted note of {v} zats, memo {} bytes of text, starts {:?}",
            text_end,
            String::from_utf8_lossy(&memo[..text_end.min(60)])
        );
        if let Some(z) = want_zats {
            let m = *v == z;
            println!("  value matches expected {z}: {m}");
            ok &= m;
        }
        if let Some(h) = &want_memo {
            let m = hex::encode(memo) == *h;
            println!("  memo matches the 512 bytes the browser sent: {m}");
            ok &= m;
        }
    }
    if !ok {
        bail!("CHECK FAILED");
    }
    println!("CHECK OK");
    Ok(())
}
