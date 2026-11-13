//! The deployment steps. Each step reads the chain first and does nothing if its result
//! is already there, so the whole deployment can be run again after a failure.

use near_workspaces::Account;
use near_workspaces::operations::Function;
use near_workspaces::types::{AccessKey, Gas, NearToken};
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::Res;
use crate::chain::{
    BYTE_COST, Chain, ContractRec, EscrowRec, MakerRec, NO_CODE, RetiredEscrow, code_hash,
};
use crate::config::{Config, Network, ONE_NEAR, fmt_near, parse_near, resolve};
use crate::keys::KeyStore;

/// What the tool knows about a deployment while it runs.
pub struct Ctx {
    pub chain: Chain,
    pub cfg: Config,
    pub base: PathBuf,
    pub keys: KeyStore,
    pub owner: Account,
}

/// The ids of the accounts in a config, before anything is deployed.
pub struct Ids {
    pub escrow: String,
    pub token: String,
    pub intents: String,
    pub maker: String,
}

pub fn ids(cfg: &Config, owner: &str) -> Ids {
    let sub = |s: &str| format!("{s}.{owner}");
    Ids {
        escrow: sub(&cfg.escrow.sub_account),
        token: if cfg.token.deploy {
            sub(&cfg.token.sub_account)
        } else {
            cfg.token.account_id.clone()
        },
        intents: if cfg.intents.deploy {
            sub(&cfg.intents.sub_account)
        } else {
            cfg.intents.account_id.clone()
        },
        maker: sub(&cfg.maker.sub_account),
    }
}

pub fn read_wasm(base: &Path, p: &str) -> Res<Vec<u8>> {
    let path = resolve(base, p)?;
    std::fs::read(&path).map_err(|e| format!("read wasm {}: {e}", path.display()).into())
}

/// Picks the owner account, creating and funding it through the faucet on testnet.
pub async fn owner_account(chain: &mut Chain, cfg: &Config, keys: &KeyStore) -> Res<Account> {
    let id = if !cfg.owner_account.is_empty() {
        cfg.owner_account.clone()
    } else if !chain.record.owner.is_empty() {
        chain.record.owner.clone()
    } else if chain.network == Network::Testnet {
        let mut r = [0u8; 5];
        rand::Rng::fill(&mut rand::thread_rng(), &mut r);
        format!("zecret-{}.testnet", hex::encode(r))
    } else {
        return Err("owner_account is not set".into());
    };
    let sk = match chain.network {
        Network::Testnet => keys.get_or_create(&id)?,
        _ => keys
            .get(&id)?
            .ok_or_else(|| format!("no key for {id} in {}", keys.dir().display()))?,
    };
    if chain.rpc.account(&id).await?.is_none() {
        if chain.network != Network::Testnet {
            return Err(format!("owner account {id} does not exist").into());
        }
        println!("creating owner {id} through the testnet faucet");
        chain
            .rpc
            .faucet_create(&cfg.faucet_url, &id, &sk.public_key().to_string())
            .await?;
    }
    let keys_on_chain = chain.rpc.access_keys(&id).await?;
    if !keys_on_chain.contains(&sk.public_key().to_string()) {
        return Err(format!("the stored key of {id} is not an access key of that account").into());
    }
    chain.record.owner = id.clone();
    chain.save()?;
    chain.w.account(&id, sk)
}

