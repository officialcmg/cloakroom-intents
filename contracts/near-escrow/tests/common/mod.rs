//! Helpers shared by the sandbox tests (`tests/sandbox.rs` with wNEAR, `tests/usdc.rs` with
//! the USDC code from mainnet). Both run the escrow against the live `intents.near` code
//! (the Verifier) from `$ZECRET_WASM_DIR`.

#![allow(dead_code)]

use ed25519_dalek::{Signer, SigningKey};
use near_workspaces::network::Sandbox;
use near_workspaces::operations::Function;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken, SecretKey};
use near_workspaces::{Account, AccountId, Contract, Worker};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;

pub type R<T> = Result<T, Box<dyn std::error::Error>>;

// Key shares from contracts/test/CloakroomEscrow.t.sol, in the encodings of this contract
// (32-byte little-endian scalars, 32-byte compressed points).
pub const A_SEC: &str = "195661e2e4789b7f57137b2e777736d79720270638d12e4882fd95e52a73d6b3";
pub const B_SEC: &str = "2b8bbfabc504814096b330ffba3d828335e8db236f79aa7fded4d77640369e2a";
pub const A_PUB: &str = "4c303c9656ca22dedd75957bc56c364c6f5461cb0fadca1e589495ebb182a197";
pub const B_PUB: &str = "950e78fd9d3129d85922bdef19d501fbec46b8c0c0f563d8df66f5e5b86d3a18";

/// Gas the clients attach (crates/zecret-core/src/near/escrow.rs).
pub const GAS_OPEN: u64 = 100;
pub const GAS_COMMIT: u64 = 15;
pub const GAS_CLAIM: u64 = 25;
pub const GAS_WITHDRAW: u64 = 25;
pub const GAS_REFUND: u64 = 50;
/// The stipend the maker sends to the taker's new key (docs/experiments/light-payout.md).
pub const STIPEND_MILLINEAR: u128 = 60;

pub fn le_hex(be: &str) -> String {
    let mut b = hex::decode(be).unwrap();
    b.reverse();
    hex::encode(b)
}

pub fn wasm_dir() -> String {
    std::env::var("ZECRET_WASM_DIR")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../.tools/wasm").into())
}

pub fn live_wasm(name: &str) -> Vec<u8> {
    let dir = wasm_dir();
    std::fs::read(format!("{dir}/{name}")).unwrap_or_else(|e| panic!("read {dir}/{name}: {e}"))
}

/// The escrow wasm: `ESCROW_WASM` if set, else built from this crate.
pub fn escrow_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        if let Ok(p) = std::env::var("ESCROW_WASM") {
            return std::fs::read(p).expect("read ESCROW_WASM");
        }
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dir.join("target"));
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
            .arg("--target-dir")
            .arg(&target)
            .current_dir(&dir)
            .env_remove("LD_PRELOAD")
            .status()
            .expect("run cargo build");
        assert!(status.success(), "wasm build failed");
        std::fs::read(target.join("wasm32-unknown-unknown/release/near_escrow.wasm"))
            .expect("read escrow wasm")
    })
}

pub fn tgas(g: Gas) -> f64 {
    g.as_gas() as f64 / 1e12
}

static NAMES: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Gives an account a short name in receipt reports.
pub fn name_account(id: &AccountId, name: &str) {
    NAMES
        .lock()
        .unwrap()
        .push((id.to_string(), name.to_string()));
}

fn name_of(id: &AccountId) -> String {
    NAMES
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(a, _)| a == id.as_str())
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| {
            let s = id.to_string();
            if s.len() == 64 {
                format!("taker {}", &s[..8])
            } else {
                s
            }
        })
}

/// Prints the total gas and every receipt (executor, gas burnt, ok or the error).
pub fn report(label: &str, o: &ExecutionFinalResult) {
    println!(
        "GAS {label:<56} {:>7.2} Tgas  ok={}",
        tgas(o.total_gas_burnt),
        o.is_success()
    );
    println!(
        "    {:<16} {:>6.2} Tgas  (transaction)",
        "",
        tgas(o.outcome().gas_burnt)
    );
    for r in o.receipt_outcomes() {
        let status = if r.is_success() {
            "ok".to_string()
        } else {
            format!("{:?}", r.clone().into_result().err())
        };
        println!(
            "    {:<16} {:>6.2} Tgas  {}",
            name_of(&r.executor_id),
            tgas(r.gas_burnt),
            status
        );
    }
}

