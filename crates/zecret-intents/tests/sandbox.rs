//! Signed exits built by this crate, executed by the Verifier code live on
//! `intents.near`, in a local NEAR sandbox.
//!
//! Needs `NEAR_SANDBOX_BIN_PATH` (nearcore 2.14.1) and `ZECRET_WASM_DIR` with
//! `intents.near.wasm` and `wrap.near.wasm`:
//!
//! ```sh
//! cd /home/user/zecret && source .tools/env.sh
//! cd crates/zecret-intents && LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox
//! ```
#![cfg(not(target_arch = "wasm32"))]

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, Contract};
use serde_json::json;
use zecret_intents::direct::execute_intents_args;
use zecret_intents::time::{self, Time};
use zecret_intents::{
    HttpRequest, HttpResponse, HttpTransport, IntentDraft, IntentSigner, Nonce, Salt,
    SignedPayload, Standard, VerifierRpc,
};

const ONE_WNEAR: u128 = 1_000_000_000_000_000_000_000_000;

/// A blocking transport over `curl`, to exercise the crate's NEAR RPC builders and
/// parsers against the sandbox node.
struct Curl;

impl HttpTransport for Curl {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut c = Command::new("curl");
        c.args(["-sS", "-X", r.method.as_str(), "-w", "\n%{http_code}"]);
        for (k, v) in &r.headers {
            c.args(["-H", &format!("{k}: {v}")]);
        }
        if r.body.is_some() {
            c.args(["--data-binary", "@-"]);
        }
        c.arg(&r.url);
        // The sandbox listens on localhost; do not send it through a proxy.
        c.env("NO_PROXY", "*").env("no_proxy", "*");
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().map_err(|e| e.to_string())?;
        let mut stdin = child.stdin.take().ok_or("stdin")?;
        if let Some(b) = &r.body {
            stdin.write_all(b).map_err(|e| e.to_string())?;
        }
        drop(stdin);
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').ok_or("no status")?;
        Ok(HttpResponse::new(
            code.trim().parse::<u16>().map_err(|e| e.to_string())?,
            body.as_bytes().to_vec(),
        ))
    }
}

fn now() -> Time {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64;
    time::from_unix_millis(ms).expect("time")
}

struct Env {
    worker: near_workspaces::Worker<near_workspaces::network::Sandbox>,
    token: Contract,
    intents: Contract,
    relayer: Account,
    destination: Account,
    rpc: VerifierRpc<Curl>,
}

async fn setup() -> Result<Env, Box<dyn std::error::Error>> {
    let dir = std::env::var("ZECRET_WASM_DIR").unwrap_or_else(|_| "wasm".into());
    let intents_wasm = std::fs::read(format!("{dir}/intents.near.wasm"))?;
    let token_wasm = std::fs::read(format!("{dir}/wrap.near.wasm"))?;
    let worker = near_workspaces::sandbox().await?;

    let token = worker.dev_deploy(&token_wasm).await?;
    token.call("new").transact().await?.into_result()?;
    let intents = worker.dev_deploy(&intents_wasm).await?;
    intents
        .call("new")
        .args_json(json!({ "config": {
            "wnear_id": token.id(),
            "fees": { "fee": 0, "fee_collector": intents.id() }
        }}))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?
        .into_result()?;
    let relayer = worker.dev_create_account().await?;
    let destination = worker.dev_create_account().await?;
    let rpc = VerifierRpc {
        transport: Curl,
        rpc_url: worker.rpc_addr(),
        verifier: intents.id().to_string(),
    };
    Ok(Env {
        worker,
        token,
        intents,
        relayer,
        destination,
        rpc,
    })
}