/// Adds faucet accounts' funds to the owner until it holds `need` (testnet only). Each
/// faucet account is created, then deleted with the owner as beneficiary, which moves
/// its whole balance to the owner in one transaction.
pub async fn top_up_from_faucet(ctx: &mut Ctx, need: u128) -> Res<()> {
    let owner_id = ctx.owner.id().to_string();
    let mut used = 0;
    loop {
        let have = ctx.chain.rpc.balance(&owner_id).await?;
        if have >= need {
            println!(
                "owner {owner_id} holds {} (needs {})",
                fmt_near(have),
                fmt_near(need)
            );
            return Ok(());
        }
        if ctx.chain.network != Network::Testnet {
            return Err(format!(
                "owner {owner_id} holds {}, the deployment needs about {}. Add funds and run again.",
                fmt_near(have),
                fmt_near(need)
            )
            .into());
        }
        if used >= ctx.cfg.max_faucet_accounts {
            return Err(format!(
                "owner holds {} after {used} faucet accounts, needs {}; raise max_faucet_accounts or run again",
                fmt_near(have),
                fmt_near(need)
            )
            .into());
        }
        // A faucet account left by an earlier run (its key is still stored) is used
        // first; otherwise a new one is made.
        let fid = match leftover_faucet_account(ctx).await? {
            Some(fid) => {
                println!("using faucet account {fid} left by an earlier run");
                fid
            }
            None => {
                let mut r = [0u8; 6];
                rand::Rng::fill(&mut rand::thread_rng(), &mut r);
                let fid = format!("zecret-fund-{}.testnet", hex::encode(r));
                let sk = ctx.keys.get_or_create(&fid)?;
                ctx.chain
                    .rpc
                    .faucet_create(&ctx.cfg.faucet_url, &fid, &sk.public_key().to_string())
                    .await?;
                used += 1;
                fid
            }
        };
        let got = ctx.chain.rpc.balance(&fid).await?;
        println!("faucet account {fid} holds {}", fmt_near(got));
        let sk = ctx.keys.get(&fid)?.ok_or("faucet key vanished")?;
        let fund = ctx.chain.w.account(&fid, sk)?;
        let mut attempt = 0;
        let o = loop {
            attempt += 1;
            let r = ctx
                .chain
                .track(
                    &format!("faucet {fid} -> owner (delete account)"),
                    true,
                    fund.batch(fund.id())
                        .delete_account(ctx.owner.id())
                        .transact_async(),
                )
                .await;
            match r {
                Ok(o) => break o,
                Err(e) if e.is::<crate::chain::Dropped>() && attempt < 4 => {
                    println!("  {e}; sending again");
                    if ctx.chain.rpc.account(&fid).await?.is_none() {
                        return Err(
                            format!("{fid} is gone although its delete was not seen").into()
                        );
                    }
                }
                Err(e) => return Err(e),
            }
        };
        // The gas refund goes to the signer, which no longer exists, so NEAR reports that
        // one refund receipt as failed (AccountDoesNotExist on the deleted account). The
        // transaction itself must have succeeded and that must be the only failure.
        let only_refund = o
            .receipt_failures
            .iter()
            .all(|f| f.starts_with(&format!("{fid}:")) && f.contains("AccountDoesNotExist"));
        if !o.ok || !only_refund {
            return Err(format!(
                "faucet top-up {} failed: {:?} {:?}",
                o.rec.hash, o.failure, o.receipt_failures
            )
            .into());
        }
        if !o.receipt_failures.is_empty() {
            println!("  (the failed receipt is the gas refund to the deleted account; expected)");
        }
        let _ = std::fs::remove_file(ctx.keys.dir().join(format!("{fid}.json")));
    }
}

/// A `zecret-fund-*` account whose key is in the key store and that still exists.
async fn leftover_faucet_account(ctx: &Ctx) -> Res<Option<String>> {
    for e in std::fs::read_dir(ctx.keys.dir())? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".json")
            && id.starts_with("zecret-fund-")
        {
            if ctx.chain.rpc.account(id).await?.is_some() {
                return Ok(Some(id.to_string()));
            }
            // Deleted already: its key opens nothing.
            let _ = std::fs::remove_file(ctx.keys.dir().join(&name));
        }
    }
    Ok(None)
}