/// Reports the call and asserts that it and every receipt succeeded.
pub fn okr(label: &str, o: ExecutionFinalResult) -> ExecutionFinalResult {
    report(label, &o);
    assert!(
        o.is_success() && o.receipt_failures().is_empty(),
        "{label} failed: {:?} / {:?}",
        o.clone().into_result().err(),
        o.receipt_failures()
    );
    o
}

pub fn ok(label: &str, o: ExecutionFinalResult) {
    let _ = okr(label, o);
}

pub fn events(o: &ExecutionFinalResult, name: &str) -> Vec<Value> {
    o.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|e| e["standard"] == "near_escrow" && e["event"] == name)
        .collect()
}

/// The failure messages of the receipts that ran on `executor`.
pub fn failures_at(o: &ExecutionFinalResult, executor: &AccountId) -> Vec<String> {
    o.receipt_outcomes()
        .iter()
        .filter(|r| &r.executor_id == executor && r.is_failure())
        .map(|r| format!("{:?}", r.clone().into_result().err()))
        .collect()
}

/// Deploys the escrow as the README says: the escrow account sends one transaction to
/// itself that deploys the code, calls `new` (private) and deletes its only access key.
pub async fn deploy_escrow(worker: &Worker<Sandbox>, intents: &AccountId) -> R<Contract> {
    let account = worker.dev_create_account().await?;
    let key = account.secret_key().public_key();
    ok(
        "escrow deploy + new + delete key (one transaction)",
        account
            .batch(account.id())
            .deploy(escrow_wasm())
            .call(Function::new("new").args_json(json!({ "intents_account": intents })))
            .delete_key(key)
            .transact()
            .await?,
    );
    name_account(account.id(), "escrow");
    Ok(Contract::from_secret_key(
        account.id().clone(),
        account.secret_key().clone(),
        worker,
    ))
}

/// The live Verifier code, with `vadmin` as super admin and DAO (it can pause features
/// and lock accounts).
pub async fn deploy_verifier(
    worker: &Worker<Sandbox>,
    wnear: &AccountId,
    vadmin: &Account,
) -> R<Contract> {
    let c = worker.dev_deploy(&live_wasm("intents.near.wasm")).await?;
    ok(
        "intents.new",
        c.call("new")
            .args_json(json!({ "config": {
                "wnear_id": wnear,
                "fees": { "fee": 0, "fee_collector": c.id() },
                "roles": { "super_admins": [vadmin.id()], "grantees": {
                    "DAO": [vadmin.id()],
                    "PauseManager": [vadmin.id()],
                    "UnpauseManager": [vadmin.id()],
                    "UnrestrictedAccountLocker": [vadmin.id()],
                    "UnrestrictedAccountUnlocker": [vadmin.id()],
                } },
            }}))
            .gas(Gas::from_tgas(300))
            .transact()
            .await?,
    );
    name_account(c.id(), "verifier");
    Ok(c)
}

/// One test world: a NEP-141 token, the live Verifier, the escrow and a maker.
pub struct Env {
    pub worker: Worker<Sandbox>,
    pub token: Contract,
    pub intents: Contract,
    pub escrow: Contract,
    pub maker: Account,
    /// Super admin and DAO of the Verifier.
    pub vadmin: Account,
}

pub async fn now_s(worker: &Worker<Sandbox>) -> R<u64> {
    Ok(worker.view_block().await?.timestamp() / 1_000_000_000)
}

pub async fn height(worker: &Worker<Sandbox>) -> R<u64> {
    Ok(worker.view_block().await?.height())
}

