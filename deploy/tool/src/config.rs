//! The deployment config (TOML). Paths in it are relative to the config file's folder.
//! A path may start with `$NAME/`, which is replaced by the environment variable NAME.

use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::Res;

pub const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Sandbox,
    Testnet,
    Mainnet,
}

impl Network {
    pub fn as_str(self) -> &'static str {
        match self {
            Network::Sandbox => "sandbox",
            Network::Testnet => "testnet",
            Network::Mainnet => "mainnet",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub network: Network,
    /// JSON-RPC endpoint. Ignored for the sandbox.
    #[serde(default)]
    pub rpc_url: String,
    /// Testnet only: the faucet (NEAR helper service).
    #[serde(default)]
    pub faucet_url: String,
    /// Testnet only: how many faucet accounts one run may use to fund the owner.
    #[serde(default = "default_max_faucet")]
    pub max_faucet_accounts: u32,
    /// The parent of all accounts below. Empty on testnet: the tool picks a name and
    /// creates it through the faucet. Mainnet: an account the owner controls, with its
    /// key in `keys_dir/<owner>.json`.
    #[serde(default)]
    pub owner_account: String,
    pub keys_dir: String,
    pub deployment_file: String,
    pub escrow: EscrowCfg,
    pub token: TokenCfg,
    pub intents: IntentsCfg,
    pub maker: MakerCfg,
}

fn default_max_faucet() -> u32 {
    6
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscrowCfg {
    /// The escrow is `<sub_account>.<owner>`.
    pub sub_account: String,
    /// The reviewed escrow wasm (built from contracts/near-escrow).
    pub wasm: String,
    /// NEAR given to the escrow on top of its code storage. It stays locked in the
    /// account forever, because the account has no keys.
    pub margin_near: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenCfg {
    /// true: deploy the stand-in token (test-usdc) as `<sub_account>.<owner>`.
    /// false: use the existing token `account_id`.
    pub deploy: bool,
    #[serde(default)]
    pub sub_account: String,
    #[serde(default)]
    pub wasm: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default = "default_margin")]
    pub margin_near: String,
    /// Decimals the token must report.
    pub decimals: u8,
    /// true: the existing token (deploy = false) is the stand-in test-usdc, whose `mint`
    /// is open to any account; the tool mints the maker's tokens. Testnet only.
    #[serde(default)]
    pub open_mint: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentsCfg {
    /// true: deploy the Verifier code (from intents.near) as `<sub_account>.<owner>`.
    /// false: use the existing Verifier `account_id` (`intents.near` on mainnet).
    pub deploy: bool,
    #[serde(default)]
    pub sub_account: String,
    #[serde(default)]
    pub wasm: String,
    #[serde(default)]
    pub account_id: String,
    /// The wNEAR token the Verifier is configured with (deploy = true only).
    #[serde(default)]
    pub wnear_id: String,
    #[serde(default = "default_margin")]
    pub margin_near: String,
}

fn default_margin() -> String {
    "0.5".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MakerCfg {
    /// The maker is `<sub_account>.<owner>`.
    pub sub_account: String,
    /// NEAR sent to the maker when the tool creates it.
    pub initial_balance_near: String,
    /// Token units (smallest unit) the maker should hold, outside and inside the
    /// Verifier together. With the stand-in token the tool mints up to this; with a real
    /// token it only reports.
    pub token_target: String,
    /// NEAR of escrow storage funds (`add_storage_funds`) the maker should have.
    pub storage_funds_near: String,
    /// Token units the maker should hold in its own Verifier account. The escrow takes
    /// swaps only from there (`mt_transfer_call`). The tool deposits the difference from
    /// the maker's plain token balance. Default 0: no deposit.
    #[serde(default = "zero")]
    pub verifier_target: String,
}

fn zero() -> String {
    "0".into()
}

impl Config {
    pub fn load(path: &Path) -> Res<(Self, PathBuf)> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("read config {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&text)?;
        let base = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        cfg.check()?;
        Ok((cfg, base))
    }

    fn check(&self) -> Res<()> {
        if self.network != Network::Sandbox && self.rpc_url.is_empty() {
            return Err("rpc_url is required".into());
        }
        if self.network == Network::Mainnet {
            if self.owner_account.is_empty() {
                return Err("mainnet: owner_account is required".into());
            }
            if self.token.deploy || self.intents.deploy {
                return Err(
                    "mainnet: the stand-in token and Verifier are for testnet only; set deploy = false"
                        .into(),
                );
            }
        }
        if !self.token.deploy && self.token.account_id.is_empty() {
            return Err("token.account_id is required when token.deploy = false".into());
        }
        if !self.intents.deploy && self.intents.account_id.is_empty() {
            return Err("intents.account_id is required when intents.deploy = false".into());
        }
        for v in [
            &self.escrow.margin_near,
            &self.token.margin_near,
            &self.intents.margin_near,
            &self.maker.initial_balance_near,
            &self.maker.storage_funds_near,
        ] {
            parse_near(v)?;
        }
        self.maker.token_target.parse::<u128>()?;
        Ok(())
    }
}

/// Resolves a config path: `$NAME/rest` from the environment, otherwise relative to
/// `base`.
pub fn resolve(base: &Path, p: &str) -> Res<PathBuf> {
    if let Some(rest) = p.strip_prefix('$') {
        let (var, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let val =
            std::env::var(var).map_err(|_| format!("environment variable {var} is not set"))?;
        return Ok(PathBuf::from(val).join(tail));
    }
    let pb = PathBuf::from(p);
    Ok(if pb.is_absolute() { pb } else { base.join(pb) })
}

/// Parses a decimal NEAR amount ("4.85") into yoctoNEAR.
pub fn parse_near(s: &str) -> Res<u128> {
    let s = s.trim();
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() && frac.is_empty() {
        return Err(format!("bad NEAR amount {s:?}").into());
    }
    if frac.len() > 24
        || !int.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("bad NEAR amount {s:?}").into());
    }
    let int: u128 = if int.is_empty() { 0 } else { int.parse()? };
    let mut frac_s = frac.to_string();
    while frac_s.len() < 24 {
        frac_s.push('0');
    }
    let frac: u128 = frac_s.parse()?;
    int.checked_mul(ONE_NEAR)
        .and_then(|v| v.checked_add(frac))
        .ok_or_else(|| format!("NEAR amount {s:?} too large").into())
}

/// Formats yoctoNEAR as NEAR with 5 decimals.
pub fn fmt_near(y: u128) -> String {
    let whole = y / ONE_NEAR;
    let frac = (y % ONE_NEAR) / 10u128.pow(19);
    format!("{whole}.{frac:05} NEAR")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_amounts() {
        assert_eq!(parse_near("1").unwrap(), ONE_NEAR);
        assert_eq!(parse_near("0.03").unwrap(), 3 * ONE_NEAR / 100);
        assert_eq!(parse_near("4.85").unwrap(), 485 * ONE_NEAR / 100);
        assert_eq!(parse_near(".5").unwrap(), ONE_NEAR / 2);
        assert!(parse_near("").is_err());
        assert!(parse_near("1.2.3").is_err());
        assert!(parse_near("-1").is_err());
        assert!(parse_near("0.0000000000000000000000001").is_err());
        assert_eq!(fmt_near(ONE_NEAR * 47 / 10), "4.70000 NEAR");
    }

    #[test]
    fn configs_in_repo_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../configs");
        for name in ["testnet.toml", "mainnet.toml", "sandbox.toml"] {
            let (cfg, _) = Config::load(&dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(format!("{}.toml", cfg.network.as_str()), name);
        }
        let (m, _) = Config::load(&dir.join("mainnet.toml")).unwrap();
        assert_eq!(m.intents.account_id, "intents.near");
        assert_eq!(
            m.token.account_id,
            "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
        );
        assert!(!m.token.deploy && !m.intents.deploy);
    }

    #[test]
    fn mainnet_cannot_deploy_stand_ins() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../configs/mainnet.toml"),
        )
        .unwrap()
        .replacen("deploy = false", "deploy = true", 1);
        let cfg: Config = toml::from_str(&text).unwrap();
        assert!(cfg.check().is_err());
    }
}