/// Creates `id` as a sub-account of the owner with a full-access key kept in the key
/// store, unless it exists.
async fn ensure_subaccount(ctx: &mut Ctx, id: &str, balance: u128, step: &str) -> Res<Account> {
    let sk = match ctx.keys.get(id)? {
        Some(sk) => sk,
        None => {
            if ctx.chain.rpc.account(id).await?.is_some() {
                return Err(format!(
                    "{id} exists but its key is not in {}",
                    ctx.keys.dir().display()
                )
                .into());
            }
            ctx.keys.get_or_create(id)?
        }
    };
    if ctx.chain.rpc.account(id).await?.is_none() {
        let o = ctx
            .chain
            .track(
                step,
                true,
                ctx.owner
                    .batch(&id.parse()?)
                    .create_account()
                    .transfer(NearToken::from_yoctonear(balance))
                    .add_key(sk.public_key(), AccessKey::full_access())
                    .transact_async(),
            )
            .await?;
        o.require_ok()?;
    } else {
        println!("{id} exists");
    }
    ctx.chain.w.account(id, sk)
}

/// The amount of NEAR this run will move out of the owner account (rough upper bound).
pub async fn funds_needed(ctx: &Ctx, ids: &Ids) -> Res<u128> {
    let rpc = &ctx.chain.rpc;
    let mut need: u128 = ONE_NEAR / 2; // gas and registrations
    if ctx.cfg.token.deploy && rpc.account(&ids.token).await?.is_none() {
        need += read_wasm(&ctx.base, &ctx.cfg.token.wasm)?.len() as u128 * BYTE_COST
            + parse_near(&ctx.cfg.token.margin_near)?;
    }
    if ctx.cfg.intents.deploy && rpc.account(&ids.intents).await?.is_none() {
        need += read_wasm(&ctx.base, &ctx.cfg.intents.wasm)?.len() as u128 * BYTE_COST
            + parse_near(&ctx.cfg.intents.margin_near)?;
    }
    if rpc.account(&ids.escrow).await?.is_none() {
        need += read_wasm(&ctx.base, &ctx.cfg.escrow.wasm)?.len() as u128 * BYTE_COST
            + parse_near(&ctx.cfg.escrow.margin_near)?;
    }
    if rpc.account(&ids.maker).await?.is_none() {
        need += parse_near(&ctx.cfg.maker.initial_balance_near)?;
    }
    Ok(need)
}

/// The stand-in token: create, deploy the wasm and call `new` in one transaction from
/// the token account.
async fn ensure_token(ctx: &mut Ctx, id: &str) -> Res<()> {
    if !ctx.cfg.token.deploy {
        let ch = ctx
            .chain
            .rpc
            .code_hash(id)
            .await?
            .ok_or_else(|| format!("token {id} does not exist"))?;
        ctx.chain.record.token = Some(ContractRec {
            account_id: id.into(),
            code_hash: ch,
            wasm_bytes: 0,
            deployed_by_this_tool: false,
            note: "existing token".into(),
        });
        return check_token_metadata(ctx, id).await;
    }
    let wasm = read_wasm(&ctx.base, &ctx.cfg.token.wasm)?;
    let want = code_hash(&wasm);
    let bal = wasm.len() as u128 * BYTE_COST + parse_near(&ctx.cfg.token.margin_near)?;
    let acc = ensure_subaccount(ctx, id, bal, "create token account").await?;
    let have = ctx.chain.rpc.code_hash(id).await?.unwrap_or_default();
    if have == NO_CODE {
        ctx.chain
            .track(
                "token: deploy + new (one transaction)",
                true,
                acc.batch(acc.id())
                    .deploy(&wasm)
                    .call(
                        Function::new("new")
                            .args_json(json!({}))
                            .gas(Gas::from_tgas(50)),
                    )
                    .transact_async(),
            )
            .await?
            .require_ok()?;
    } else if have != want {
        return Err(format!("token {id} has code {have}, the local wasm is {want}").into());
    } else if ctx
        .chain
        .rpc
        .view(id, "ft_metadata", &json!({}))
        .await?
        .is_err()
    {
        ctx.chain
            .track(
                "token: new",
                true,
                acc.call(acc.id(), "new")
                    .args_json(json!({}))
                    .transact_async(),
            )
            .await?
            .require_ok()?;
    } else {
        println!("token {id}: code {want} deployed and initialised");
    }
    ctx.chain.record.token = Some(ContractRec {
        account_id: id.into(),
        code_hash: want,
        wasm_bytes: wasm.len() as u64,
        deployed_by_this_tool: true,
        note: "stand-in NEP-141 (deploy/test-usdc), 6 decimals, open mint; no value".into(),
    });
    ctx.chain.save()?;
    check_token_metadata(ctx, id).await
}

