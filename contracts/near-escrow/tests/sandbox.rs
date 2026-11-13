//! Sandbox integration tests: the escrow with the live `wrap.near` (NEP-141) and
//! `intents.near` contract code.
//!
//! Run from `contracts/near-escrow/`:
//!
//! ```sh
//! source ../../.tools/env.sh
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox -- --nocapture
//! ```
//!
//! The escrow wasm is built by the test with `cargo build --target wasm32-unknown-unknown
//! --release`, unless `ESCROW_WASM` names a prebuilt file. The live contract code is read
//! from `$ZECRET_WASM_DIR` (default `../../.tools/wasm`).

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use near_workspaces::network::Sandbox;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken, SecretKey};
use near_workspaces::{Account, AccountId, Contract, Worker};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;

/// 1 wNEAR (24 decimals).
const ONE: u128 = 1_000_000_000_000_000_000_000_000;
const AMOUNT: u128 = 3 * ONE;

// Key shares from contracts/test/CloakroomEscrow.t.sol, converted to the encodings of this
// contract (32-byte little-endian scalars, 32-byte compressed points).
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

fn escrow_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        if let Ok(p) = std::env::var("ESCROW_WASM") {
            return std::fs::read(p).expect("read ESCROW_WASM");
        }
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
            .current_dir(&dir)
            .env_remove("LD_PRELOAD")
            .status()
            .expect("run cargo build");
        assert!(status.success(), "wasm build failed");
        let target = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dir.join("target"));
        std::fs::read(target.join("wasm32-unknown-unknown/release/near_escrow.wasm"))
            .expect("read escrow wasm")
    })
}

fn live_wasm(name: &str) -> Vec<u8> {
    let dir = std::env::var("ZECRET_WASM_DIR")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../.tools/wasm").into());
    std::fs::read(format!("{dir}/{name}")).unwrap_or_else(|e| panic!("read {dir}/{name}: {e}"))
}

fn tgas(o: &ExecutionFinalResult) -> f64 {
    o.total_gas_burnt.as_gas() as f64 / 1e12
}

/// Prints the gas of a call and fails the test if any receipt failed (unless allowed).
fn report(label: &str, o: &ExecutionFinalResult) {
    println!(
        "GAS {label:<44} {:>7.2} Tgas  ok={}",
        tgas(o),
        o.is_success()
    );
    for f in o.receipt_failures() {
        println!(
            "    receipt failure at {}: {:?}",
            f.executor_id,
            f.clone().into_result().err()
        );
    }
}

