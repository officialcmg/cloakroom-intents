//! zecret-deploy: deploys and verifies the Zecret contracts on NEAR.
//!
//! See deploy/README.md and docs/DEPLOY.md.

mod chain;
mod config;
mod deploy;
mod e2e;
mod fund;
mod keys;
mod rpc;
mod verify;

use std::path::{Path, PathBuf};

use chain::{Chain, Deployment, W, code_hash};
use config::{Config, Network};
use deploy::{Ctx, ids, owner_account};
use keys::KeyStore;
use rpc::Rpc;

pub type Err = Box<dyn std::error::Error + Send + Sync>;
pub type Res<T> = Result<T, Err>;

pub const MAINNET_FLAG: &str = "--i-am-the-owner-and-accept-spending-real-funds";

const USAGE: &str = "\
usage: zecret-deploy <command> [options]

commands:
  deploy    --config FILE   create or complete the deployment (safe to run again)
  verify    --config FILE   read-only checks of the deployment; spends nothing
  e2e       --config FILE   one swap end to end on the deployment (testnet, sandbox)
  rehearse  --config FILE   sandbox config only: deploy, verify and e2e in a new local sandbox
  hash      WASM_FILE       print the NEAR code hash (sha256, base58) of a wasm file
  fund      --config FILE ACCOUNT_ID NEAR
                            testnet only: send at least NEAR to the existing account
                            ACCOUNT_ID through faucet accounts (10 NEAR each)
  maker-key --config FILE --out DIR
                            write the maker's NEAR key to DIR/near_key.json (mode 600),
                            the file the maker daemon (zecret maker --dir DIR) reads,
                            and print the deployment's chain settings for its config

options:
  --escrow-wasm FILE   use this escrow wasm instead of escrow.wasm from the config
  --i-am-the-owner-and-accept-spending-real-funds
                       required for any command that sends transactions on mainnet
";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    command: String,
    config: Option<PathBuf>,
    escrow_wasm: Option<String>,
    mainnet_ok: bool,
    file: Option<PathBuf>,
    /// `fund` only: the target account id and the NEAR amount.
    fund: Option<(String, String)>,
}

fn parse_args(argv: &[String]) -> Res<Args> {
    let mut it = argv.iter();
    let command = it.next().ok_or(USAGE)?.clone();
    let mut a = Args {
        command,
        config: None,
        escrow_wasm: None,
        mainnet_ok: false,
        file: None,
        fund: None,
    };
    let mut pos: Vec<String> = Vec::new();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--config" => a.config = Some(it.next().ok_or("--config needs a file")?.into()),
            "--out" => a.file = Some(it.next().ok_or("--out needs a directory")?.into()),
            "--escrow-wasm" => {
                a.escrow_wasm = Some(it.next().ok_or("--escrow-wasm needs a file")?.clone())
            }
            MAINNET_FLAG => a.mainnet_ok = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s if !s.starts_with('-') => pos.push(s.to_string()),
            s => return Err(format!("unknown argument {s}\n\n{USAGE}").into()),
        }
    }
    if a.command == "fund" {
        match <[String; 2]>::try_from(pos) {
            Ok([id, amount]) => a.fund = Some((id, amount)),
            Err(_) => {
                return Err(
                    format!("fund needs an account id and a NEAR amount\n\n{USAGE}").into(),
                );
            }
        }
    } else if let Some(s) = pos.first() {
        if pos.len() > 1 || a.file.is_some() {
            return Err(format!("unknown argument {}\n\n{USAGE}", pos[pos.len() - 1]).into());
        }
        a.file = Some(s.into());
    }
    Ok(a)
}

/// The mainnet guard. Commands that send transactions refuse to run on mainnet without
/// the explicit flag; the end-to-end check never runs on mainnet.
fn guard(network: Network, command: &str, mainnet_ok: bool) -> Res<()> {
    // fund uses the testnet faucet and nothing else.
    if command == "fund" && network != Network::Testnet {
        return Err(format!(
            "refusing to run fund on {}: it runs on testnet only (it uses the testnet faucet)",
            network.as_str()
        )
        .into());
    }
    if network != Network::Mainnet {
        return Ok(());
    }
    match command {
        "verify" => Ok(()),
        "deploy" if mainnet_ok => {
            println!("MAINNET: the owner flag is given; this run spends real NEAR.");
            Ok(())
        }
        "deploy" => Err(format!(
            "refusing to run on mainnet: this spends real funds. Only the owner may run it, with {MAINNET_FLAG}"
        )
        .into()),
        "e2e" | "rehearse" => Err(format!("{command} does not run on mainnet").into()),
        _ => Err(format!("unknown command {command}").into()),
    }
}

