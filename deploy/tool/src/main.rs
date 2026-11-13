//! zecret-deploy: deploys and verifies the Zecret contracts on NEAR.
//!
//! See deploy/README.md and docs/DEPLOY.md.

mod chain;
mod config;
mod deploy;
mod e2e;
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
    };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--config" => a.config = Some(it.next().ok_or("--config needs a file")?.into()),
            "--escrow-wasm" => a.escrow_wasm = Some(it.next().ok_or("--escrow-wasm needs a file")?.clone()),
            MAINNET_FLAG => a.mainnet_ok = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s if !s.starts_with('-') && a.file.is_none() => a.file = Some(s.into()),
            s => return Err(format!("unknown argument {s}\n\n{USAGE}").into()),
        }
    }
    Ok(a)
}

/// The mainnet guard. Commands that send transactions refuse to run on mainnet without
/// the explicit flag; the end-to-end check never runs on mainnet.
fn guard(network: Network, command: &str, mainnet_ok: bool) -> Res<()> {
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
        _ => {
            let worker = near_workspaces::custom(&cfg.rpc_url).await?;
            (W::Remote(worker), cfg.rpc_url.clone(), None)
        }
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
    println!("network {}  rpc {}  owner {}", cfg.network.as_str(), rpc_url, owner.id());
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
        println!("{}  {} bytes  {}", code_hash(&wasm), wasm.len(), f.display());
        return Ok(true);
    }
    let cfg_path = a.config.clone().ok_or_else(|| format!("--config is required\n\n{USAGE}"))?;
    let (mut cfg, base) = Config::load(&cfg_path)?;
    guard(cfg.network, &a.command, a.mainnet_ok)?;
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
                w: W::Remote(near_workspaces::custom(&cfg.rpc_url).await?),
                record,
                record_path,
                verbose: true,
            };
            if cfg.network == Network::Sandbox {
                return Err("verify reads a live network; use rehearse for the sandbox".into());
            }
            let ids = ids(&cfg, &ctx_owner);
            let r = verify::verify(&chain, &cfg, &base, &ids).await?;
            println!("{}", if r.passed() { "ALL CHECKS PASSED" } else { "SOME CHECKS FAILED" });
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

fn write_e2e(ctx: &Ctx, rep: &e2e::E2eReport) -> Res<()> {
    let p = ctx
        .chain
        .record_path
        .with_file_name(format!("e2e.{}.json", ctx.chain.network.as_str()));
    std::fs::write(&p, serde_json::to_string_pretty(rep)? + "\n")?;
    println!("end-to-end record: {}", p.display());
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
        assert_eq!(code_hash(b""), "GKot5hBsd81kMupNCXHaqbhv3huEbxAFMLnpcX2hniwn");
    }
}
