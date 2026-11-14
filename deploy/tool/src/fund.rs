//! `fund`: tops up any existing testnet account from the testnet faucet.
//!
//! The tool creates faucet accounts (the NEAR helper service gives each one 10 NEAR) and
//! deletes each one with the target as beneficiary. A delete moves the whole balance of
//! the faucet account, less the gas of the delete, to the target in one transaction.

use crate::Res;
use crate::chain::{Chain, Deployment, W};
use crate::config::{self, Config, Network, ONE_NEAR, fmt_near, parse_near};
use crate::deploy::drain_one_faucet_account;
use crate::keys::KeyStore;
use crate::rpc::Rpc;

/// What the testnet faucet gives one new account. Used only to plan the number of
/// faucet accounts; the tool counts what each account really holds.
pub const FAUCET_NEAR: u128 = 10 * ONE_NEAR;

/// What a delete may cost the beneficiary: the gas of the delete transaction. The tool
/// counts a faucet account as `balance - DELETE_ALLOWANCE`, so the count is never more
/// than what the target received. Measured cost is about 0.0005 NEAR.
pub const DELETE_ALLOWANCE: u128 = ONE_NEAR / 100;

/// The amount a faucet account with `balance` adds to the target, counted low.
pub fn credit(balance: u128) -> u128 {
    balance.saturating_sub(DELETE_ALLOWANCE)
}

/// The number of new faucet accounts needed for `amount`, if each holds `FAUCET_NEAR`.
pub fn accounts_needed(amount: u128) -> u128 {
    let per = credit(FAUCET_NEAR);
    amount.div_ceil(per)
}

/// Parses and checks the amount: a NEAR amount above 0 that `max_accounts` faucet
/// accounts can cover.
pub fn check_amount(s: &str, max_accounts: u32) -> Res<u128> {
    let amount = parse_near(s)?;
    if amount == 0 {
        return Err("the amount must be more than 0 NEAR".into());
    }
    let n = accounts_needed(amount);
    if n > max_accounts as u128 {
        return Err(format!(
            "{} needs {n} faucet accounts; max_faucet_accounts in the config is {max_accounts}",
            fmt_near(amount)
        )
        .into());
    }
    Ok(amount)
}

/// Checks the target account id. It must be a valid NEAR account id and must not be a
/// mainnet name.
pub fn check_target(id: &str) -> Res<String> {
    let parsed: near_workspaces::AccountId = id
        .parse()
        .map_err(|e| format!("{id:?} is not a valid account id: {e}"))?;
    let s = parsed.to_string();
    if s == "near" || s.ends_with(".near") {
        return Err(format!("{s} is a mainnet name; fund tops up testnet accounts only").into());
    }
    Ok(s)
}

/// Balance in yoctoNEAR and block height of `id` at a final block; `None` if the account
/// does not exist.
async fn final_balance(rpc: &Rpc, id: &str) -> Res<Option<(u128, u64)>> {
    Ok(match rpc.account(id).await? {
        Some(v) => Some((
            v["amount"].as_str().unwrap_or("0").parse()?,
            v["block_height"].as_u64().unwrap_or(0),
        )),
        None => None,
    })
}

/// Runs `fund`. The caller has checked that the config is for testnet.
pub async fn run(cfg: &Config, base: &std::path::Path, target: &str, amount: &str) -> Res<()> {
    if cfg.network != Network::Testnet {
        return Err("fund runs on testnet only: it uses the testnet faucet".into());
    }
    let target = check_target(target)?;
    let amount = check_amount(amount, cfg.max_faucet_accounts)?;
    let keys = KeyStore::open(&config::resolve(base, &cfg.keys_dir)?)?;
    let mut chain = Chain {
        network: cfg.network,
        rpc: Rpc::new(&cfg.rpc_url)?,
        w: W::remote(cfg.network, &cfg.rpc_url).await?,
        // The record is never saved: fund sends its transactions with record = false.
        record: Deployment::default(),
        record_path: config::resolve(base, &cfg.deployment_file)?,
        verbose: true,
    };
    let (before, before_h) = final_balance(&chain.rpc, &target)
        .await?
        .ok_or_else(|| format!("{target} does not exist on testnet; not funding it"))?;
    println!(
        "network testnet  rpc {}  target {target}  amount {}",
        cfg.rpc_url,
        fmt_near(amount)
    );
    println!(
        "before: {target} holds {} at final block {before_h}",
        fmt_near(before)
    );
    let mut received: u128 = 0;
    let mut created: u32 = 0;
    let mut txs = Vec::new();
    while received < amount {
        if created >= cfg.max_faucet_accounts {
            return Err(format!(
                "{target} received about {} from {created} faucet accounts (tx {}), less than {}; \
                 max_faucet_accounts is reached. Run again for the rest.",
                fmt_near(received),
                txs.join(", "),
                fmt_near(amount)
            )
            .into());
        }
        let d = drain_one_faucet_account(&mut chain, cfg, &keys, &target, &target, false).await?;
        if d.created {
            created += 1;
        }
        received += credit(d.balance);
        println!(
            "  {} -> {target}: about {} (tx {})",
            d.faucet_account,
            fmt_near(credit(d.balance)),
            d.tx_hash
        );
        txs.push(d.tx_hash);
    }
    let (after, after_h) = final_balance(&chain.rpc, &target)
        .await?
        .ok_or_else(|| format!("{target} is gone"))?;
    println!(
        "after:  {target} holds {} at final block {after_h}",
        fmt_near(after)
    );
    println!(
        "received at least {} through {} faucet account(s); transactions: {}",
        fmt_near(received),
        txs.len(),
        txs.join(", ")
    );
    println!(
        "(the balance change also includes what {target} itself spent or received between the two blocks)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_counts_low() {
        assert_eq!(credit(FAUCET_NEAR), 10 * ONE_NEAR - ONE_NEAR / 100);
        assert_eq!(credit(ONE_NEAR / 1000), 0);
        assert_eq!(credit(0), 0);
    }

    #[test]
    fn faucet_accounts_for_an_amount() {
        assert_eq!(accounts_needed(ONE_NEAR), 1);
        assert_eq!(accounts_needed(parse_near("9.99").unwrap()), 1);
        // 10 NEAR is more than one faucet account gives after the delete gas.
        assert_eq!(accounts_needed(10 * ONE_NEAR), 2);
        assert_eq!(accounts_needed(parse_near("19.98").unwrap()), 2);
        assert_eq!(accounts_needed(parse_near("19.99").unwrap()), 3);
    }

    #[test]
    fn amount_checks() {
        assert_eq!(check_amount("4", 6).unwrap(), 4 * ONE_NEAR);
        assert!(check_amount("0", 6).is_err());
        assert!(check_amount("0.0", 6).is_err());
        assert!(check_amount("-1", 6).is_err());
        assert!(check_amount("ten", 6).is_err());
        assert!(check_amount("59.94", 6).is_ok());
        let e = check_amount("60", 6).unwrap_err().to_string();
        assert!(e.contains("needs 7 faucet accounts"), "{e}");
        assert!(check_amount("1", 0).is_err());
    }

    #[test]
    fn target_checks() {
        assert_eq!(
            check_target("maker.zecret-3f109eea1e.testnet").unwrap(),
            "maker.zecret-3f109eea1e.testnet"
        );
        assert!(check_target("Bad Name").is_err());
        assert!(check_target("").is_err());
        assert!(check_target("maker.near").is_err());
        assert!(check_target("near").is_err());
    }
}