async fn open(cfg: Config, base: PathBuf, sandbox_fresh: bool) -> Res<Ctx> {
    let record_path = config::resolve(&base, &cfg.deployment_file)?;
    let keys = KeyStore::open(&config::resolve(&base, &cfg.keys_dir)?)?;
    if sandbox_fresh {
        let _ = std::fs::remove_file(&record_path);
    }
    let record: Deployment = if record_path.exists() {
        serde_json::from_slice(&std::fs::read(&record_path)?)?
    } else {
        Deployment::default()
    };
    if !record.network.is_empty() && record.network != cfg.network.as_str() {
        return Err(format!("{} is for {}", record_path.display(), record.network).into());
    }
    let (w, rpc_url, root) = match cfg.network {
        Network::Sandbox => {
            let worker = near_workspaces::sandbox().await?;
            let root = worker.root_account()?;
            let url = worker.rpc_addr();
            (W::Sandbox(worker), url, Some(root))
        }
        _ => (
            W::remote(cfg.network, &cfg.rpc_url).await?,
            cfg.rpc_url.clone(),
            None,
        ),
    };
    let mut chain = Chain {
        network: cfg.network,
        rpc: Rpc::new(&rpc_url)?,
        w,
        record,
        record_path,
        verbose: true,
    };
    let owner = match root {
        Some(root) => {
            chain.record.owner = root.id().to_string();
            root
        }
        None => owner_account(&mut chain, &cfg, &keys).await?,
    };
    println!(
        "network {}  rpc {}  owner {}",
        cfg.network.as_str(),
        rpc_url,
        owner.id()
    );
    Ok(Ctx {
        chain,
        cfg,
        base,
        keys,
        owner,
    })
}

