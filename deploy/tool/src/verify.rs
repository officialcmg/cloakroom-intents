//! Read-only checks of a deployment at final blocks. They spend nothing.

use serde_json::json;

use crate::Res;
use crate::chain::{Chain, NO_CODE, code_hash};
use crate::config::{Config, fmt_near};
use crate::deploy::{Ids, read_wasm};
use std::path::Path;

pub struct Report {
    pub lines: Vec<(bool, String)>,
}

impl Report {
    fn check(&mut self, ok: bool, what: String) {
        println!("  [{}] {what}", if ok { "PASS" } else { "FAIL" });
        self.lines.push((ok, what));
    }
    fn info(&mut self, what: String) {
        println!("  [info] {what}");
    }
    pub fn passed(&self) -> bool {
        self.lines.iter().all(|(ok, _)| *ok)
    }
}

/// The three checks a taker makes before funding (escrow README, "Deployment"), plus
/// checks of the token, the Verifier and the maker.
pub async fn verify(chain: &Chain, cfg: &Config, base: &Path, ids: &Ids) -> Res<Report> {
    let rpc = &chain.rpc;
    let mut r = Report { lines: vec![] };
    let fb = rpc.block(None, "final").await?;
    println!(
        "verify {} at final block {} ({})",
        chain.network.as_str(),
        fb["header"]["height"],
        fb["header"]["hash"].as_str().unwrap_or("")
    );

    // Escrow: the taker's checks.
    let wasm = read_wasm(base, &cfg.escrow.wasm)?;
    let local = code_hash(&wasm);
    println!(
        "escrow {} (local wasm {} bytes, code hash {local})",
        ids.escrow,
        wasm.len()
    );
    match rpc.code_hash(&ids.escrow).await? {
        None => r.check(false, format!("escrow account {} exists", ids.escrow)),
        Some(h) => {
            r.check(
                h == local,
                format!("escrow code hash {h} equals the local wasm"),
            );
            if let Some(rec) = &chain.record.escrow {
                r.check(
                    rec.code_hash == h,
                    format!("escrow code hash equals the recorded {}", rec.code_hash),
                );
            }
            let keys = rpc.access_keys(&ids.escrow).await?;
            r.check(
                keys.is_empty(),
                format!("escrow has no access keys (found {})", keys.len()),
            );
            let ia = rpc
                .view(&ids.escrow, "get_intents_account", &json!({}))
                .await?;
            match ia {
                Ok(v) => r.check(
                    v.as_str() == Some(ids.intents.as_str()),
                    format!(
                        "escrow get_intents_account = {v} (expected {})",
                        ids.intents
                    ),
                ),
                Err(e) => r.check(false, format!("escrow get_intents_account: {e}")),
            }
        }
    }

    // Token.
    match rpc.view(&ids.token, "ft_metadata", &json!({})).await? {
        Ok(m) => {
            r.check(
                m["decimals"].as_u64() == Some(cfg.token.decimals as u64),
                format!(
                    "token {} has {} decimals (symbol {})",
                    ids.token, m["decimals"], m["symbol"]
                ),
            );
        }
        Err(e) => r.check(false, format!("token {} ft_metadata: {e}", ids.token)),
    }
    if cfg.token.deploy {
        let local = code_hash(&read_wasm(base, &cfg.token.wasm)?);
        let h = rpc.code_hash(&ids.token).await?.unwrap_or_default();
        r.check(
            h == local,
            format!("token code hash {h} equals local test-usdc wasm"),
        );
    }
    for who in [&ids.escrow, &ids.intents, &ids.maker] {
        let sb = rpc
            .view(
                &ids.token,
                "storage_balance_of",
                &json!({ "account_id": who }),
            )
            .await?;
        r.check(
            matches!(&sb, Ok(v) if !v.is_null()),
            format!("{who} is registered on the token"),
        );
    }

    // Verifier.
    let vh = rpc.code_hash(&ids.intents).await?.unwrap_or_default();
    if cfg.intents.deploy {
        let local = code_hash(&read_wasm(base, &cfg.intents.wasm)?);
        r.check(
            vh == local,
            format!(
                "Verifier {} code hash {vh} equals local intents.near wasm",
                ids.intents
            ),
        );
    } else {
        r.check(
            vh != NO_CODE && !vh.is_empty(),
            format!("Verifier {} has code {vh}", ids.intents),
        );
    }
    match rpc.view(&ids.intents, "current_salt", &json!({})).await? {
        Ok(s) => r.check(true, format!("Verifier current_salt = {s}")),
        Err(e) => r.check(false, format!("Verifier current_salt: {e}")),
    }

    // Maker.
    match rpc.account(&ids.maker).await? {
        None => r.check(false, format!("maker {} exists", ids.maker)),
        Some(a) => {
            r.info(format!(
                "maker {} holds {}",
                ids.maker,
                fmt_near(a["amount"].as_str().unwrap_or("0").parse().unwrap_or(0))
            ));
            let bal = rpc
                .view(
                    &ids.token,
                    "ft_balance_of",
                    &json!({ "account_id": ids.maker }),
                )
                .await?;
            let target: u128 = cfg.maker.token_target.parse()?;
            let have: u128 = bal
                .as_ref()
                .ok()
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if cfg.token.deploy {
                r.check(
                    have > 0,
                    format!("maker token balance {have} (target {target})"),
                );
            } else {
                r.info(format!("maker token balance {have} (target {target})"));
            }
            let funds = rpc
                .view(
                    &ids.escrow,
                    "get_storage_funds",
                    &json!({ "account_id": ids.maker, "token": ids.token }),
                )
                .await?;
            let f: u128 = funds
                .as_ref()
                .ok()
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            r.check(f > 0, format!("maker escrow storage funds {}", fmt_near(f)));
        }
    }
    Ok(r)
}
