//! sendmemo: native build of the same wallet code; fully shielded send with a text memo.
//! The test uses it to pay the browser wallet from a coinbase-funded `node/` wallet.
//!
//!   sendmemo --zebra http://127.0.0.1:18232 --nu7 2 --dir WALLET_DIR --to UA --zats N --memo TEXT

use anyhow::{anyhow, Result};
use browser_wallet::{backend::Rpc, net::default_transport, wallet::{self, Backend, Net, Wallet}, Me};
use orchard::keys::Scope;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let zebra = arg(&args, "--zebra").unwrap_or("http://127.0.0.1:18232".into());
    let nu7 = arg(&args, "--nu7").map(|s| s.parse()).transpose()?;
    let dir = arg(&args, "--dir").ok_or_else(|| anyhow!("--dir"))?;
    let me = Me::from_wallet_json(&std::fs::read_to_string(format!("{dir}/wallet.json"))?)?;
    let to = wallet::decode_ua(&arg(&args, "--to").ok_or_else(|| anyhow!("--to"))?)?;
    let zats: u64 = arg(&args, "--zats").ok_or_else(|| anyhow!("--zats"))?.parse()?;
    let text = arg(&args, "--memo").unwrap_or_default();
    if text.len() > 512 {
        anyhow::bail!("memo longer than 512 bytes");
    }
    let mut memo = [0u8; 512];
    memo[..text.len()].copy_from_slice(text.as_bytes());
    let mut w = Wallet::new(Backend::Zebra(Rpc::new(default_transport(), &zebra)), Net::Local(wallet::regtest_params(nu7)));
    w.watch("wallet", &me.fvk, &[Scope::External, Scope::Internal]);
    let txid = w.pay("wallet", &me.ask.clone(), me.change(), &me.ovk.clone(), vec![(to, zats, memo)])?;
    eprintln!("timings {}", serde_json::to_string(&w.last_send)?);
    println!("{txid}");
    Ok(())
}