async fn run(argv: Vec<String>) -> Res<bool> {
    let a = parse_args(&argv)?;
    if a.command == "hash" {
        let f = a.file.ok_or("hash needs a wasm file")?;
        let wasm = std::fs::read(&f)?;
        println!(
            "{}  {} bytes  {}",
            code_hash(&wasm),
            wasm.len(),
            f.display()
        );
        return Ok(true);
    }
    if a.command == "maker-key" {
        let cfg_path = a.config.clone().ok_or("--config is required")?;
        let (cfg, base) = Config::load(&cfg_path)?;
        let out = a.file.clone().ok_or("maker-key needs --out DIR")?;
        return maker_key(&cfg, &base, &out).map(|_| true);
    }
    let cfg_path = a
        .config
        .clone()
        .ok_or_else(|| format!("--config is required\n\n{USAGE}"))?;
    let (mut cfg, base) = Config::load(&cfg_path)?;
    guard(cfg.network, &a.command, a.mainnet_ok)?;
    if let Some((id, amount)) = &a.fund {
        return fund::run(&cfg, &base, id, amount).await.map(|_| true);
    }
    if let Some(w) = &a.escrow_wasm {
        // A command-line path is relative to the current directory.
        cfg.escrow.wasm = std::fs::canonicalize(w)?.to_string_lossy().into_owned();
    }
    let escrow_wasm = deploy::read_wasm(&base, &cfg.escrow.wasm)?;
    println!(
        "escrow wasm {}: {} bytes, code hash {}",
        config::resolve(&base, &cfg.escrow.wasm)?.display(),
        escrow_wasm.len(),
        code_hash(&escrow_wasm)
    );
    match a.command.as_str() {
        "deploy" => {
            let mut ctx = open(cfg, base, false).await?;
            deploy::deploy(&mut ctx).await?;
            println!("== deployed; verifying");
            let ids = ids(&ctx.cfg, ctx.owner.id().as_str());
            let r = verify::verify(&ctx.chain, &ctx.cfg, &ctx.base, &ids).await?;
            println!("deployment record: {}", ctx.chain.record_path.display());
            Ok(r.passed())
        }
        "verify" => {
            if cfg.network == Network::Sandbox {
                return Err("verify reads a live network; use rehearse for the sandbox".into());
            }
            let ctx_owner = verify_owner(&cfg, &base)?;
            let record_path = config::resolve(&base, &cfg.deployment_file)?;
            let record: Deployment = if record_path.exists() {
                serde_json::from_slice(&std::fs::read(&record_path)?)?
            } else {
                Deployment::default()
            };
            let chain = Chain {
                network: cfg.network,
                rpc: Rpc::new(&cfg.rpc_url)?,
                w: W::remote(cfg.network, &cfg.rpc_url).await?,
                record,
                record_path,
                verbose: true,
            };
            let ids = ids(&cfg, &ctx_owner);
            let r = verify::verify(&chain, &cfg, &base, &ids).await?;
            println!(
                "{}",
                if r.passed() {
                    "ALL CHECKS PASSED"
                } else {
                    "SOME CHECKS FAILED"
                }
            );
            Ok(r.passed())
        }
        "e2e" => {
            let mut ctx = open(cfg, base, false).await?;
            let ids = ids(&ctx.cfg, ctx.owner.id().as_str());
            let rep = e2e::run(&mut ctx, &ids).await?;
            write_e2e(&ctx, &rep)?;
            Ok(true)
        }
        "rehearse" => {
            if cfg.network != Network::Sandbox {
                return Err("rehearse needs a sandbox config".into());
            }
            let mut ctx = open(cfg, base, true).await?;
            deploy::deploy(&mut ctx).await?;
            println!("== second deploy run (must change nothing)");
            let n = ctx.chain.record.transactions.len();
            deploy::deploy(&mut ctx).await?;
            if ctx.chain.record.transactions.len() != n {
                return Err("the second deploy run sent transactions; it is not idempotent".into());
            }
            println!("second run sent no transactions");
            let ids = ids(&ctx.cfg, ctx.owner.id().as_str());
            let rep = e2e::run(&mut ctx, &ids).await?;
            write_e2e(&ctx, &rep)?;
            Ok(true)
        }
        c => Err(format!("unknown command {c}\n\n{USAGE}").into()),
    }
}

fn verify_owner(cfg: &Config, base: &Path) -> Res<String> {
    if !cfg.owner_account.is_empty() {
        return Ok(cfg.owner_account.clone());
    }
    let p = config::resolve(base, &cfg.deployment_file)?;
    let rec: Deployment = serde_json::from_slice(
        &std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?,
    )?;
    if rec.owner.is_empty() {
        return Err(format!("{} names no owner", p.display()).into());
    }
    Ok(rec.owner)
}

/// Exports the maker's key for the maker daemon. The daemon's store keeps it as the raw
/// `ed25519:...` string in `<dir>/near_key.json` (crates/zecret-core `maker::KEY_NEAR`).
fn maker_key(cfg: &Config, base: &Path, out: &Path) -> Res<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let owner = verify_owner(cfg, base)?;
    let ids = ids(cfg, &owner);
    let keys = KeyStore::open(&config::resolve(base, &cfg.keys_dir)?)?;
    let sk = keys
        .get(&ids.maker)?
        .ok_or_else(|| format!("no key for maker {}", ids.maker))?;
    std::fs::create_dir_all(out)?;
    std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o700))?;
    let p = out.join("near_key.json");
    if p.exists() {
        return Err(format!("{} exists; not overwriting", p.display()).into());
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&p)?;
    f.write_all(sk.to_string().as_bytes())?;
    println!(
        "wrote the key of {} to {} (mode 600)",
        ids.maker,
        p.display()
    );
    let rpc = if cfg.network == Network::Sandbox {
        String::new()
    } else {
        cfg.rpc_url.clone()
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "near_account": ids.maker,
            "chains": { "near_rpc": rpc, "escrow": ids.escrow, "token": ids.token, "intents": ids.intents },
        }))?
    );
    Ok(())
}