impl Env {
    /// Deposits `amount` of the token into the Verifier for `owner`, like the escrow's
    /// payout does (`ft_transfer_call` with `msg` = the owner's account id).
    async fn deposit_for(
        &self,
        owner: &str,
        amount: u128,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let payer = self.worker.dev_create_account().await?;
        payer
            .call(self.token.id(), "near_deposit")
            .deposit(NearToken::from_near(5))
            .transact()
            .await?
            .into_result()?;
        for account in [self.intents.id(), self.destination.id()] {
            payer
                .call(self.token.id(), "storage_deposit")
                .args_json(json!({ "account_id": account }))
                .deposit(NearToken::from_millinear(10))
                .transact()
                .await?
                .into_result()?;
        }
        payer
            .call(self.token.id(), "ft_transfer_call")
            .args_json(json!({
                "receiver_id": self.intents.id(),
                "amount": amount.to_string(),
                "msg": owner,
            }))
            .deposit(NearToken::from_yoctonear(1))
            .gas(Gas::from_tgas(100))
            .transact()
            .await?
            .into_result()?;
        Ok(())
    }

    fn token_id(&self) -> String {
        format!("nep141:{}", self.token.id())
    }

    async fn verifier_balance(&self, owner: &str) -> Result<u128, Box<dyn std::error::Error>> {
        let s: String = self
            .intents
            .view("mt_balance_of")
            .args_json(json!({ "account_id": owner, "token_id": self.token_id() }))
            .await?
            .json()?;
        Ok(s.parse()?)
    }

    async fn ft_balance(&self, account: &str) -> Result<u128, Box<dyn std::error::Error>> {
        let s: String = self
            .token
            .view("ft_balance_of")
            .args_json(json!({ "account_id": account }))
            .await?
            .json()?;
        Ok(s.parse()?)
    }

    /// `execute_intents` from the relayer, a third account that pays the gas.
    async fn execute(
        &self,
        signed: &[SignedPayload],
    ) -> Result<near_workspaces::result::ExecutionFinalResult, Box<dyn std::error::Error>> {
        let outcome = self
            .relayer
            .call(self.intents.id(), "execute_intents")
            .args_json(execute_intents_args(signed))
            .gas(Gas::from_tgas(300))
            .transact()
            .await?;
        println!(
            "execute_intents ok={} gas={:.1} Tgas",
            outcome.is_success(),
            outcome.total_gas_burnt.as_gas() as f64 / 1e12
        );
        Ok(outcome)
    }

