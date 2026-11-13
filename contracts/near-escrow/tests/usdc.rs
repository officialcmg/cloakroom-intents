//! Sandbox tests of the escrow's Intents withdraw with the real USDC code from NEAR
//! mainnet (`17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`, code hash
//! `DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q`). They check the fix for
//! docs/security/nv02-usdc.md: a USDC pause or blocklist that lands between the Verifier's
//! `ft_on_transfer` and USDC's `ft_resolve_transfer` makes `ft_transfer_call` report
//! failure after a delivery, and the escrow must still pay the taker exactly once.
//!
//! Run from `contracts/near-escrow/`:
//!
//! ```sh
//! source ../../.tools/env.sh
//! ./fetch-usdc-wasm.sh          # once: writes $ZECRET_WASM_DIR/usdc.near.wasm
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test usdc -- --nocapture --test-threads 1
//! ```
//!
//! Without `usdc.near.wasm` every test prints `SKIPPED` and returns. `ESCROW_WASM` names a
//! prebuilt escrow wasm (used to run these tests against the escrow before the fix).
//!
//! Two kinds of Verifier:
//!
//! - `Verifier::Live`: the live `intents.near` code from `$ZECRET_WASM_DIR`. A restriction
//!   is sent as its own transaction `d` blocks after the withdraw, as on mainnet.
//! - `Verifier::Restricting`: `test-receiver/`, a stand-in that applies the restriction
//!   inside its own `ft_on_transfer`, so it lands exactly between the deposit and USDC's
//!   resolve step every time. It holds the blocklister role and a pauser role on USDC.

use ed25519_dalek::{Signer, SigningKey};
use near_workspaces::network::Sandbox;
use near_workspaces::operations::Function;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract, Worker};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;

const USDC_CODE_HASH: &str = "DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q";
/// 1 USDC (6 decimals).
const UNIT: u128 = 1_000_000;
/// The claimed swap.
const AMT: u128 = 1000 * UNIT;
/// Another open swap: "other swaps' funds", which must stay intact.
const OTHER: u128 = 1000 * UNIT;
/// Gas attached to `withdraw` in these tests (see `withdraw_gas_needed`).
const WITHDRAW_GAS: u64 = 200;
const SWEEP_GAS: u64 = 150;

const A_SEC: &str = "195661e2e4789b7f57137b2e777736d79720270638d12e4882fd95e52a73d6b3";
const B_SEC: &str = "2b8bbfabc504814096b330ffba3d828335e8db236f79aa7fded4d77640369e2a";
const A_PUB: &str = "4c303c9656ca22dedd75957bc56c364c6f5461cb0fadca1e589495ebb182a197";
const B_PUB: &str = "950e78fd9d3129d85922bdef19d501fbec46b8c0c0f563d8df66f5e5b86d3a18";

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn le_hex(be: &str) -> String {
    let mut b = hex::decode(be).unwrap();
    b.reverse();
    hex::encode(b)
}

fn wasm_dir() -> String {
    std::env::var("ZECRET_WASM_DIR")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../.tools/wasm").into())
}

/// The USDC code, or None if it was not downloaded.
fn usdc_wasm() -> Option<Vec<u8>> {
    std::fs::read(format!("{}/usdc.near.wasm", wasm_dir())).ok()
}

macro_rules! usdc_or_skip {
    () => {
        match usdc_wasm() {
            Some(w) => w,
            None => {
                eprintln!(
                    "SKIPPED: {}/usdc.near.wasm is missing. Run ./fetch-usdc-wasm.sh in \
                     contracts/near-escrow to download the USDC code from mainnet.",
                    wasm_dir()
                );
                return Ok(());
            }
        }
    };
}

fn build_wasm(crate_dir: PathBuf, file: &str) -> Vec<u8> {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .arg("--target-dir")
        .arg(&target)
        .current_dir(&crate_dir)
        .env_remove("LD_PRELOAD")
        .status()
        .expect("run cargo build");
    assert!(status.success(), "wasm build failed in {crate_dir:?}");
    std::fs::read(target.join("wasm32-unknown-unknown/release").join(file)).expect("read wasm")
}

fn escrow_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        if let Ok(p) = std::env::var("ESCROW_WASM") {
            return std::fs::read(p).expect("read ESCROW_WASM");
        }
        build_wasm(env!("CARGO_MANIFEST_DIR").into(), "near_escrow.wasm")
    })
}

fn receiver_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        build_wasm(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-receiver"),
            "restricting_receiver.wasm",
        )
    })
}

fn tgas(g: Gas) -> f64 {
    g.as_gas() as f64 / 1e12
}

/// Readable names of the accounts in receipt reports.
static NAMES: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

fn name_account(id: &AccountId, name: &str) {
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
        .unwrap_or_else(|| id.to_string())
}

/// Prints the total gas and every receipt (executor, gas burnt, ok or the error).
fn report(label: &str, o: &ExecutionFinalResult) {
    println!(
        "GAS {label:<52} {:>7.2} Tgas  ok={}",
        tgas(o.total_gas_burnt),
        o.is_success()
    );
    for r in o.receipt_outcomes() {
        let status = if r.is_success() {
            "ok".to_string()
        } else {
            format!("{:?}", r.clone().into_result().err())
        };
        println!(
            "    {:<10} {:>6.2} Tgas  {}",
            name_of(&r.executor_id),
            tgas(r.gas_burnt),
            status
        );
    }
}