fn write_e2e(ctx: &Ctx, rep: &e2e::E2eReport) -> Res<()> {
    let p = ctx
        .chain
        .record_path
        .with_file_name(format!("e2e.{}.json", ctx.chain.network.as_str()));
    // The file keeps every run: {"runs": [...]}.
    let mut all: serde_json::Value = std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({ "runs": [] }));
    if !all["runs"].is_array() {
        all = serde_json::json!({ "runs": [all] });
    }
    all["runs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(rep)?);
    std::fs::write(&p, serde_json::to_string_pretty(&all)? + "\n")?;
    println!(
        "end-to-end record: {} ({} runs)",
        p.display(),
        all["runs"].as_array().unwrap().len()
    );
    Ok(())
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(argv).await {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn mainnet_refuses_without_the_owner_flag() {
        assert!(guard(Network::Mainnet, "deploy", false).is_err());
        assert!(guard(Network::Mainnet, "deploy", true).is_ok());
        assert!(guard(Network::Mainnet, "e2e", true).is_err());
        assert!(guard(Network::Mainnet, "rehearse", true).is_err());
        assert!(guard(Network::Mainnet, "verify", false).is_ok());
        assert!(guard(Network::Testnet, "deploy", false).is_ok());
        assert!(guard(Network::Testnet, "e2e", false).is_ok());
    }

    #[test]
    fn fund_runs_on_testnet_only() {
        assert!(guard(Network::Testnet, "fund", false).is_ok());
        for net in [Network::Mainnet, Network::Sandbox] {
            for flag in [false, true] {
                let e = guard(net, "fund", flag).unwrap_err().to_string();
                assert!(e.contains("testnet only"), "{e}");
            }
        }
    }

    #[test]
    fn fund_arguments() {
        let a = parse_args(&args(&["fund", "--config", "c.toml", "x.testnet", "4"])).unwrap();
        assert_eq!(a.fund, Some(("x.testnet".into(), "4".into())));
        assert_eq!(a.file, None);
        assert!(parse_args(&args(&["fund", "--config", "c.toml", "x.testnet"])).is_err());
        assert!(parse_args(&args(&["fund", "--config", "c.toml"])).is_err());
        assert!(parse_args(&args(&["fund", "--config", "c.toml", "a", "1", "2"])).is_err());
        // Other commands keep one positional argument.
        let a = parse_args(&args(&["hash", "f.wasm"])).unwrap();
        assert_eq!(a.file, Some(PathBuf::from("f.wasm")));
        assert_eq!(a.fund, None);
        assert!(parse_args(&args(&["hash", "a.wasm", "b.wasm"])).is_err());
        assert!(parse_args(&args(&["maker-key", "--out", "d", "x"])).is_err());
    }

    #[tokio::test]
    async fn fund_on_mainnet_stops_before_any_network_access() {
        let cfg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../configs/mainnet.toml");
        let e = run(args(&[
            "fund",
            "--config",
            cfg.to_str().unwrap(),
            "someone.near",
            "1",
        ]))
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("testnet only"), "{e}");
    }

    #[tokio::test]
    async fn fund_on_the_sandbox_config_is_refused() {
        let cfg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../configs/sandbox.toml");
        let e = run(args(&[
            "fund",
            "--config",
            cfg.to_str().unwrap(),
            "x.testnet",
            "1",
        ]))
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("testnet only"), "{e}");
    }

    #[test]
    fn the_flag_is_parsed_only_when_spelled_exactly() {
        let a = parse_args(&args(&["deploy", "--config", "c.toml", MAINNET_FLAG])).unwrap();
        assert!(a.mainnet_ok);
        let a = parse_args(&args(&["deploy", "--config", "c.toml"])).unwrap();
        assert!(!a.mainnet_ok);
        assert!(parse_args(&args(&["deploy", "--i-am-the-owner"])).is_err());
    }

    #[tokio::test]
    async fn mainnet_deploy_stops_before_any_network_access() {
        let cfg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../configs/mainnet.toml");
        let e = run(args(&["deploy", "--config", cfg.to_str().unwrap()]))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("refusing to run on mainnet"), "{e}");
    }

    #[test]
    fn code_hash_is_base58_sha256() {
        // sha256("") = e3b0c442...; base58 of it:
        assert_eq!(
            code_hash(b""),
            "GKot5hBsd81kMupNCXHaqbhv3huEbxAFMLnpcX2hniwn"
        );
    }
}