    /// Waits until the RPC at final finality shows `expected` for `owner` (final lags
    /// the latest block by a few blocks).
    fn wait_final_balance(&self, owner: &str, expected: u128) -> u128 {
        let mut seen = u128::MAX;
        for _ in 0..40 {
            seen = self
                .rpc
                .balance(owner, &self.token_id())
                .expect("rpc balance");
            if seen == expected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        seen
    }
}

fn failure_text(outcome: &near_workspaces::result::ExecutionFinalResult) -> String {
    outcome
        .failures()
        .iter()
        .map(|f| format!("{f:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn ft_withdraw_and_transfer_signed_by_an_unfunded_key()
-> Result<(), Box<dyn std::error::Error>> {
    let env = setup().await?;
    let taker = IntentSigner::from_seed(&[0x42; 32]);
    let owner = taker.account_id();
    env.deposit_for(&owner, 3 * ONE_WNEAR).await?;
    assert_eq!(env.verifier_balance(&owner).await?, 3 * ONE_WNEAR);
    // The key's account does not exist on NEAR and holds no NEAR.
    assert!(env.worker.view_account(&owner.parse()?).await.is_err());

    // The salt for versioned nonces, read with the crate's RPC builder and parser.
    let salt = env.rpc.current_salt()?;
    let view: String = env.intents.view("current_salt").await?.json()?;
    assert_eq!(salt.to_hex(), view);
    println!("sandbox Verifier salt {view}");
    assert_eq!(env.wait_final_balance(&owner, 3 * ONE_WNEAR), 3 * ONE_WNEAR);

    // (a) ft_withdraw, raw_ed25519, versioned nonce: 1 wNEAR to a NEAR account.
    let t = now();
    let deadline = t + chrono::Duration::minutes(10);
    let withdraw = IntentDraft::new(
        &owner,
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(salt, deadline)?,
    )
    .ft_withdraw(
        env.token.id().as_str(),
        env.destination.id().as_str(),
        ONE_WNEAR,
    )
    .sign(&taker, Standard::RawEd25519, t)?;
    // Simulation through the RPC first: it passes and changes nothing.
    let sim = env.rpc.simulate(std::slice::from_ref(&withdraw))?;
    println!(
        "simulate_intents: {}",
        sim.to_string().chars().take(200).collect::<String>()
    );
    assert_eq!(env.verifier_balance(&owner).await?, 3 * ONE_WNEAR);

    let before = env.ft_balance(env.destination.id().as_str()).await?;
    let outcome = env.execute(std::slice::from_ref(&withdraw)).await?;
    assert!(outcome.is_success(), "{}", failure_text(&outcome));
    assert_eq!(
        env.ft_balance(env.destination.id().as_str()).await? - before,
        ONE_WNEAR
    );
    assert_eq!(env.verifier_balance(&owner).await?, 2 * ONE_WNEAR);

    // The same signed payload again: the nonce is used, the Verifier refuses.
    let nonce = withdraw.unsigned().decode()?.nonce;
    let replay = env.execute(std::slice::from_ref(&withdraw)).await?;
    assert!(!replay.is_success());
    let text = failure_text(&replay);
    println!(
        "replay failure: {}",
        text.chars().take(300).collect::<String>()
    );
    assert!(text.contains("nonce was already used"), "{text}");
    assert_eq!(env.verifier_balance(&owner).await?, 2 * ONE_WNEAR);
    let used = (0..40).any(|_| {
        let u = env.rpc.nonce_used(&owner, &nonce).expect("rpc");
        if !u {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        u
    });
    assert!(used);

    // (b) transfer, nep413, versioned nonce: 1 wNEAR to another unfunded key's account
    // inside the Verifier (the shape of the 1Click exit: a transfer to the deposit
    // address).
    let receiver = IntentSigner::from_seed(&[0x43; 32]).account_id();
    let t = now();
    let deadline = t + chrono::Duration::minutes(10);
    let transfer = IntentDraft::new(
        &owner,
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(salt, deadline)?,
    )
    .transfer(&receiver, &env.token_id(), ONE_WNEAR)
    .sign(&taker, Standard::Nep413, t)?;
    let outcome = env.execute(std::slice::from_ref(&transfer)).await?;
    assert!(outcome.is_success(), "{}", failure_text(&outcome));
    assert_eq!(env.verifier_balance(&receiver).await?, ONE_WNEAR);
    assert_eq!(env.verifier_balance(&owner).await?, ONE_WNEAR);
    // The Verifier logs the intent hash we computed.
    let hash = transfer.intent_hash_base58()?;
    let logs = outcome.logs().join("\n");
    assert!(logs.contains(&hash), "hash {hash} not in logs: {logs}");

    // (b2) More than the balance (what 1Click hit with our unfunded key on mainnet):
    // the Verifier refuses and nothing moves.
    let t = now();
    let deadline = t + chrono::Duration::minutes(10);
    let too_much = IntentDraft::new(
        &owner,
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(salt, deadline)?,
    )
    .transfer(&receiver, &env.token_id(), 2 * ONE_WNEAR)
    .sign(&taker, Standard::RawEd25519, t)?;
    let outcome = env.execute(std::slice::from_ref(&too_much)).await?;
    assert!(!outcome.is_success());
    let text = failure_text(&outcome);
    assert!(text.contains("insufficient balance"), "{text}");
    assert_eq!(env.verifier_balance(&owner).await?, ONE_WNEAR);

    // (c) A wrong salt: the Verifier refuses and nothing moves.
    let t = now();
    let deadline = t + chrono::Duration::minutes(10);
    let bad_salt = Salt([0, 0, 0, 1]);
    assert_ne!(bad_salt, salt);
    let bad = IntentDraft::new(
        &owner,
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(bad_salt, deadline)?,
    )
    .transfer(&receiver, &env.token_id(), 1)
    .sign(&taker, Standard::RawEd25519, t)?;
    assert!(env.rpc.simulate(std::slice::from_ref(&bad)).is_err());
    let outcome = env.execute(std::slice::from_ref(&bad)).await?;
    assert!(!outcome.is_success());
    assert!(
        failure_text(&outcome).contains("salt"),
        "{}",
        failure_text(&outcome)
    );
    assert_eq!(env.verifier_balance(&owner).await?, ONE_WNEAR);

    // (d) A payload for the taker's account signed by another key: the crate's draft
    // signing refuses, and the Verifier refuses it too.
    let other = IntentSigner::from_seed(&[0x44; 32]);
    let draft = IntentDraft::new(
        &owner,
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(salt, deadline)?,
    )
    .transfer(&receiver, &env.token_id(), 1);
    assert!(draft.sign(&other, Standard::RawEd25519, t).is_err());
    let forged = other.sign(&draft.to_unsigned(Standard::RawEd25519, t)?)?;
    let outcome = env.execute(std::slice::from_ref(&forged)).await?;
    assert!(!outcome.is_success());
    let text = failure_text(&outcome);
    assert!(
        text.contains("public key") && text.contains("doesn't exist"),
        "{text}"
    );
    assert_eq!(env.verifier_balance(&owner).await?, ONE_WNEAR);

    // (e) A legacy nonce still works: withdraw the rest.
    let t = now();
    let mut legacy = [0u8; 32];
    zecret_intents::random_bytes(&mut legacy)?;
    legacy[0] = 0; // not the versioned magic
    let last = IntentDraft::new(
        &owner,
        env.intents.id().as_str(),
        t + chrono::Duration::minutes(10),
        Nonce::legacy(legacy)?,
    )
    .ft_withdraw(
        env.token.id().as_str(),
        env.destination.id().as_str(),
        ONE_WNEAR,
    )
    .sign(&taker, Standard::Nep413, t)?;
    let outcome = env.execute(std::slice::from_ref(&last)).await?;
    assert!(outcome.is_success(), "{}", failure_text(&outcome));
    assert_eq!(env.verifier_balance(&owner).await?, 0);
    assert_eq!(
        env.ft_balance(env.destination.id().as_str()).await? - before,
        2 * ONE_WNEAR
    );

    // The taker's key never paid gas: its account still does not exist.
    assert!(env.worker.view_account(&owner.parse()?).await.is_err());
    Ok(())
}

#[tokio::test]
async fn several_signed_payloads_in_one_call() -> Result<(), Box<dyn std::error::Error>> {
    let env = setup().await?;
    let a = IntentSigner::from_seed(&[0x51; 32]);
    let b = IntentSigner::from_seed(&[0x52; 32]);
    env.deposit_for(&a.account_id(), ONE_WNEAR).await?;
    let salt = Salt::from_hex(&env.intents.view("current_salt").await?.json::<String>()?)?;
    let t = now();
    let deadline = t + chrono::Duration::minutes(5);
    // a pays b, then b withdraws: both in one execute_intents.
    let p1 = IntentDraft::new(
        a.account_id(),
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(salt, deadline)?,
    )
    .transfer(&b.account_id(), &env.token_id(), ONE_WNEAR)
    .sign(&a, Standard::RawEd25519, t)?;
    let p2 = IntentDraft::new(
        b.account_id(),
        env.intents.id().as_str(),
        deadline,
        Nonce::versioned_random(salt, deadline)?,
    )
    .ft_withdraw(
        env.token.id().as_str(),
        env.destination.id().as_str(),
        ONE_WNEAR,
    )
    .sign(&b, Standard::Nep413, t)?;
    let before = env.ft_balance(env.destination.id().as_str()).await?;
    let outcome = env.execute(&[p1, p2]).await?;
    assert!(outcome.is_success(), "{}", failure_text(&outcome));
    assert_eq!(
        env.ft_balance(env.destination.id().as_str()).await? - before,
        ONE_WNEAR
    );
    assert_eq!(env.verifier_balance(&a.account_id()).await?, 0);
    assert_eq!(env.verifier_balance(&b.account_id()).await?, 0);
    Ok(())
}
