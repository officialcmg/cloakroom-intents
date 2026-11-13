//! zecret: the native maker daemon, the reference taker and local-network tools.
//!
//! Usage (each party keeps its keys and records in its own directory `--dir D`; the
//! settings are in `D/config.json`):
//!
//!   zecret init      --dir D                      create the wallet key, print the UA
//!   zecret balance   --dir D                      shielded balance (JSON)
//!   zecret pay       --dir D --to UA --zats N     fully shielded payment
//!   zecret near-key  --dir D                      create D/near_key.json, print the key
//!   zecret directory --dir D                      maker: print the signed directory entry
//!   zecret maker     --dir D [--exit-after N] [--vanish-at STAGE]
//!   zecret taker     --dir D --zats N [--vanish-at STAGE]
//!   zecret devnet    --rpc URL --root-key FILE --wasm-dir DIR --escrow-wasm FILE --maker-dir D
//!                                                 set up a NEAR sandbox; print its accounts
//!
//! `--vanish-at STAGE` makes the process exit (state stored) when the swap reaches STAGE,
//! so a test can make a party disappear; running the same command again resumes.


use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zecret_core::{
    config::{Chains, MakerConfig, TakerConfig},
    engine::{self, Event, Next},
    maker::{self, Maker},
    near::{At, NearKey, NearRpc, Signer},
    net::SharedTransport,
    store::Store,
    taker::{self, Taker},
    wallet, Me,
};