async fn check_token_metadata(ctx: &Ctx, id: &str) -> Res<()> {
    let m = ctx.chain.rpc.view_ok(id, "ft_metadata", &json!({})).await?;
    let d = m["decimals"].as_u64().unwrap_or(0);
    if d != ctx.cfg.token.decimals as u64 {
        return Err(format!(
            "token {id} has {d} decimals, config says {}",
            ctx.cfg.token.decimals
        )
        .into());
    }
    Ok(())
}

/// The stand-in Intents Verifier: the code live on intents.near, initialised as the
/// sandbox tests do (wNEAR id, zero fee, fee collector = itself).
async fn ensure_intents(ctx: &mut Ctx, id: &str) -> Res<()> {
    if !ctx.cfg.intents.deploy {
        let ch = ctx
            .chain
            .rpc
            .code_hash(id)
            .await?
            .ok_or_else(|| format!("Verifier {id} does not exist"))?;
        ctx.chain.record.intents = Some(ContractRec {
            account_id: id.into(),
            code_hash: ch,
            wasm_bytes: 0,
            deployed_by_this_tool: false,
            note: "existing Verifier".into(),
        });
        ctx.chain.save()?;
        return Ok(());
    }
    let wasm = read_wasm(&ctx.base, &ctx.cfg.intents.wasm)?;
    let want = code_hash(&wasm);
    let bal = wasm.len() as u128 * BYTE_COST + parse_near(&ctx.cfg.intents.margin_near)?;
    let acc = ensure_subaccount(ctx, id, bal, "create Verifier account").await?;
    let init = Function::new("new")
        .args_json(json!({ "config": {
            "wnear_id": ctx.cfg.intents.wnear_id,
            "fees": { "fee": 0, "fee_collector": id },
        }}))
        .gas(Gas::from_tgas(250));
    let have = ctx.chain.rpc.code_hash(id).await?.unwrap_or_default();
    if have == NO_CODE {
        ctx.chain
            .track(
                "Verifier: deploy intents.near code + new (one transaction)",
                true,
                acc.batch(acc.id())
                    .deploy(&wasm)
                    .call(init)
                    .transact_async(),
            )
            .await?
            .require_ok()?;
    } else if have != want {
        return Err(format!("Verifier {id} has code {have}, the local wasm is {want}").into());
    } else if ctx
        .chain
        .rpc
        .view(id, "current_salt", &json!({}))
        .await?
        .is_err()
    {
        ctx.chain
            .track(
                "Verifier: new",
                true,
                acc.batch(acc.id()).call(init).transact_async(),
            )
            .await?
            .require_ok()?;
    } else {
        println!("Verifier {id}: code {want} deployed and initialised");
    }
    ctx.chain.record.intents = Some(ContractRec {
        account_id: id.into(),
        code_hash: want,
        wasm_bytes: wasm.len() as u64,
        deployed_by_this_tool: true,
        note: format!(
            "stand-in: the code of mainnet intents.near (github.com/near/intents, MIT), wnear_id {}, fee 0; keeps its access key",
            ctx.cfg.intents.wnear_id
        ),
    });
    ctx.chain.save()?;
    Ok(())
}