fn okr(label: &str, o: ExecutionFinalResult) -> ExecutionFinalResult {
    assert!(
        o.is_success() && o.receipt_failures().is_empty(),
        "{label} failed: {:?} / {:?}",
        o.clone().into_result().err(),
        o.receipt_failures()
    );
    o
}

fn ok(label: &str, o: ExecutionFinalResult) {
    let _ = okr(label, o);
}

fn events(o: &ExecutionFinalResult, name: &str) -> Vec<Value> {
    o.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|e| e["standard"] == "near_escrow" && e["event"] == name)
        .collect()
}

/// The failure messages of the receipts that ran on `executor`.
fn failures_at(o: &ExecutionFinalResult, executor: &AccountId) -> Vec<String> {
    o.receipt_outcomes()
        .iter()
        .filter(|r| &r.executor_id == executor && r.is_failure())
        .map(|r| format!("{:?}", r.clone().into_result().err()))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verifier {
    Live,
    Restricting,
}

struct Env {
    worker: Worker<Sandbox>,
    usdc: Contract,
    verifier: Contract,
    escrow: Contract,
    maker: Account,
    pausers: [Account; 2],
    /// Blocklister and third pauser on USDC. With `Verifier::Restricting` it is the
    /// Verifier stand-in itself.
    restrictor: Account,
    /// Super admin of the live Verifier (unused with the stand-in).
    vadmin: Account,
    /// The other swap (OTHER, Open). Refunded at the end to show its funds are intact.
    other_id: String,
}

async fn usdc_call(
    by: &Account,
    usdc: &Contract,
    method: &str,
    args: Value,
) -> R<ExecutionFinalResult> {
    Ok(by
        .call(usdc.id(), method)
        .args_json(args)
        .gas(Gas::from_tgas(50))
        .transact()
        .await?)
}

/// Creates a multisig request and approves it with both approvers. Returns its id.
async fn multisig_request(usdc: &Contract, by: [&Account; 2], action: Value) -> R<u32> {
    let o = okr(
        "create_multisig_request",
        usdc_call(
            by[0],
            usdc,
            "create_multisig_request",
            json!({ "action": action }),
        )
        .await?,
    );
    let id: u32 = o.json()?;
    for a in by {
        ok(
            "approve_multisig_request",
            usdc_call(
                a,
                usdc,
                "approve_multisig_request",
                json!({ "request_id": id }),
            )
            .await?,
        );
    }
    Ok(id)
}

async fn multisig(usdc: &Contract, by: [&Account; 2], action: Value) -> R<()> {
    let id = multisig_request(usdc, by, action).await?;
    ok(
        "execute_multisig_request",
        usdc_call(
            by[1],
            usdc,
            "execute_multisig_request",
            json!({ "request_id": id }),
        )
        .await?,
    );
    Ok(())
}

async fn usdc_register(payer: &Account, usdc: &Contract, who: &AccountId) -> R<()> {
    ok(
        "usdc.storage_deposit",
        payer
            .call(usdc.id(), "storage_deposit")
            .args_json(json!({ "account_id": who }))
            .deposit(NearToken::from_millinear(10))
            .transact()
            .await?,
    );
    Ok(())
}

async fn ft_balance(usdc: &Contract, who: &AccountId) -> R<u128> {
    let v: String = usdc
        .view("ft_balance_of")
        .args_json(json!({ "account_id": who }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

async fn vbalance(env: &Env, who: &str) -> R<u128> {
    let v: String = env
        .verifier
        .view("mt_balance_of")
        .args_json(json!({ "account_id": who, "token_id": format!("nep141:{}", env.usdc.id()) }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

async fn credit(env: &Env, owner: &str) -> R<Value> {
    Ok(env
        .escrow
        .view("get_credit")
        .args_json(json!({ "token": env.usdc.id(), "owner": owner }))
        .await?
        .json()?)
}

async fn intents_credit(env: &Env, owner: &str) -> R<u128> {
    Ok(credit(env, owner).await?["intents"]
        .as_str()
        .unwrap()
        .parse()?)
}

async fn in_flight(env: &Env) -> R<u128> {
    let v: String = env
        .escrow
        .view("get_in_flight")
        .args_json(json!({ "token": env.usdc.id() }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

async fn height(worker: &Worker<Sandbox>) -> R<u64> {
    Ok(worker.view_block().await?.height())
}

async fn now_s(worker: &Worker<Sandbox>) -> R<u64> {
    Ok(worker.view_block().await?.timestamp() / 1_000_000_000)
}

async fn wait_blocks(worker: &Worker<Sandbox>, from: u64, d: u64) -> R<()> {
    while height(worker).await? < from + d {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Ok(())
}

async fn deploy_escrow(worker: &Worker<Sandbox>, intents: &AccountId) -> R<Contract> {
    let account = worker.dev_create_account().await?;
    let key = account.secret_key().public_key();
    ok(
        "escrow deploy + new + delete key",
        account
            .batch(account.id())
            .deploy(escrow_wasm())
            .call(Function::new("new").args_json(json!({ "intents_account": intents })))
            .delete_key(key)
            .transact()
            .await?,
    );
    Ok(Contract::from_secret_key(
        account.id().clone(),
        account.secret_key().clone(),
        worker,
    ))
}

async fn setup(kind: Verifier, usdc_code: &[u8]) -> R<Env> {
    let worker = near_workspaces::sandbox().await?;
    let mut acc = Vec::new();
    for _ in 0..12 {
        acc.push(worker.dev_create_account().await?);
    }
    let [admin1, admin2, mm1, mm2, owner1, pauser1, pauser2, ctrl1, ctrl2, minter, vadmin, restrictor]: [Account; 12] =
        acc.try_into().map_err(|_| "accounts")?;

    // USDC from mainnet, initialised as in circlefin/stablecoin-near: 2-of-n multisig
    // roles, one blocklister. The restrictor is the blocklister and a third pauser.
    let usdc = worker.dev_deploy(usdc_code).await?;
    let code_hash = match worker.view_account(usdc.id()).await?.contract_state {
        near_workspaces::ContractState::LocalHash(h) => h.to_string(),
        other => format!("{other:?}"),
    };
    assert_eq!(
        code_hash, USDC_CODE_HASH,
        "the USDC wasm is not the mainnet code"
    );
    ok(
        "usdc.init",
        usdc.call("init")
            .args_json(json!({
                "admin_ids": [admin1.id(), admin2.id()],
                "master_minter_ids": [mm1.id(), mm2.id()],
                "owner_ids": [owner1.id()],
                "pauser_ids": [pauser1.id(), pauser2.id(), restrictor.id()],
                "blocklister_id": restrictor.id(),
                "metadata": { "spec": "ft-1.0.0", "name": "USDC", "symbol": "USDC", "decimals": 6 },
            }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );
    for ctrl in [&ctrl1, &ctrl2] {
        multisig(
            &usdc,
            [&mm1, &mm2],
            json!({ "ConfigureController": { "controller_id": ctrl.id(), "minter_id": minter.id() } }),
        )
        .await?;
    }
    multisig(
        &usdc,
        [&ctrl1, &ctrl2],
        json!({ "ConfigureMinterAllowance": {
            "controller_id": ctrl1.id(), "minter_allowance": (1_000_000 * UNIT).to_string() } }),
    )
    .await?;

    let verifier = match kind {
        Verifier::Restricting => {
            let c = restrictor.deploy(receiver_wasm()).await?.into_result()?;
            ok("receiver.new", c.call("new").transact().await?);
            c
        }
        Verifier::Live => {
            let c = worker
                .dev_deploy(&std::fs::read(format!("{}/intents.near.wasm", wasm_dir()))?)
                .await?;
            ok(
                "intents.new",
                c.call("new")
                    .args_json(json!({ "config": {
                        "wnear_id": "wrap.test.near",
                        "fees": { "fee": 0, "fee_collector": c.id() },
                        "roles": { "super_admins": [vadmin.id()], "grantees": { "DAO": [vadmin.id()] } },
                    }}))
                    .gas(Gas::from_tgas(300))
                    .transact()
                    .await?,
            );
            c
        }
    };
    let escrow = deploy_escrow(&worker, verifier.id()).await?;
    let maker = worker.dev_create_account().await?;
    for who in [escrow.id(), verifier.id(), maker.id()] {
        usdc_register(&minter, &usdc, who).await?;
    }
    ok(
        "usdc.mint (maker)",
        usdc_call(
            &minter,
            &usdc,
            "mint",
            json!({ "to": maker.id(), "amount": (100_000 * UNIT).to_string() }),
        )
        .await?,
    );
    ok(
        "escrow.add_storage_funds",
        maker
            .call(escrow.id(), "add_storage_funds")
            .args_json(json!({ "token": usdc.id() }))
            .deposit(NearToken::from_near(1))
            .transact()
            .await?,
    );
    for (id, n) in [
        (escrow.id(), "escrow"),
        (usdc.id(), "usdc"),
        (verifier.id(), "verifier"),
        (maker.id(), "maker"),
    ] {
        name_account(id, n);
    }
    let mut env = Env {
        worker,
        usdc,
        verifier,
        escrow,
        maker,
        pausers: [pauser1, pauser2],
        restrictor,
        vadmin,
        other_id: String::new(),
    };
    // The other swap stays Open. Its funds must never pay anyone else.
    env.other_id = open(&env, 200, &taker(200), OTHER).await?;
    Ok(env)
}

struct Taker {
    key: SigningKey,
    id: String,
}

fn taker(seed: u8) -> Taker {
    let key = SigningKey::from_bytes(&[seed; 32]);
    let id = hex::encode(key.verifying_key().to_bytes());
    Taker { key, id }
}

async fn open(env: &Env, salt: u8, t: &Taker, amount: u128) -> R<String> {
    let now = now_s(&env.worker).await?;
    let terms = json!({
        "salt": hex::encode([salt; 32]),
        "taker_pk": t.id,
        "a": A_PUB,
        "b": B_PUB,
        "t0": now + 6000,
        "t1": now + 12000,
        "t2": now + 18000,
        "reveal_window": 6000,
    });
    ok(
        "open (usdc.ft_transfer_call)",
        env.maker
            .call(env.usdc.id(), "ft_transfer_call")
            .args_json(json!({
                "receiver_id": env.escrow.id(),
                "amount": amount.to_string(),
                "msg": terms.to_string(),
            }))
            .deposit(NearToken::from_yoctonear(1))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );
    Ok(env
        .escrow
        .view("swap_id")
        .args_json(json!({ "maker": env.maker.id(), "salt": hex::encode([salt; 32]) }))
        .await?
        .json()?)
}

/// Opens a swap for a new taker and claims it: the taker gets an Intents credit.
async fn claimed(env: &Env, seed: u8, amount: u128) -> R<Taker> {
    let t = taker(seed);
    let id = open(env, seed, &t, amount).await?;
    ok(
        "set_ready",
        env.maker
            .call(env.escrow.id(), "set_ready")
            .args_json(json!({ "id": id }))
            .transact()
            .await?,
    );
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
    let sig = hex::encode(t.key.sign(&hex::decode(&msg)?).to_bytes());
    ok(
        "commit_claim",
        env.maker
            .call(env.escrow.id(), "commit_claim")
            .args_json(json!({ "id": id, "commitment": commitment, "signature": sig }))
            .transact()
            .await?,
    );
    ok(
        "claim",
        env.maker
            .call(env.escrow.id(), "claim")
            .args_json(json!({ "id": id, "a": a }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );
    assert_eq!(intents_credit(env, &t.id).await?, amount);
    Ok(t)
}

/// Sends `withdraw` and waits until all its receipts have run.
async fn withdraw_with_gas(env: &Env, owner: &str, gas: u64) -> R<ExecutionFinalResult> {
    Ok(env
        .maker
        .call(env.escrow.id(), "withdraw")
        .args_json(json!({ "token": env.usdc.id(), "owner": owner }))
        .gas(Gas::from_tgas(gas))
        .transact()
        .await?)
}

async fn withdraw(env: &Env, owner: &str) -> R<ExecutionFinalResult> {
    withdraw_with_gas(env, owner, WITHDRAW_GAS).await
}

async fn sweep(env: &Env) -> R<ExecutionFinalResult> {
    Ok(env
        .maker
        .call(env.escrow.id(), "sweep")
        .args_json(json!({ "token": env.usdc.id() }))
        .gas(Gas::from_tgas(SWEEP_GAS))
        .transact()
        .await?)
}

#[derive(Clone, Copy, Debug)]
enum Restriction {
    Pause,
    BlocklistVerifier,
    BlocklistEscrow,
}

impl Restriction {
    fn panic_text(self) -> &'static str {
        match self {
            Restriction::Pause => "FiatToken: paused",
            _ => "is blocklisted",
        }
    }
}

/// Prepares a restriction: for a pause, a Pause request approved by two pausers.
/// Returns the call that applies it.
async fn prepare(env: &Env, r: Restriction) -> R<(&'static str, Value)> {
    Ok(match r {
        Restriction::Pause => {
            let id = multisig_request(
                &env.usdc,
                [&env.pausers[0], &env.pausers[1]],
                json!("Pause"),
            )
            .await?;
            ("execute_multisig_request", json!({ "request_id": id }))
        }
        Restriction::BlocklistVerifier => ("blocklist", json!({ "account_id": env.verifier.id() })),
        Restriction::BlocklistEscrow => ("blocklist", json!({ "account_id": env.escrow.id() })),
    })
}

/// Lifts the restriction (unpause by multisig, or unblocklist).
async fn lift(env: &Env, r: Restriction) -> R<()> {
    match r {
        Restriction::Pause => {
            multisig(
                &env.usdc,
                [&env.pausers[0], &env.pausers[1]],
                json!("Unpause"),
            )
            .await?
        }
        Restriction::BlocklistVerifier => ok(
            "unblocklist",
            usdc_call(
                &env.restrictor,
                &env.usdc,
                "unblocklist",
                json!({ "account_id": env.verifier.id() }),
            )
            .await?,
        ),
        Restriction::BlocklistEscrow => ok(
            "unblocklist",
            usdc_call(
                &env.restrictor,
                &env.usdc,
                "unblocklist",
                json!({ "account_id": env.escrow.id() }),
            )
            .await?,
        ),
    }
    let paused = env.usdc.view("ft_total_supply").await.is_err();
    assert!(!paused, "USDC still paused");
    Ok(())
}

/// After the restriction is lifted: a second withdraw for the same taker must pay
/// nothing more, the taker holds exactly `amount` in the Verifier, nothing is left in
/// flight, and the escrow's USDC plus its own Verifier balance still covers the other swap.
async fn assert_paid_once(env: &Env, t: &Taker, amount: u128, label: &str) -> R<()> {
    let o = withdraw(env, &t.id).await?;
    report(&format!("{label}: second withdraw"), &o);
    let held = vbalance(env, &t.id).await?;
    let escrow_usdc = ft_balance(&env.usdc, env.escrow.id()).await?;
    let escrow_v = vbalance(env, env.escrow.id().as_str()).await?;
    println!(
        "{label}: taker holds {held} in the Verifier; escrow USDC {escrow_usdc}, escrow Verifier balance {escrow_v}"
    );
    assert_eq!(
        held, amount,
        "{label}: the taker must be paid exactly once (holds {held}, credit was {amount})"
    );
    assert_eq!(intents_credit(env, &t.id).await?, 0, "{label}: credit left");
    assert!(
        escrow_usdc + escrow_v >= OTHER,
        "{label}: the other swap's funds are gone (escrow USDC {escrow_usdc} + Verifier {escrow_v} < {OTHER})"
    );
    // The escrow before the fix (ESCROW_WASM) has no get_in_flight view.
    match env
        .escrow
        .view("get_in_flight")
        .args_json(json!({ "token": env.usdc.id() }))
        .await
    {
        Ok(v) => assert_eq!(v.json::<String>()?, "0", "{label}: withdraw left in flight"),
        Err(e) => assert!(
            std::env::var("ESCROW_WASM").is_ok(),
            "get_in_flight failed: {e}"
        ),
    }
    Ok(())
}

/// The other swap is refunded and the maker withdraws it: its funds are intact. Then the
/// escrow holds nothing.
async fn assert_other_swap_intact(env: &Env) -> R<()> {
    if vbalance(env, env.escrow.id().as_str()).await? > 0 {
        let o = okr("sweep before the final refund", sweep(env).await?);
        report("sweep", &o);
    }
    ok(
        "refund the other swap",
        env.maker
            .call(env.escrow.id(), "refund")
            .args_json(json!({ "id": env.other_id, "b": le_hex(B_SEC) }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );
    let before = ft_balance(&env.usdc, env.maker.id()).await?;
    let o = okr(
        "maker withdraw",
        withdraw(env, env.maker.id().as_str()).await?,
    );
    assert_eq!(
        events(&o, "withdrawn")[0]["data"][0]["amount"],
        OTHER.to_string()
    );
    assert_eq!(ft_balance(&env.usdc, env.maker.id()).await? - before, OTHER);
    assert_eq!(ft_balance(&env.usdc, env.escrow.id()).await?, 0);
    assert_eq!(vbalance(env, env.escrow.id().as_str()).await?, 0);
    Ok(())
}

// ===================================================================== normal path

/// Normal withdraw into the live Intents code with USDC: the deposit goes to the escrow's
/// own Verifier account, mt_transfer pays the taker. Prints the gas of every receipt.
#[tokio::test]
async fn normal_withdraw_into_live_intents() -> R<()> {
    let code = usdc_or_skip!();
    let env = setup(Verifier::Live, &code).await?;
    let t = claimed(&env, 1, AMT).await?;
    let o = okr("withdraw", withdraw(&env, &t.id).await?);
    report("withdraw (USDC, live intents, normal)", &o);
    let w = events(&o, "withdrawn");
    assert_eq!(w.len(), 1);
    assert_eq!(w[0]["data"][0]["amount"], AMT.to_string());
    assert_eq!(w[0]["data"][0]["restored"], "0");
    assert_eq!(vbalance(&env, &t.id).await?, AMT);
    assert_eq!(vbalance(&env, env.escrow.id().as_str()).await?, 0);
    assert_eq!(ft_balance(&env.usdc, env.escrow.id()).await?, OTHER);
    assert_paid_once(&env, &t, AMT, "normal").await?;

    // The callbacks are private.
    for (method, args) in [
        (
            "on_deposit",
            json!({ "token": env.usdc.id(), "owner": t.id, "amount": "1" }),
        ),
        (
            "on_withdraw",
            json!({ "token": env.usdc.id(), "owner": t.id, "route": "intents", "amount": "1" }),
        ),
        ("on_sweep_balance", json!({ "token": env.usdc.id() })),
        (
            "on_swept",
            json!({ "token": env.usdc.id(), "balance": "1", "in_flight": "0" }),
        ),
    ] {
        let o = env
            .maker
            .call(env.escrow.id(), method)
            .args_json(args)
            .gas(Gas::from_tgas(100))
            .transact()
            .await?;
        assert!(
            format!("{:?}", o.into_result().err()).contains("is private"),
            "{method} must be private"
        );
    }

    // sweep with nothing stranded moves nothing.
    let o = okr("sweep (nothing to sweep)", sweep(&env).await?);
    report("sweep (nothing to sweep)", &o);
    let s = events(&o, "swept");
    assert_eq!(s[0]["data"][0]["amount"], "0");
    assert_other_swap_intact(&env).await
}

/// The gas `withdraw` needs on the Intents route. With too little gas the whole call
/// fails and the credit is unchanged.
#[tokio::test]
async fn withdraw_gas_needed() -> R<()> {
    let code = usdc_or_skip!();
    let env = setup(Verifier::Live, &code).await?;
    let t = claimed(&env, 1, AMT).await?;
    let mut needed = None;
    for g in [100u64, 110, 120, 130, 140, 150] {
        let o = withdraw_with_gas(&env, &t.id, g).await?;
        let credit_left = intents_credit(&env, &t.id).await?;
        println!(
            "withdraw with {g} Tgas: ok={} receipt failures={} credit left={credit_left}",
            o.is_success(),
            o.receipt_failures().len()
        );
        if o.is_success() {
            assert!(
                o.receipt_failures().is_empty(),
                "{g} Tgas: a receipt failed"
            );
            assert_eq!(credit_left, 0);
            report(&format!("withdraw with {g} Tgas"), &o);
            needed = Some(g);
            break;
        }
        assert_eq!(credit_left, AMT, "a failed withdraw must leave the credit");
        assert_eq!(in_flight(&env).await?, 0);
    }
    println!("smallest prepaid gas tried that works: {needed:?} Tgas");
    assert!(needed.is_some());
    assert_paid_once(&env, &t, AMT, "gas").await
}

// ===================================================================== exact timing

/// The restriction lands exactly between the Verifier's ft_on_transfer and USDC's resolve
/// step (test receiver). USDC's resolve panics, ft_transfer_call reports failure after the
/// delivery, and the taker must still be paid exactly once.
#[tokio::test]
async fn restriction_between_deposit_and_resolve_pays_once() -> R<()> {
    let code = usdc_or_skip!();
    for r in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistEscrow,
    ] {
        let env = setup(Verifier::Restricting, &code).await?;
        let t = claimed(&env, 1, AMT).await?;
        let (method, args) = prepare(&env, r).await?;
        let arm = match method {
            "blocklist" => json!({ "Blocklist": args }),
            _ => json!({ "ExecuteMultisig": args }),
        };
        ok(
            "receiver.arm",
            env.maker
                .call(env.verifier.id(), "arm")
                .args_json(json!({ "restriction": arm }))
                .transact()
                .await?,
        );
        let o = withdraw(&env, &t.id).await?;
        report(&format!("withdraw, {r:?} between deposit and resolve"), &o);
        let usdc_failures = failures_at(&o, env.usdc.id());
        println!("{r:?}: USDC receipt failures: {usdc_failures:?}");
        assert!(
            usdc_failures.iter().any(|f| f.contains(r.panic_text())),
            "{r:?}: USDC's resolve step must have panicked"
        );
        lift(&env, r).await?;
        assert_paid_once(&env, &t, AMT, &format!("{r:?} exact")).await?;
        let w = events(&o, "withdrawn");
        assert_eq!(w[0]["data"][0]["amount"], AMT.to_string(), "{r:?}");
        assert_eq!(w[0]["data"][0]["restored"], "0", "{r:?}");
        assert_other_swap_intact(&env).await?;
    }
    Ok(())
}

/// The restriction is in place before the withdraw: USDC refuses the deposit, the
/// Verifier balance does not cover mt_transfer, the credit is restored, and after the
/// restriction is lifted it is paid once.
#[tokio::test]
async fn restriction_before_withdraw_restores_then_pays_once() -> R<()> {
    let code = usdc_or_skip!();
    let env = setup(Verifier::Live, &code).await?;
    for (i, r) in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistEscrow,
    ]
    .into_iter()
    .enumerate()
    {
        let t = claimed(&env, 10 + i as u8, AMT).await?;
        let (method, args) = prepare(&env, r).await?;
        let signer = if method == "blocklist" {
            &env.restrictor
        } else {
            &env.pausers[1]
        };
        ok(
            "restrict",
            usdc_call(signer, &env.usdc, method, args).await?,
        );
        let o = withdraw(&env, &t.id).await?;
        report(&format!("withdraw while {r:?}"), &o);
        let w = events(&o, "withdrawn");
        assert_eq!(w[0]["data"][0]["amount"], "0", "{r:?}");
        assert_eq!(w[0]["data"][0]["restored"], AMT.to_string(), "{r:?}");
        assert_eq!(intents_credit(&env, &t.id).await?, AMT);
        assert_eq!(vbalance(&env, &t.id).await?, 0);
        lift(&env, r).await?;
        let o = okr("withdraw after lift", withdraw(&env, &t.id).await?);
        assert_eq!(
            events(&o, "withdrawn")[0]["data"][0]["amount"],
            AMT.to_string()
        );
        assert_paid_once(&env, &t, AMT, &format!("{r:?} before")).await?;
    }
    assert_other_swap_intact(&env).await
}

// ===================================================================== live race

/// The race of nv02-usdc.md section 4.3 with the live Intents code: the restriction is
/// sent as its own transaction d blocks after the withdraw. For every d the taker is paid
/// exactly once and the other swap stays covered.
#[tokio::test]
async fn race_with_live_intents_at_each_delay() -> R<()> {
    let code = usdc_or_skip!();
    for r in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistEscrow,
    ] {
        let env = setup(Verifier::Live, &code).await?;
        let mut hits = Vec::new();
        for d in 0..=5u64 {
            let t = claimed(&env, 20 + d as u8, AMT).await?;
            let (method, args) = prepare(&env, r).await?;
            let signer = if method == "blocklist" {
                &env.restrictor
            } else {
                &env.pausers[1]
            };
            let start = height(&env.worker).await?;
            // The withdraw is sent first; the restriction is sent once d blocks have
            // passed. Both run concurrently and each is awaited to its final outcome.
            let (o, restrict) = tokio::join!(withdraw(&env, &t.id), async {
                wait_blocks(&env.worker, start, d).await?;
                R::Ok(
                    signer
                        .call(env.usdc.id(), method)
                        .args_json(args)
                        .gas(Gas::from_tgas(50))
                        .transact()
                        .await?,
                )
            });
            let o = o?;
            ok("restriction", restrict?);
            let usdc_failures = failures_at(&o, env.usdc.id());
            let after_delivery = usdc_failures.iter().any(|f| f.contains(r.panic_text()))
                && o.logs()
                    .iter()
                    .any(|l| l.contains("mt_mint") || l.contains("\"deposit\""));
            let ev = events(&o, "withdrawn");
            println!(
                "{r:?} d={d}: resolve failed after delivery: {after_delivery}; withdrawn {}; USDC failures {usdc_failures:?}",
                ev.first().map(|e| e["data"][0].to_string()).unwrap_or_default()
            );
            if after_delivery {
                hits.push(d);
            }
            lift(&env, r).await?;
            assert_paid_once(&env, &t, AMT, &format!("{r:?} d={d}")).await?;
        }
        println!("{r:?}: delays where USDC reported failure after a delivery: {hits:?}");
        assert!(
            !hits.is_empty(),
            "{r:?}: the restriction never landed between delivery and resolve; the test did not exercise the race"
        );
        assert_other_swap_intact(&env).await?;
    }
    Ok(())
}

// ===================================================================== Verifier refusals

/// The Verifier refuses mt_transfer (its admin pauses the feature): the deposit stays in
/// the escrow's own Verifier account and the credit is restored. sweep brings the tokens
/// back to the escrow; then one withdraw pays once.
#[tokio::test]
async fn verifier_refuses_mt_transfer_then_sweep() -> R<()> {
    let code = usdc_or_skip!();
    let env = setup(Verifier::Live, &code).await?;
    let t = claimed(&env, 1, AMT).await?;
    ok(
        "intents.pa_pause_feature(mt_transfer)",
        env.vadmin
            .call(env.verifier.id(), "pa_pause_feature")
            .args_json(json!({ "key": "mt_transfer" }))
            .gas(Gas::from_tgas(30))
            .transact()
            .await?,
    );
    let o = withdraw(&env, &t.id).await?;
    report("withdraw while mt_transfer is paused", &o);
    let w = events(&o, "withdrawn");
    assert_eq!(w[0]["data"][0]["amount"], "0");
    assert_eq!(w[0]["data"][0]["restored"], AMT.to_string());
    assert_eq!(intents_credit(&env, &t.id).await?, AMT);
    assert_eq!(vbalance(&env, &t.id).await?, 0);
    assert_eq!(
        vbalance(&env, env.escrow.id().as_str()).await?,
        AMT,
        "deposit stays with the escrow"
    );
    assert_eq!(ft_balance(&env.usdc, env.escrow.id()).await?, OTHER);
    assert_eq!(in_flight(&env).await?, 0);

    // Anyone can sweep. It only returns the escrow's tokens to the escrow.
    // The smallest prepaid gas that works is measured here: a sweep with too little gas
    // fails as a whole and moves nothing.
    let stranger = env.worker.dev_create_account().await?;
    let mut swept = None;
    for g in [70u64, 80, 90, 100, 110, 120, SWEEP_GAS] {
        let o = stranger
            .call(env.escrow.id(), "sweep")
            .args_json(json!({ "token": env.usdc.id() }))
            .gas(Gas::from_tgas(g))
            .transact()
            .await?;
        println!("sweep with {g} Tgas: ok={}", o.is_success());
        if o.is_success() {
            println!("smallest prepaid gas tried that works for sweep: {g} Tgas");
            swept = Some(o);
            break;
        }
        assert_eq!(vbalance(&env, env.escrow.id().as_str()).await?, AMT);
    }
    let o = okr("sweep", swept.ok_or("no sweep succeeded")?);
    report("sweep (stranded deposit)", &o);
    assert_eq!(events(&o, "swept")[0]["data"][0]["amount"], AMT.to_string());
    assert_eq!(vbalance(&env, env.escrow.id().as_str()).await?, 0);
    assert_eq!(ft_balance(&env.usdc, env.escrow.id()).await?, OTHER + AMT);
    assert_eq!(ft_balance(&env.usdc, stranger.id()).await.unwrap_or(0), 0);
    assert_eq!(
        intents_credit(&env, &t.id).await?,
        AMT,
        "sweep changes no credit"
    );

    ok(
        "intents.pa_unpause_feature(mt_transfer)",
        env.vadmin
            .call(env.verifier.id(), "pa_unpause_feature")
            .args_json(json!({ "key": "mt_transfer" }))
            .gas(Gas::from_tgas(30))
            .transact()
            .await?,
    );
    let o = okr("withdraw after unpause", withdraw(&env, &t.id).await?);
    assert_eq!(
        events(&o, "withdrawn")[0]["data"][0]["amount"],
        AMT.to_string()
    );
    assert_paid_once(&env, &t, AMT, "mt_transfer paused").await?;
    assert_other_swap_intact(&env).await
}

/// The Verifier's admin locks the escrow's Verifier account: mt_transfer fails, the credit
/// is restored, sweep moves nothing while the lock holds, and after the unlock sweep and
/// one withdraw pay once.
#[tokio::test]
async fn verifier_locks_the_escrow_account() -> R<()> {
    let code = usdc_or_skip!();
    let env = setup(Verifier::Live, &code).await?;
    let t = claimed(&env, 1, AMT).await?;
    let lock = |m: &'static str| {
        env.vadmin
            .call(env.verifier.id(), m)
            .args_json(json!({ "account_id": env.escrow.id() }))
            .deposit(NearToken::from_yoctonear(1))
            .gas(Gas::from_tgas(30))
            .transact()
    };
    ok(
        "intents.force_lock_account(escrow)",
        lock("force_lock_account").await?,
    );
    let o = withdraw(&env, &t.id).await?;
    report("withdraw while the escrow's Verifier account is locked", &o);
    let w = events(&o, "withdrawn");
    assert_eq!(w[0]["data"][0]["amount"], "0");
    assert_eq!(intents_credit(&env, &t.id).await?, AMT);
    let stranded = vbalance(&env, env.escrow.id().as_str()).await?;
    println!("escrow Verifier balance while locked: {stranded}");
    let o = sweep(&env).await?;
    report("sweep while locked", &o);
    assert_eq!(
        vbalance(&env, env.escrow.id().as_str()).await?,
        stranded,
        "nothing moves while locked"
    );
    assert_eq!(
        ft_balance(&env.usdc, env.escrow.id()).await? + stranded,
        OTHER + AMT,
        "custody unchanged"
    );
    ok(
        "intents.force_unlock_account(escrow)",
        lock("force_unlock_account").await?,
    );
    if stranded > 0 {
        let o = okr("sweep after unlock", sweep(&env).await?);
        assert_eq!(
            events(&o, "swept")[0]["data"][0]["amount"],
            stranded.to_string()
        );
    }
    let o = okr("withdraw after unlock", withdraw(&env, &t.id).await?);
    assert_eq!(
        events(&o, "withdrawn")[0]["data"][0]["amount"],
        AMT.to_string()
    );
    assert_paid_once(&env, &t, AMT, "locked").await?;
    assert_other_swap_intact(&env).await
}

// ===================================================================== shared balance

/// Two takers' withdraws race on the escrow's one Verifier balance, and taker B's deposit
/// fails (the stand-in rejects deposits of B's amount). B is sent k blocks before A for
/// k in 0..=4, so B's mt_transfer can run after A's deposit and before A's mt_transfer and
/// be paid from A's tokens. In every case: each taker is paid at most its credit, paid
/// plus remaining credit equals the original credit, and the escrow's custody (USDC plus
/// its Verifier balance) falls by exactly what was paid. After sweep and retries each
/// taker is paid exactly once.
#[tokio::test]
async fn concurrent_withdraws_share_the_verifier_balance_safely() -> R<()> {
    let code = usdc_or_skip!();
    let env = setup(Verifier::Restricting, &code).await?;
    let (amt_a, amt_b) = (300 * UNIT, 200 * UNIT);
    ok(
        "receiver.reject_deposits_of(B)",
        env.maker
            .call(env.verifier.id(), "reject_deposits_of")
            .args_json(json!({ "amount": amt_b.to_string() }))
            .transact()
            .await?,
    );
    let mut cross = Vec::new();
    for k in 0..=4u64 {
        let a = claimed(&env, 40 + 2 * k as u8, amt_a).await?;
        let b = claimed(&env, 41 + 2 * k as u8, amt_b).await?;
        let custody_before = ft_balance(&env.usdc, env.escrow.id()).await?
            + vbalance(&env, env.escrow.id().as_str()).await?;
        let start = height(&env.worker).await?;
        let (ob, oa) = tokio::join!(withdraw(&env, &b.id), async {
            wait_blocks(&env.worker, start, k).await?;
            withdraw(&env, &a.id).await
        });
        let (ob, oa) = (ob?, oa?);
        let paid_a = vbalance(&env, &a.id).await?;
        let paid_b = vbalance(&env, &b.id).await?;
        let custody_after = ft_balance(&env.usdc, env.escrow.id()).await?
            + vbalance(&env, env.escrow.id().as_str()).await?;
        let ev = |o: &ExecutionFinalResult| events(o, "withdrawn")[0]["data"][0].to_string();
        println!(
            "k={k}: A {} | B {} | paid A {paid_a} B {paid_b} | custody {custody_before} -> {custody_after}",
            ev(&oa),
            ev(&ob)
        );
        if paid_b > 0 {
            cross.push(k);
        }
        assert!(paid_a <= amt_a && paid_b <= amt_b);
        assert_eq!(
            paid_a + intents_credit(&env, &a.id).await?,
            amt_a,
            "k={k}: A"
        );
        assert_eq!(
            paid_b + intents_credit(&env, &b.id).await?,
            amt_b,
            "k={k}: B"
        );
        assert_eq!(
            custody_before - custody_after,
            paid_a + paid_b,
            "k={k}: custody"
        );
        assert_eq!(in_flight(&env).await?, 0);

        // Clean up for the next round: accept deposits again, sweep, retry, then reject B.
        ok(
            "accept all",
            env.maker
                .call(env.verifier.id(), "reject_deposits_of")
                .args_json(json!({ "amount": null }))
                .transact()
                .await?,
        );
        let o = okr("sweep", sweep(&env).await?);
        report(&format!("k={k}: sweep"), &o);
        for (t, amt) in [(&a, amt_a), (&b, amt_b)] {
            assert_paid_once(&env, t, amt, &format!("k={k}")).await?;
        }
        assert_eq!(vbalance(&env, env.escrow.id().as_str()).await?, 0);
        assert_eq!(ft_balance(&env.usdc, env.escrow.id()).await?, OTHER);
        ok(
            "reject B again",
            env.maker
                .call(env.verifier.id(), "reject_deposits_of")
                .args_json(json!({ "amount": amt_b.to_string() }))
                .transact()
                .await?,
        );
    }
    println!("offsets where B was paid from A's deposit in the first attempt: {cross:?}");
    assert!(
        !cross.is_empty(),
        "no offset made B's mt_transfer spend A's deposit; the test did not exercise the shared balance"
    );
    ok(
        "accept all",
        env.maker
            .call(env.verifier.id(), "reject_deposits_of")
            .args_json(json!({ "amount": null }))
            .transact()
            .await?,
    );
    assert_other_swap_intact(&env).await
}