use zecret_node::{devnet, io::{FileStore, Http}};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn need(args: &[String], name: &str) -> Result<String> {
    arg(args, name).ok_or_else(|| anyhow!("{name} is required"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn transport() -> Result<SharedTransport> {
    Ok(Rc::new(Http::new()?))
}

fn read_config(dir: &Path) -> Result<Value> {
    let p = dir.join("config.json");
    serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("read {}", p.display()))?).with_context(|| format!("parse {}", p.display()))
}

fn wallet_key(store: &mut FileStore) -> Result<Me> {
    match store.get(taker::KEY_WALLET)? {
        Some(b) => Me::from_hex(std::str::from_utf8(&b)?),
        None => {
            let me = Me::generate();
            store.put(taker::KEY_WALLET, hex::encode(me.sk_bytes()).as_bytes())?;
            Ok(me)
        }
    }
}

/// Prints events as log lines.
fn print_events(role: &str, events: &[Event]) {
    for e in events {
        let line = match e {
            Event::Log { swap, line } => format!("{} {line}", engine::short(swap)),
            Event::Stage { swap, stage } => format!("{} stage {stage}", engine::short(swap)),
            Event::Warning { swap, message } => format!("{} transient: {message}", engine::short(swap)),
            Event::Done { swap, outcome } => format!("{} DONE {outcome}", engine::short(swap)),
            Event::Balance { zats } => format!("balance {zats} zats"),
            Event::Payout { swap, account, token, amount } => format!("{} payout {amount} {token} to {account} in intents", engine::short(swap)),
            other => format!("{}", serde_json::to_string(other).unwrap_or_default()),
        };
        println!("[{role} {}] {line}", now());
    }
}

/// Sleeps until `next` (at most 3 s, so a long wait still notices a stop signal).
fn sleep_until(next: Next) {
    if let Next::At(t) = next {
        let n = now();
        if t > n {
            std::thread::sleep(Duration::from_secs((t - n).min(3)));
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).cloned().unwrap_or_default();
    if cmd == "devnet" {
        return run_devnet(&args);
    }
    let dir = PathBuf::from(need(&args, "--dir")?);
    let mut store = FileStore::new(&dir)?;
    match cmd.as_str() {
        "init" => {
            let cfg = read_config(&dir).ok();
            if let Some(n) = cfg.as_ref().and_then(|c| c["chains"]["network"].as_str()) {
                wallet::set_network(n)?;
            }
            println!("{}", wallet_key(&mut store)?.ua());
        }
        "near-key" => {
            let key = match store.get(maker::KEY_NEAR)? {
                Some(b) => NearKey::from_secret_str(std::str::from_utf8(&b)?)?,
                None => {
                    let k = NearKey::generate();
                    store.put(maker::KEY_NEAR, k.secret_str().as_bytes())?;
                    k
                }
            };
            println!("{}", key.public_key_str());
        }
        "balance" => {
            let chains: Chains = serde_json::from_value(read_config(&dir)?["chains"].clone())?;
            let me = wallet_key(&mut store)?;
            let mut w = engine::wallet_for(&chains, transport()?)?;
            w.watch("wallet", &me.fvk, &[orchard::keys::Scope::External, orchard::keys::Scope::Internal]);
            w.sync()?;
            println!("{}", serde_json::json!({"ua": me.ua(), "height": w.height, "zats": w.balance("wallet")}));
        }
        "pay" => {
            let chains: Chains = serde_json::from_value(read_config(&dir)?["chains"].clone())?;
            let me = wallet_key(&mut store)?;
            let mut w = engine::wallet_for(&chains, transport()?)?;
            w.watch("wallet", &me.fvk, &[orchard::keys::Scope::External, orchard::keys::Scope::Internal]);
            let to = wallet::decode_ua(&need(&args, "--to")?)?;
            let zats: u64 = need(&args, "--zats")?.parse()?;
            println!("{}", w.pay("wallet", &me.ask, me.change(), vec![(to, zats, wallet::empty_memo())])?);
        }
        "directory" => {
            let cfg: MakerConfig = serde_json::from_value(read_config(&dir)?)?;
            let m = Maker::open(cfg, transport()?, &mut store)?;
            println!("{}", serde_json::to_string(&m.directory_entry())?);
        }
        "maker" => run_maker(&dir, &mut store, &args)?,
        "taker" => run_taker(&dir, &mut store, &args)?,
        _ => bail!("unknown command {cmd:?}; see the usage at the top of crates/zecret-node/src/main.rs"),
    }
    Ok(())
}

fn run_maker(dir: &Path, store: &mut FileStore, args: &[String]) -> Result<()> {
    let cfg: MakerConfig = serde_json::from_value(read_config(dir)?)?;
    let exit_after: Option<usize> = arg(args, "--exit-after").map(|s| s.parse()).transpose()?;
    let vanish_at = arg(args, "--vanish-at");
    let mut m = Maker::open(cfg, transport()?, store)?;
    println!("[maker {}] up: UA {} NEAR {} key {}", now(), m.ua(), m.cfg.near_account, m.signer.key.public_key_str());
    loop {
        let t = m.tick(now(), store);
        print_events("maker", &t.events);
        if let Some(v) = &vanish_at {
            if m.swaps().any(|s| s.stage.name().eq_ignore_ascii_case(v)) {
                println!("[maker {}] VANISH at stage {v} (process exits; state is stored)", now());
                return Ok(());
            }
        }
        if let Some(n) = exit_after {
            let done = m.swaps().filter(|s| s.stage.is_terminal()).count();
            if done >= n {
                println!("[maker {}] {done} swap(s) terminal; exiting", now());
                return Ok(());
            }
        }
        sleep_until(t.next);
    }
}

fn run_taker(dir: &Path, store: &mut FileStore, args: &[String]) -> Result<()> {
    let cfg: TakerConfig = serde_json::from_value(read_config(dir)?)?;
    let vanish_at = arg(args, "--vanish-at");
    let mut tk = Taker::open(cfg, transport()?, store)?;
    match tk.swap() {
        Some(s) => println!("[taker {}] resuming swap {} at stage {}", now(), engine::short(&s.rfq_id), s.stage.name()),
        None => {
            let zats: u64 = need(args, "--zats")?.parse()?;
            let ua = tk.ua();
            let s = tk.start(zats, now(), store)?;
            println!("[taker {}] new swap {} for {zats} zats; NEAR account {}; wallet {ua}", now(), engine::short(&s.rfq_id), s.taker_account);
        }
    }
    let ev = tk.take_events();
    print_events("taker", &ev);
    loop {
        let t = tk.tick(now(), store);
        print_events("taker", &t.events);
        if let (Some(v), Some(s)) = (&vanish_at, tk.swap()) {
            if s.stage.name().eq_ignore_ascii_case(v) {
                println!("[taker {}] VANISH at stage {v} (process exits; state is stored)", now());
                return Ok(());
            }
        }
        if t.next == Next::Done {
            let s = tk.swap().ok_or_else(|| anyhow!("no swap"))?;
            println!("[taker {}] finished: {}", now(), s.stage.name());
            return Ok(());
        }
        sleep_until(t.next);
    }
}

fn run_devnet(args: &[String]) -> Result<()> {
    let rpc_url = need(args, "--rpc")?;
    let root_key: Value = serde_json::from_slice(&std::fs::read(need(args, "--root-key")?)?)?;
    let root_id = root_key["account_id"].as_str().ok_or_else(|| anyhow!("root key file has no account_id"))?;
    let root_sk = NearKey::from_secret_str(root_key["secret_key"].as_str().or_else(|| root_key["private_key"].as_str()).ok_or_else(|| anyhow!("root key file has no secret_key"))?)?;
    let wasm_dir = PathBuf::from(need(args, "--wasm-dir")?);
    let maker_dir = PathBuf::from(need(args, "--maker-dir")?);
    let mut mstore = FileStore::new(&maker_dir)?;
    let maker_key = match mstore.get(maker::KEY_NEAR)? {
        Some(b) => NearKey::from_secret_str(std::str::from_utf8(&b)?)?,
        None => {
            let k = NearKey::generate();
            mstore.put(maker::KEY_NEAR, k.secret_str().as_bytes())?;
            k
        }
    };
    let rpc = NearRpc::new(transport()?, &rpc_url);
    rpc.block(&At::Final).context("the sandbox is not answering")?;
    let wnear: u128 = arg(args, "--maker-wnear").map(|s| s.parse()).transpose()?.unwrap_or(1000);
    let setup = devnet::Setup {
        rpc: &rpc,
        root: Signer::new(root_id, root_sk),
        wrap_wasm: std::fs::read(wasm_dir.join("wrap.near.wasm"))?,
        intents_wasm: std::fs::read(wasm_dir.join("intents.near.wasm"))?,
        escrow_wasm: std::fs::read(need(args, "--escrow-wasm")?)?,
        maker_key,
        maker_wnear: wnear * zecret_core::near::ONE_NEAR,
    };
    println!("{}", serde_json::to_string_pretty(&setup.run()?)?);
    Ok(())
}