/// The escrow. Its account is created with one temporary full-access key. Then the
/// account sends one transaction to itself: deploy the code, call `new` (private) and
/// delete that key. After it the account has no keys.
async fn ensure_escrow(ctx: &mut Ctx, id: &str, intents: &str) -> Res<()> {
    let wasm = read_wasm(&ctx.base, &ctx.cfg.escrow.wasm)?;
    let want = code_hash(&wasm);
    let have = ctx.chain.rpc.code_hash(id).await?;
    if let Some(have) = &have
        && have != NO_CODE
    {
        if *have != want {
            return Err(format!(
                "escrow {id} already holds code {have}; the local wasm is {want}. \
                 A deployed escrow cannot change. Use a new escrow sub_account."
            )
            .into());
        }
        println!("escrow {id}: code {want} already deployed");
    } else {
        let bal = wasm.len() as u128 * BYTE_COST + parse_near(&ctx.cfg.escrow.margin_near)?;
        let acc =
            ensure_subaccount(ctx, id, bal, "create escrow account (one temporary key)").await?;
        let keys = ctx.chain.rpc.access_keys(id).await?;
        let pk = acc.secret_key().public_key();
        if keys != vec![pk.to_string()] {
            return Err(format!(
                "escrow {id} must have exactly the stored key before deploy; has {keys:?}"
            )
            .into());
        }
        ctx.chain
            .track(
                "escrow: deploy + new + delete key (one transaction)",
                true,
                acc.batch(acc.id())
                    .deploy(&wasm)
                    .call(
                        Function::new("new")
                            .args_json(json!({ "intents_account": intents }))
                            .gas(Gas::from_tgas(30)),
                    )
                    .delete_key(pk)
                    .transact_async(),
            )
            .await?
            .require_ok()?;
        // The key no longer opens anything; keep no copy of it.
        let _ = std::fs::remove_file(ctx.keys.dir().join(format!("{id}.json")));
    }
    let keys = ctx.chain.rpc.access_keys(id).await?;
    let ia = ctx
        .chain
        .rpc
        .view_ok(id, "get_intents_account", &json!({}))
        .await?;
    // A new escrow account (a new sub_account): keep the old record.
    if let Some(old) = ctx.chain.record.escrow.take()
        && old.account_id != id
    {
        println!(
            "escrow {} (code {}) is kept as a retired escrow; {id} replaces it",
            old.account_id, old.code_hash
        );
        ctx.chain.record.retired_escrows.push(RetiredEscrow {
            escrow: old,
            retired_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            replaced_by: id.into(),
        });
    }
    ctx.chain.record.escrow = Some(EscrowRec {
        account_id: id.into(),
        code_hash: want,
        wasm_bytes: wasm.len() as u64,
        intents_account: ia.as_str().unwrap_or("").into(),
        access_keys: keys.len(),
    });
    ctx.chain.save()?;
    Ok(())
}