/// Waits until the sandbox has produced `d` blocks after height `from`.
pub async fn wait_blocks(worker: &Worker<Sandbox>, from: u64, d: u64) -> R<()> {
    while height(worker).await? < from + d {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(())
}

/// Moves sandbox time forward until the block timestamp is at least `target` seconds.
pub async fn advance_to(worker: &Worker<Sandbox>, target: u64) -> R<()> {
    while now_s(worker).await? < target {
        worker.fast_forward(50).await?;
    }
    Ok(())
}

pub async fn ft_balance(token: &Contract, who: &AccountId) -> R<u128> {
    let v: String = token
        .view("ft_balance_of")
        .args_json(json!({ "account_id": who }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

pub fn token_id(env: &Env) -> String {
    format!("nep141:{}", env.token.id())
}

/// Balance of `who` in the Verifier, in the env's token.
pub async fn vbal(env: &Env, who: &str) -> R<u128> {
    let v: String = env
        .intents
        .view("mt_balance_of")
        .args_json(json!({ "account_id": who, "token_id": token_id(env) }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

pub async fn owed(env: &Env) -> R<u128> {
    let v: String = env
        .escrow
        .view("get_owed")
        .args_json(json!({ "token": env.token.id() }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

pub async fn get_swap(env: &Env, id: &str) -> R<Value> {
    Ok(env
        .escrow
        .view("get_swap")
        .args_json(json!({ "id": id }))
        .await?
        .json()?)
}

/// The taker's check before it funds: at one final block, the escrow's Verifier balance
/// covers everything the escrow owes in the token. Returns (balance, owed).
pub async fn assert_backed(env: &Env, label: &str) -> R<(u128, u128)> {
    let b = env.worker.view_block().await?;
    let bal: String = env
        .intents
        .view("mt_balance_of")
        .block_height(b.height())
        .args_json(json!({ "account_id": env.escrow.id(), "token_id": token_id(env) }))
        .await?
        .json()?;
    let owed: String = env
        .escrow
        .view("get_owed")
        .block_height(b.height())
        .args_json(json!({ "token": env.token.id() }))
        .await?
        .json()?;
    let (bal, owed): (u128, u128) = (bal.parse()?, owed.parse()?);
    println!(
        "BACKING {label}: block {} escrow Verifier balance {bal}, owed {owed}",
        b.height()
    );
    assert!(bal >= owed, "{label}: escrow balance {bal} < owed {owed}");
    Ok((bal, owed))
}

/// The maker deposits `amount` of the token into its own Verifier account.
pub async fn maker_deposit(env: &Env, amount: u128) -> R<ExecutionFinalResult> {
    Ok(env
        .maker
        .call(env.token.id(), "ft_transfer_call")
        .args_json(
            json!({ "receiver_id": env.intents.id(), "amount": amount.to_string(), "msg": "" }),
        )
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?)
}

pub struct Taker {
    pub key: SigningKey,
    pub id: String,
}

pub fn taker(seed: u8) -> Taker {
    let key = SigningKey::from_bytes(&[seed; 32]);
    let id = hex::encode(key.verifying_key().to_bytes());
    Taker { key, id }
}

/// The taker's implicit account, signing with the taker key.
pub fn taker_account(t: &Taker, worker: &Worker<Sandbox>) -> R<Account> {
    let mut kp = t.key.to_bytes().to_vec();
    kp.extend_from_slice(&t.key.verifying_key().to_bytes());
    let sk: SecretKey = format!("ed25519:{}", bs58::encode(kp).into_string()).parse()?;
    Ok(Account::from_secret_key(t.id.parse()?, sk, worker))
}

pub fn terms(salt: u8, t: &Taker, t0: u64, t1: u64, t2: u64, w: u64) -> Value {
    json!({
        "salt": hex::encode([salt; 32]),
        "taker_pk": t.id,
        "a": A_PUB,
        "b": B_PUB,
        "t0": t0,
        "t1": t1,
        "t2": t2,
        "reveal_window": w,
    })
}

/// The maker opens a swap: `mt_transfer_call` on the Verifier to the escrow with the
/// terms in `msg`. Returns the swap id and the outcome (not checked).
pub async fn open_msg(
    env: &Env,
    salt: u8,
    msg: String,
    amount: u128,
) -> R<(String, ExecutionFinalResult)> {
    let o = env
        .maker
        .call(env.intents.id(), "mt_transfer_call")
        .args_json(json!({
            "receiver_id": env.escrow.id(),
            "token_id": token_id(env),
            "amount": amount.to_string(),
            "msg": msg,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(GAS_OPEN))
        .transact()
        .await?;
    let id: String = env
        .escrow
        .view("swap_id")
        .args_json(json!({ "maker": env.maker.id(), "salt": hex::encode([salt; 32]) }))
        .await?
        .json()?;
    Ok((id, o))
}

/// Opens a swap with windows far in the future; asserts success. Returns the id.
pub async fn open(env: &Env, salt: u8, t: &Taker, amount: u128) -> R<String> {
    let now = now_s(&env.worker).await?;
    let msg = terms(salt, t, now + 6000, now + 12000, now + 18000, 6000).to_string();
    let (id, o) = open_msg(env, salt, msg, amount).await?;
    let o = okr(
        "open (Verifier mt_transfer_call -> escrow mt_on_transfer)",
        o,
    );
    assert_eq!(events(&o, "opened").len(), 1);
    // The Verifier's mt_transfer_call returns the amounts used: all of it.
    assert_eq!(o.json::<Vec<String>>()?, vec![amount.to_string()]);
    Ok(id)
}

pub async fn set_ready(env: &Env, id: &str) -> R<()> {
    ok(
        "set_ready",
        env.maker
            .call(env.escrow.id(), "set_ready")
            .args_json(json!({ "id": id }))
            .transact()
            .await?,
    );
    Ok(())
}

/// The commitment and the taker key's signature for `commit_claim`.
pub async fn commit_args(env: &Env, t: &Taker, id: &str) -> R<Value> {
    let a = le_hex(A_SEC);
    let commitment: String = env
        .escrow
        .view("commitment_of")
        .args_json(json!({ "id": id, "a": a }))
        .await?
        .json()?;
    let msg: String = env
        .escrow
        .view("commit_message")
        .args_json(json!({ "id": id, "commitment": commitment }))
        .await?
        .json()?;
    let sig = t.key.sign(&hex::decode(&msg)?).to_bytes();
    Ok(json!({ "id": id, "commitment": commitment, "signature": hex::encode(sig) }))
}

pub async fn call(
    by: &Account,
    env: &Env,
    method: &str,
    args: Value,
    tgas: u64,
) -> R<ExecutionFinalResult> {
    Ok(by
        .call(env.escrow.id(), method)
        .args_json(args)
        .gas(Gas::from_tgas(tgas))
        .transact()
        .await?)
}

pub async fn claim(by: &Account, env: &Env, id: &str, tgas: u64) -> R<ExecutionFinalResult> {
    call(
        by,
        env,
        "claim",
        json!({ "id": id, "a": le_hex(A_SEC) }),
        tgas,
    )
    .await
}

pub async fn withdraw(by: &Account, env: &Env, id: &str, tgas: u64) -> R<ExecutionFinalResult> {
    call(by, env, "withdraw", json!({ "id": id }), tgas).await
}

pub async fn refund(by: &Account, env: &Env, id: &str, tgas: u64) -> R<ExecutionFinalResult> {
    call(
        by,
        env,
        "refund",
        json!({ "id": id, "b": le_hex(B_SEC) }),
        tgas,
    )
    .await
}

/// Opens, sets Ready, commits and claims with `claim_gas` (sent by `by`). Returns the
/// taker and the id.
pub async fn claimed(
    env: &Env,
    by: &Account,
    seed: u8,
    amount: u128,
    claim_gas: u64,
) -> R<(Taker, String)> {
    let t = taker(seed);
    let id = open(env, seed, &t, amount).await?;
    set_ready(env, &id).await?;
    ok(
        "commit_claim",
        call(
            by,
            env,
            "commit_claim",
            commit_args(env, &t, &id).await?,
            GAS_COMMIT,
        )
        .await?,
    );
    ok("claim", claim(by, env, &id, claim_gas).await?);
    Ok((t, id))
}

/// A Verifier admin call: `pa_pause_feature` / `pa_unpause_feature` (no deposit) or
/// `force_lock_account` / `force_unlock_account` (1 yoctoNEAR).
pub async fn vadmin_call(env: &Env, method: &str, args: Value) -> R<()> {
    let deposit = if method.starts_with("pa_") { 0 } else { 1 };
    let o = env
        .vadmin
        .call(env.intents.id(), method)
        .args_json(args)
        .deposit(NearToken::from_yoctonear(deposit))
        .gas(Gas::from_tgas(30))
        .transact()
        .await?;
    ok(&format!("intents.{method}"), o);
    Ok(())
}

pub async fn near_balance(worker: &Worker<Sandbox>, who: &str) -> R<u128> {
    Ok(worker
        .view_account(&who.parse()?)
        .await?
        .balance
        .as_yoctonear())
}