/// Reports the call, asserts that it and all its receipts succeeded, and returns it.
fn okr(label: &str, o: ExecutionFinalResult) -> ExecutionFinalResult {
    report(label, &o);
    assert!(
        o.is_success(),
        "{label} failed: {:?}",
        o.clone().into_result().err()
    );
    assert!(
        o.receipt_failures().is_empty(),
        "{label} had a failed receipt"
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

struct Env {
    worker: Worker<Sandbox>,
    token: Contract,
    intents: Contract,
    escrow: Contract,
    maker: Account,
}

async fn deploy_escrow(worker: &Worker<Sandbox>, intents: &AccountId) -> R<Contract> {
    let escrow = worker.dev_deploy(escrow_wasm()).await?;
    ok(
        "escrow.new",
        escrow
            .call("new")
            .args_json(json!({ "intents_account": intents }))
            .transact()
            .await?,
    );
    Ok(escrow)
}

async fn storage_deposit(payer: &Account, token: &Contract, who: &AccountId) -> R<()> {
    ok(
        "token.storage_deposit",
        payer
            .call(token.id(), "storage_deposit")
            .args_json(json!({ "account_id": who }))
            .deposit(NearToken::from_millinear(10))
            .transact()
            .await?,
    );
    Ok(())
}

async fn new_maker(
    env_worker: &Worker<Sandbox>,
    token: &Contract,
    escrow: &Contract,
    wnear: u128,
) -> R<Account> {
    let maker = env_worker.dev_create_account().await?;
    ok(
        "token.near_deposit",
        maker
            .call(token.id(), "near_deposit")
            .deposit(NearToken::from_yoctonear(wnear))
            .transact()
            .await?,
    );
    ok(
        "escrow.add_storage_funds (0.1 NEAR)",
        maker
            .call(escrow.id(), "add_storage_funds")
            .args_json(json!({ "token": token.id() }))
            .deposit(NearToken::from_millinear(100))
            .transact()
            .await?,
    );
    Ok(maker)
}

async fn setup() -> R<Env> {
    let worker = near_workspaces::sandbox().await?;
    let token = worker.dev_deploy(&live_wasm("wrap.near.wasm")).await?;
    ok("token.new", token.call("new").transact().await?);
    let intents = worker.dev_deploy(&live_wasm("intents.near.wasm")).await?;
    ok(
        "intents.new",
        intents
            .call("new")
            .args_json(json!({ "config": {
                "wnear_id": token.id(),
                "fees": { "fee": 0, "fee_collector": intents.id() }
            }}))
            .gas(Gas::from_tgas(300))
            .transact()
            .await?,
    );
    let escrow = deploy_escrow(&worker, intents.id()).await?;
    let maker = new_maker(&worker, &token, &escrow, 10 * ONE).await?;
    storage_deposit(&maker, &token, escrow.id()).await?;
    storage_deposit(&maker, &token, intents.id()).await?;
    Ok(Env {
        worker,
        token,
        intents,
        escrow,
        maker,
    })
}

async fn now_s(worker: &Worker<Sandbox>) -> R<u64> {
    Ok(worker.view_block().await?.timestamp() / 1_000_000_000)
}

/// Moves sandbox time forward until the block timestamp is at least `target` seconds.
async fn advance_to(worker: &Worker<Sandbox>, target: u64) -> R<()> {
    while now_s(worker).await? < target {
        worker.fast_forward(50).await?;
    }
    Ok(())
}

async fn ft_balance(token: &Contract, who: &AccountId) -> R<u128> {
    let v: String = token
        .view("ft_balance_of")
        .args_json(json!({ "account_id": who }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

async fn intents_balance(intents: &Contract, token: &Contract, who: &str) -> R<u128> {
    let v: String = intents
        .view("mt_balance_of")
        .args_json(json!({ "account_id": who, "token_id": format!("nep141:{}", token.id()) }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

async fn get_swap(escrow: &Contract, id: &str) -> R<Value> {
    Ok(escrow
        .view("get_swap")
        .args_json(json!({ "id": id }))
        .await?
        .json()?)
}

async fn get_credit(escrow: &Contract, token: &Contract, owner: &str) -> R<Value> {
    Ok(escrow
        .view("get_credit")
        .args_json(json!({ "token": token.id(), "owner": owner }))
        .await?
        .json()?)
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

fn taker_account(t: &Taker, worker: &Worker<Sandbox>) -> R<Account> {
    let mut kp = t.key.to_bytes().to_vec();
    kp.extend_from_slice(&t.key.verifying_key().to_bytes());
    let sk: SecretKey = format!("ed25519:{}", bs58::encode(kp).into_string()).parse()?;
    Ok(Account::from_secret_key(t.id.parse()?, sk, worker))
}

#[allow(clippy::too_many_arguments)]
async fn open(
    env: &Env,
    maker: &Account,
    salt: u8,
    taker: &Taker,
    amount: u128,
    t0: u64,
    t1: u64,
    t2: u64,
    w: u64,
) -> R<(String, ExecutionFinalResult)> {
    let terms = json!({
        "salt": hex::encode([salt; 32]),
        "taker_pk": taker.id,
        "a": A_PUB,
        "b": B_PUB,
        "t0": t0,
        "t1": t1,
        "t2": t2,
        "reveal_window": w,
    });
    let o = maker
        .call(env.token.id(), "ft_transfer_call")
        .args_json(json!({
            "receiver_id": env.escrow.id(),
            "amount": amount.to_string(),
            "msg": terms.to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    let id: String = env
        .escrow
        .view("swap_id")
        .args_json(json!({ "maker": maker.id(), "salt": hex::encode([salt; 32]) }))
        .await?
        .json()?;
    Ok((id, o))
}

async fn withdraw(caller: &Account, env: &Env, owner: &str) -> R<ExecutionFinalResult> {
    Ok(caller
        .call(env.escrow.id(), "withdraw")
        .args_json(json!({ "token": env.token.id(), "owner": owner }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?)
}

// =============================================================================== (a)

#[tokio::test]
async fn happy_path_claim_into_intents_then_signed_exit() -> R<()> {
    let env = setup().await?;
    let t = taker(7);

    // Step 11 of the design: the maker's stipend creates the taker's implicit account.
    // In this test the taker also sends withdraw (100 Tgas) and execute_intents (50 Tgas)
    // herself. A signer must hold the attached gas at a pessimistic price before NEAR
    // accepts the transaction (see stipend_needed_for_taker_commit_and_claim), so this
    // test sends 0.2 NEAR.
    ok(
        "maker -> taker stipend 0.2 NEAR",
        env.maker
            .transfer_near(&t.id.parse()?, NearToken::from_millinear(200))
            .await?,
    );
    let taker_acc = taker_account(&t, &env.worker)?;

    let now = now_s(&env.worker).await?;
    let funds_before: String = env
        .escrow
        .view("get_storage_funds")
        .args_json(json!({ "account_id": env.maker.id(), "token": env.token.id() }))
        .await?
        .json()?;
    let (id, o) = open(
        &env,
        &env.maker,
        1,
        &t,
        AMOUNT,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    let o = okr("open (ft_transfer_call -> ft_on_transfer)", o);
    assert_eq!(events(&o, "opened").len(), 1);
    let used: String = o.json()?;
    assert_eq!(
        used,
        AMOUNT.to_string(),
        "the token reports the whole amount as used"
    );
    let funds_after: String = env
        .escrow
        .view("get_storage_funds")
        .args_json(json!({ "account_id": env.maker.id(), "token": env.token.id() }))
        .await?
        .json()?;
    println!(
        "storage cost of one swap: {} yoctoNEAR",
        funds_before.parse::<u128>()? - funds_after.parse::<u128>()?
    );
    assert_eq!(ft_balance(&env.token, env.escrow.id()).await?, AMOUNT);
    assert_eq!(get_swap(&env.escrow, &id).await?["state"], "Open");

    ok(
        "set_ready",
        env.maker
            .call(env.escrow.id(), "set_ready")
            .args_json(json!({ "id": id }))
            .transact()
            .await?,
    );

    // The taker signs the exact bytes the escrow returns, and sends the commit herself.
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
    let o = okr(
        "commit_claim (ed25519_verify)",
        taker_acc
            .call(env.escrow.id(), "commit_claim")
            .args_json(json!({ "id": id, "commitment": commitment, "signature": hex::encode(sig) }))
            .gas(Gas::from_tgas(10))
            .transact()
            .await?,
    );
    assert_eq!(events(&o, "locked").len(), 1);

    let o = okr(
        "claim (Pallas a·G == A)",
        taker_acc
            .call(env.escrow.id(), "claim")
            .args_json(json!({ "id": id, "a": a }))
            .gas(Gas::from_tgas(30))
            .transact()
            .await?,
    );
    assert_eq!(events(&o, "claimed")[0]["data"][0]["a"], a);
    let swap = get_swap(&env.escrow, &id).await?;
    assert_eq!(swap["state"], "Claimed");
    assert_eq!(swap["a"], a, "the maker reads a from contract state");
    assert_eq!(
        get_credit(&env.escrow, &env.token, &t.id).await?["intents"],
        AMOUNT.to_string()
    );

    // on_withdraw is private.
    let o = env
        .maker
        .call(env.escrow.id(), "on_withdraw")
        .args_json(
            json!({ "token": env.token.id(), "owner": t.id, "route": "intents", "amount": "1" }),
        )
        .transact()
        .await?;
    assert!(o.is_failure(), "on_withdraw must reject outside callers");

    let o = okr(
        "withdraw (taker -> intents, ft_transfer_call)",
        withdraw(&taker_acc, &env, &t.id).await?,
    );
    let w = events(&o, "withdrawn");
    assert_eq!(w.len(), 1);
    assert_eq!(w[0]["data"][0]["amount"], AMOUNT.to_string());
    assert_eq!(w[0]["data"][0]["restored"], "0");
    assert_eq!(
        get_credit(&env.escrow, &env.token, &t.id).await?["intents"],
        "0"
    );
    assert_eq!(ft_balance(&env.token, env.escrow.id()).await?, 0);
    let held = intents_balance(&env.intents, &env.token, &t.id).await?;
    println!("taker {} holds {held} inside Intents", t.id);
    assert_eq!(held, AMOUNT);

    // A second withdraw pays nothing.
    let o = okr(
        "withdraw again (nothing to pay)",
        withdraw(&taker_acc, &env, &t.id).await?,
    );
    assert!(events(&o, "withdrawn").is_empty());

    // The taker key signs one ft_withdraw intent; the taker submits it with the stipend.
    let destination = env.worker.dev_create_account().await?;
    storage_deposit(&env.maker, &env.token, destination.id()).await?;
    let deadline = (chrono::Utc::now() + chrono::Duration::hours(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let nonce = base64::engine::general_purpose::STANDARD.encode([9u8; 32]);
    let payload = json!({
        "signer_id": t.id,
        "verifying_contract": env.intents.id(),
        "deadline": deadline,
        "nonce": nonce,
        "intents": [{
            "intent": "ft_withdraw",
            "token": env.token.id(),
            "receiver_id": destination.id(),
            "amount": AMOUNT.to_string(),
        }],
    })
    .to_string();
    let signature = t.key.sign(payload.as_bytes()).to_bytes();
    let pk = t.key.verifying_key().to_bytes();
    let signed = json!({
        "standard": "raw_ed25519",
        "payload": payload,
        "public_key": format!("ed25519:{}", bs58::encode(pk).into_string()),
        "signature": format!("ed25519:{}", bs58::encode(signature).into_string()),
    });
    ok(
        "intents.execute_intents (ft_withdraw)",
        taker_acc
            .call(env.intents.id(), "execute_intents")
            .args_json(json!({ "signed": [signed] }))
            .gas(Gas::from_tgas(50))
            .transact()
            .await?,
    );
    assert_eq!(ft_balance(&env.token, destination.id()).await?, AMOUNT);
    assert_eq!(intents_balance(&env.intents, &env.token, &t.id).await?, 0);
    Ok(())
}

// =============================================================================== (b)

#[tokio::test]
async fn refund_paths_and_maker_withdraw() -> R<()> {
    let env = setup().await?;
    let t = taker(8);
    let start_balance = ft_balance(&env.token, env.maker.id()).await?;
    let b = le_hex(B_SEC);

    // Window 1: Open, before t0. Anyone can send the refund; here a third party does.
    let now = now_s(&env.worker).await?;
    let (id1, o) = open(
        &env,
        &env.maker,
        1,
        &t,
        AMOUNT,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    ok("open", o);
    let third = env.worker.dev_create_account().await?;
    let wrong = third
        .call(env.escrow.id(), "refund")
        .args_json(json!({ "id": id1, "b": le_hex(A_SEC) }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    report("refund with the wrong share (rejected)", &wrong);
    assert!(format!("{:?}", wrong.into_result().err()).contains("BadSecret"));
    let o = okr(
        "refund (Open, before t0)",
        third
            .call(env.escrow.id(), "refund")
            .args_json(json!({ "id": id1, "b": b }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );
    assert_eq!(events(&o, "refunded")[0]["data"][0]["b"], b);
    let swap = get_swap(&env.escrow, &id1).await?;
    assert_eq!(swap["state"], "Refunded");
    assert_eq!(swap["b"], b, "the taker reads b from contract state");

    // Window 2: Open, in [t1, t2). Short windows and sandbox fast-forward.
    let now = now_s(&env.worker).await?;
    let (t0, t1, t2) = (now + 20, now + 40, now + 1_000_000);
    let (id2, o) = open(&env, &env.maker, 2, &t, AMOUNT, t0, t1, t2, 600).await?;
    ok("open (short windows)", o);
    advance_to(&env.worker, t0).await?;
    let early = env
        .maker
        .call(env.escrow.id(), "refund")
        .args_json(json!({ "id": id2, "b": b }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    report("refund in [t0, t1) (rejected)", &early);
    assert!(format!("{:?}", early.into_result().err()).contains("TooEarly"));
    advance_to(&env.worker, t1).await?;
    println!(
        "sandbox time advanced to t1 + {} s",
        now_s(&env.worker).await? - t1
    );
    ok(
        "refund (Open, in [t1, t2))",
        env.maker
            .call(env.escrow.id(), "refund")
            .args_json(json!({ "id": id2, "b": b }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );

    assert_eq!(
        get_credit(&env.escrow, &env.token, env.maker.id().as_str()).await?["direct"],
        (2 * AMOUNT).to_string()
    );
    assert_eq!(
        ft_balance(&env.token, env.maker.id()).await?,
        start_balance - 2 * AMOUNT
    );
    let o = okr(
        "withdraw (maker, ft_transfer)",
        withdraw(&third, &env, env.maker.id().as_str()).await?,
    );
    assert_eq!(
        events(&o, "withdrawn")[0]["data"][0]["amount"],
        (2 * AMOUNT).to_string()
    );
    assert_eq!(ft_balance(&env.token, env.maker.id()).await?, start_balance);
    assert_eq!(ft_balance(&env.token, env.escrow.id()).await?, 0);
    assert_eq!(
        get_credit(&env.escrow, &env.token, env.maker.id().as_str()).await?["direct"],
        "0"
    );
    Ok(())
}

// =============================================================================== (c)

#[tokio::test]
async fn failed_withdraw_restores_credit() -> R<()> {
    let env = setup().await?;
    let t = taker(9);
    let b = le_hex(B_SEC);

    // Direct route: a maker who is no longer registered on the token.
    let maker2 = new_maker(&env.worker, &env.token, &env.escrow, AMOUNT).await?;
    // wrap.near takes its storage fee out of the first deposit, so open with the balance.
    let amt2 = ft_balance(&env.token, maker2.id()).await?;
    let now = now_s(&env.worker).await?;
    let (id, o) = open(
        &env,
        &maker2,
        1,
        &t,
        amt2,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    ok("open (maker2, whole balance)", o);
    assert_eq!(ft_balance(&env.token, maker2.id()).await?, 0);
    ok(
        "token.storage_unregister (maker2)",
        maker2
            .call(env.token.id(), "storage_unregister")
            .args_json(json!({ "force": false }))
            .deposit(NearToken::from_yoctonear(1))
            .transact()
            .await?,
    );
    ok(
        "refund",
        maker2
            .call(env.escrow.id(), "refund")
            .args_json(json!({ "id": id, "b": b }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?,
    );
    let o = withdraw(&maker2, &env, maker2.id().as_str()).await?;
    report("withdraw to unregistered maker (ft_transfer fails)", &o);
    // The transaction succeeds; the token's receipt fails and the callback restores.
    assert!(o.is_success());
    assert_eq!(o.receipt_failures().len(), 1);
    let w = events(&o, "withdrawn");
    assert_eq!(w[0]["data"][0]["amount"], "0");
    assert_eq!(w[0]["data"][0]["restored"], amt2.to_string());
    assert_eq!(
        get_credit(&env.escrow, &env.token, maker2.id().as_str()).await?["direct"],
        amt2.to_string()
    );
    assert_eq!(ft_balance(&env.token, env.escrow.id()).await?, amt2);
    // After registering again, the same credit pays out.
    storage_deposit(&maker2, &env.token, maker2.id()).await?;
    let o = withdraw(&maker2, &env, maker2.id().as_str()).await?;
    ok("withdraw after re-registering", o);
    assert_eq!(ft_balance(&env.token, maker2.id()).await?, amt2);
    assert_eq!(
        get_credit(&env.escrow, &env.token, maker2.id().as_str()).await?["direct"],
        "0"
    );

    // Intents route, two failures, each with its own escrow:
    // (1) the Intents account is not registered on the token, so ft_transfer_call fails;
    // (2) the Intents account has no contract, so the token's ft_resolve_transfer returns
    //     the whole amount as unused.
    let unregistered = env.worker.dev_create_account().await?;
    let no_code = env.worker.dev_create_account().await?;
    storage_deposit(&env.maker, &env.token, no_code.id()).await?;
    for (label, intents_account) in [
        ("intents account not registered", unregistered.id()),
        ("intents account without code", no_code.id()),
    ] {
        let escrow = deploy_escrow(&env.worker, intents_account).await?;
        storage_deposit(&env.maker, &env.token, escrow.id()).await?;
        ok(
            "escrow.add_storage_funds",
            env.maker
                .call(escrow.id(), "add_storage_funds")
                .args_json(json!({ "token": env.token.id() }))
                .deposit(NearToken::from_millinear(100))
                .transact()
                .await?,
        );
        let env2 = Env {
            worker: env.worker.clone(),
            token: env.token.clone(),
            intents: env.intents.clone(),
            escrow,
            maker: env.maker.clone(),
        };
        let now = now_s(&env.worker).await?;
        let (id, o) = open(
            &env2,
            &env.maker,
            1,
            &t,
            AMOUNT,
            now + 600,
            now + 1200,
            now + 1800,
            600,
        )
        .await?;
        ok("open", o);
        ok(
            "set_ready",
            env.maker
                .call(env2.escrow.id(), "set_ready")
                .args_json(json!({ "id": id }))
                .transact()
                .await?,
        );
        let a = le_hex(A_SEC);
        let commitment: String = env2
            .escrow
            .view("commitment_of")
            .args_json(json!({ "id": id, "a": a }))
            .await?
            .json()?;
        let msg: String = env2
            .escrow
            .view("commit_message")
            .args_json(json!({ "id": id, "commitment": commitment }))
            .await?
            .json()?;
        let sig = hex::encode(t.key.sign(&hex::decode(&msg)?).to_bytes());
        // Anyone may send the commit; here the maker relays the taker's signature.
        ok(
            "commit_claim (sent by a third party)",
            env.maker
                .call(env2.escrow.id(), "commit_claim")
                .args_json(json!({ "id": id, "commitment": commitment, "signature": sig }))
                .transact()
                .await?,
        );
        ok(
            "claim",
            env.maker
                .call(env2.escrow.id(), "claim")
                .args_json(json!({ "id": id, "a": a }))
                .gas(Gas::from_tgas(100))
                .transact()
                .await?,
        );
        let o = withdraw(&env.maker, &env2, &t.id).await?;
        report(&format!("withdraw, {label}"), &o);
        assert!(o.is_success());
        let w = events(&o, "withdrawn");
        assert_eq!(w[0]["data"][0]["amount"], "0", "{label}");
        assert_eq!(w[0]["data"][0]["restored"], AMOUNT.to_string(), "{label}");
        assert_eq!(
            get_credit(&env2.escrow, &env.token, &t.id).await?["intents"],
            AMOUNT.to_string()
        );
        assert_eq!(
            ft_balance(&env.token, env2.escrow.id()).await?,
            AMOUNT,
            "{label}"
        );
        assert_eq!(
            ft_balance(&env.token, intents_account).await.unwrap_or(0),
            0,
            "{label}"
        );
    }
    Ok(())
}

// =============================================================================== open guards

#[tokio::test]
async fn bad_open_is_refunded_by_the_token() -> R<()> {
    let env = setup().await?;
    let t = taker(10);
    let before = ft_balance(&env.token, env.maker.id()).await?;
    let now = now_s(&env.worker).await?;
    // t1 <= t0: ft_on_transfer panics and the token returns the whole amount.
    let (id, o) = open(
        &env,
        &env.maker,
        1,
        &t,
        AMOUNT,
        now + 600,
        now + 600,
        now + 1800,
        600,
    )
    .await?;
    report("open with bad windows (refunded by token)", &o);
    let used: String = o.json()?;
    assert_eq!(used, "0");
    assert_eq!(ft_balance(&env.token, env.maker.id()).await?, before);
    assert!(get_swap(&env.escrow, &id).await?.is_null());

    // A maker with no storage funds cannot open.
    let poor = env.worker.dev_create_account().await?;
    ok(
        "token.near_deposit",
        poor.call(env.token.id(), "near_deposit")
            .deposit(NearToken::from_yoctonear(ONE))
            .transact()
            .await?,
    );
    // wrap.near takes its storage fee out of the first deposit, so read the balance.
    let poor_balance = ft_balance(&env.token, poor.id()).await?;
    assert!(poor_balance > 0);
    let (id, o) = open(
        &env,
        &poor,
        1,
        &t,
        poor_balance,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    report("open without storage funds (refunded by token)", &o);
    assert_eq!(o.json::<String>()?, "0");
    assert_eq!(ft_balance(&env.token, poor.id()).await?, poor_balance);
    assert!(get_swap(&env.escrow, &id).await?.is_null());

    // A fake "token" calls ft_on_transfer directly and names the maker as sender. It
    // cannot spend the maker's storage funds for the real token, so no swap is stored.
    let fake = env.worker.dev_create_account().await?;
    let terms = json!({
        "salt": hex::encode([2u8; 32]),
        "taker_pk": t.id,
        "a": A_PUB,
        "b": B_PUB,
        "t0": now + 600,
        "t1": now + 1200,
        "t2": now + 1800,
        "reveal_window": 600,
    });
    let o = fake
        .call(env.escrow.id(), "ft_on_transfer")
        .args_json(json!({
            "sender_id": env.maker.id(),
            "amount": AMOUNT.to_string(),
            "msg": terms.to_string(),
        }))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    report("ft_on_transfer from a fake token (rejected)", &o);
    assert!(format!("{:?}", o.into_result().err()).contains("InsufficientStorage"));

    // The maker opens with that salt; then the same id again is rejected.
    let (_, o) = open(
        &env,
        &env.maker,
        2,
        &t,
        AMOUNT,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    ok("open", o);
    let (_, o) = open(
        &env,
        &env.maker,
        2,
        &t,
        AMOUNT,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    report("open with a used swap id (refunded by token)", &o);
    assert_eq!(o.json::<String>()?, "0");
    assert_eq!(ft_balance(&env.token, env.escrow.id()).await?, AMOUNT);
    Ok(())
}

// =============================================================================== stipend

/// Step 11 of the design plans a stipend of about 0.01 NEAR for the taker's key. NEAR
/// checks before it accepts a transaction that the signer holds the attached gas at a
/// pessimistic price (measured here: about 10 times the gas price), plus the fees. This
/// test shows that 0.01 NEAR cannot send even a 10 Tgas commit_claim, and that 0.03 NEAR
/// lets the taker send commit_claim and claim herself.
#[tokio::test]
async fn stipend_needed_for_taker_commit_and_claim() -> R<()> {
    let env = setup().await?;
    let price = env.worker.gas_price().await?;
    println!(
        "sandbox gas price: {} yoctoNEAR per gas",
        price.as_yoctonear()
    );
    let t = taker(11);
    ok(
        "maker -> taker stipend 0.01 NEAR",
        env.maker
            .transfer_near(&t.id.parse()?, NearToken::from_millinear(10))
            .await?,
    );
    let taker_acc = taker_account(&t, &env.worker)?;
    let now = now_s(&env.worker).await?;
    let (id, o) = open(
        &env,
        &env.maker,
        1,
        &t,
        AMOUNT,
        now + 600,
        now + 1200,
        now + 1800,
        600,
    )
    .await?;
    ok("open", o);
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
    let commit = || {
        taker_acc
            .call(env.escrow.id(), "commit_claim")
            .args_json(json!({ "id": id, "commitment": commitment, "signature": sig }))
            .gas(Gas::from_tgas(10))
            .transact()
    };
    let rejected = commit().await;
    let reason = format!("{:?}", rejected.as_ref().err());
    println!("commit_claim with a 0.01 NEAR stipend: {reason}");
    assert!(reason.contains("NotEnoughBalance"));

    ok(
        "maker -> taker stipend top-up to 0.03 NEAR",
        env.maker
            .transfer_near(&t.id.parse()?, NearToken::from_millinear(20))
            .await?,
    );
    ok(
        "commit_claim by the taker (10 Tgas attached)",
        commit().await?,
    );
    ok(
        "claim by the taker (25 Tgas attached)",
        taker_acc
            .call(env.escrow.id(), "claim")
            .args_json(json!({ "id": id, "a": a }))
            .gas(Gas::from_tgas(25))
            .transact()
            .await?,
    );
    // Gas refunds arrive in later blocks; wait for them before reading the balance.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let left = env
        .worker
        .view_account(&t.id.parse()?)
        .await?
        .balance
        .as_yoctonear();
    println!("taker NEAR left of the 0.03 NEAR stipend after refunds: {left} yoctoNEAR");
    assert_eq!(get_swap(&env.escrow, &id).await?["state"], "Claimed");
    Ok(())
}