/// Registers `who` for storage on the token, paid by the owner, unless registered.
pub async fn ensure_registered(ctx: &mut Ctx, token: &str, who: &str) -> Res<()> {
    let sb = ctx
        .chain
        .rpc
        .view_ok(token, "storage_balance_of", &json!({ "account_id": who }))
        .await?;
    if !sb.is_null() {
        return Ok(());
    }
    let bounds = ctx
        .chain
        .rpc
        .view_ok(token, "storage_balance_bounds", &json!({}))
        .await?;
    let min: u128 = bounds["min"]
        .as_str()
        .ok_or("storage_balance_bounds without min")?
        .parse()?;
    ctx.chain
        .track(
            &format!("token.storage_deposit for {who}"),
            true,
            ctx.owner
                .call(&token.parse()?, "storage_deposit")
                .args_json(json!({ "account_id": who, "registration_only": true }))
                .deposit(NearToken::from_yoctonear(min))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    Ok(())
}

pub async fn ft_balance(ctx: &Ctx, token: &str, who: &str) -> Res<u128> {
    let v = ctx
        .chain
        .rpc
        .view_ok(token, "ft_balance_of", &json!({ "account_id": who }))
        .await?;
    Ok(v.as_str().ok_or("ft_balance_of: not a string")?.parse()?)
}

pub async fn storage_funds(ctx: &Ctx, escrow: &str, who: &str, token: &str) -> Res<u128> {
    let v = ctx
        .chain
        .rpc
        .view_ok(
            escrow,
            "get_storage_funds",
            &json!({ "account_id": who, "token": token }),
        )
        .await?;
    Ok(v.as_str()
        .ok_or("get_storage_funds: not a string")?
        .parse()?)
}

/// The maker: account, token registration, tokens (minted with the stand-in token) and
/// escrow storage funds.
async fn ensure_maker(ctx: &mut Ctx, ids: &Ids) -> Res<()> {
    let bal = parse_near(&ctx.cfg.maker.initial_balance_near)?;
    let maker = ensure_subaccount(ctx, &ids.maker, bal, "create maker account").await?;
    ensure_registered(ctx, &ids.token, &ids.maker).await?;

    let target: u128 = ctx.cfg.maker.token_target.parse()?;
    let mut have = ft_balance(ctx, &ids.token, &ids.maker).await?;
    if ctx.cfg.token.deploy {
        while have < target {
            let amount = (target - have).min(test_usdc_max_mint());
            ctx.chain
                .track(
                    &format!("token.mint {amount} to maker"),
                    true,
                    maker
                        .call(&ids.token.parse()?, "mint")
                        .args_json(json!({ "amount": amount.to_string() }))
                        .transact_async(),
                )
                .await?
                .require_ok()?;
            have = ft_balance(ctx, &ids.token, &ids.maker).await?;
        }
    } else if have < target {
        println!(
            "NOTE: maker {} holds {have} of {}, below the target {target}. Send tokens to it.",
            ids.maker, ids.token
        );
    }

    let want = parse_near(&ctx.cfg.maker.storage_funds_near)?;
    let funds = storage_funds(ctx, &ids.escrow, &ids.maker, &ids.token).await?;
    // The escrow takes the storage of the funds entry itself (about 0.001 NEAR) from
    // the first deposit, so the balance ends a little under the target. Top up only when
    // it is more than 0.01 NEAR under, so a second run changes nothing.
    if funds + ONE_NEAR / 100 < want {
        let add = want - funds;
        let maker_bal = ctx.chain.rpc.balance(&ids.maker).await?;
        if maker_bal < add + ONE_NEAR / 10 {
            return Err(format!(
                "maker holds {}, needs {} for storage funds plus gas",
                fmt_near(maker_bal),
                fmt_near(add)
            )
            .into());
        }
        ctx.chain
            .track(
                "escrow.add_storage_funds (maker)",
                true,
                maker
                    .call(&ids.escrow.parse()?, "add_storage_funds")
                    .args_json(json!({ "token": ids.token }))
                    .deposit(NearToken::from_yoctonear(add))
                    .transact_async(),
            )
            .await?
            .require_ok()?;
    }
    ctx.chain.record.maker = Some(MakerRec {
        account_id: ids.maker.clone(),
        public_key: maker.secret_key().public_key().to_string(),
        token_balance: ft_balance(ctx, &ids.token, &ids.maker).await?.to_string(),
        storage_funds_yocto: storage_funds(ctx, &ids.escrow, &ids.maker, &ids.token)
            .await?
            .to_string(),
    });
    ctx.chain.save()?;
    Ok(())
}

/// The largest single `mint` of the stand-in token (test-usdc `MAX_MINT`).
pub fn test_usdc_max_mint() -> u128 {
    10_000 * 1_000_000
}

/// Runs every step in order.
pub async fn deploy(ctx: &mut Ctx) -> Res<()> {
    let ids = ids(&ctx.cfg, ctx.owner.id().as_str());
    println!(
        "accounts: escrow {}  token {}  Verifier {}  maker {}",
        ids.escrow, ids.token, ids.intents, ids.maker
    );
    let need = funds_needed(ctx, &ids).await?;
    top_up_from_faucet(ctx, need).await?;
    ensure_token(ctx, &ids.token).await?;
    ensure_intents(ctx, &ids.intents).await?;
    ensure_escrow(ctx, &ids.escrow, &ids.intents).await?;
    for who in [ids.escrow.clone(), ids.intents.clone()] {
        ensure_registered(ctx, &ids.token, &who).await?;
    }
    ensure_maker(ctx, &ids).await?;
    Ok(())
}
