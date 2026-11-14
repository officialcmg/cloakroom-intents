//! Sandbox integration tests: the escrow with the live `wrap.near` (NEP-141) and
//! `intents.near` (the Verifier) contract code.
//!
//! Run from `contracts/near-escrow/`:
//!
//! ```sh
//! source ../../.tools/env.sh
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox -- --nocapture --test-threads 1
//! ```
//!
//! The escrow wasm is built by the test with `cargo build --target wasm32-unknown-unknown
//! --release`, unless `ESCROW_WASM` names a prebuilt file.

mod common;

use base64::Engine;
use common::*;
use ed25519_dalek::Signer;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::AccountId;
use serde_json::json;

/// 1 wNEAR (24 decimals).
const ONE: u128 = 1_000_000_000_000_000_000_000_000;
const AMOUNT: u128 = 3 * ONE;

async fn setup() -> R<Env> {
    let worker = near_workspaces::sandbox().await?;
    let token = worker.dev_deploy(&live_wasm("wrap.near.wasm")).await?;
    ok("token.new", token.call("new").transact().await?);
    name_account(token.id(), "wnear");
    let vadmin = worker.dev_create_account().await?;
    let intents = deploy_verifier(&worker, token.id(), &vadmin).await?;
    let escrow = deploy_escrow(&worker, intents.id()).await?;
    let maker = worker.dev_create_account().await?;
    name_account(maker.id(), "maker");
    ok(
        "token.near_deposit",
        maker
            .call(token.id(), "near_deposit")
            .deposit(NearToken::from_near(40))
            .transact()
            .await?,
    );
    ok(
        "token.storage_deposit (Verifier)",
        maker
            .call(token.id(), "storage_deposit")
            .args_json(json!({ "account_id": intents.id() }))
            .deposit(NearToken::from_millinear(10))
            .transact()
            .await?,
    );
    ok(
        "escrow.add_storage_funds (1 NEAR)",
        maker
            .call(escrow.id(), "add_storage_funds")
            .args_json(json!({ "token": token.id() }))
            .deposit(NearToken::from_near(1))
            .transact()
            .await?,
    );
    let env = Env {
        worker,
        token,
        intents,
        escrow,
        maker,
        vadmin,
    };
    // The maker keeps its tokens in its own Verifier account and opens swaps from there.
    ok(
        "maker deposit 30 wNEAR into its Verifier account",
        maker_deposit(&env, 30 * ONE).await?,
    );
    assert_eq!(vbal(&env, env.maker.id().as_str()).await?, 30 * ONE);
    Ok(env)
}

// ================================================================== happy path

/// The design's swap with the stipend as the taker's only NEAR: the taker's new key sends
/// commit_claim, claim and withdraw itself and ends with the tokens under its key in the
/// Verifier. Then it signs one ft_withdraw intent (the exit) and submits it itself.
#[tokio::test]
async fn happy_path_taker_completes_alone_then_signed_exit() -> R<()> {
    let env = setup().await?;
    let t = taker(7);
    let id = open(&env, 1, &t, AMOUNT).await?;
    let s = get_swap(&env, &id).await?;
    assert_eq!(s["state"], "Open");
    assert_eq!(s["payout"], "none");
    assert_eq!(s["recipient"], serde_json::Value::Null);
    assert_eq!(vbal(&env, env.escrow.id().as_str()).await?, AMOUNT);
    assert_eq!(owed(&env).await?, AMOUNT);
    assert_eq!(
        ft_balance(&env.token, env.escrow.id()).await?,
        0,
        "the escrow holds no plain tokens"
    );
    assert_backed(&env, "after open").await?;

    ok(
        "maker -> taker stipend",
        env.maker
            .transfer_near(&t.id.parse()?, NearToken::from_millinear(STIPEND_MILLINEAR))
            .await?,
    );
    let taker_acc = taker_account(&t, &env.worker)?;
    set_ready(&env, &id).await?;
    let o = okr(
        "commit_claim by the taker",
        call(
            &taker_acc,
            &env,
            "commit_claim",
            commit_args(&env, &t, &id).await?,
            GAS_COMMIT,
        )
        .await?,
    );
    assert_eq!(events(&o, "locked").len(), 1);

    // claim with the client's gas: a is published, the payout stays Due (not enough gas
    // left for it in 25 Tgas).
    let o = okr(
        "claim by the taker (25 Tgas)",
        claim(&taker_acc, &env, &id, GAS_CLAIM).await?,
    );
    assert_eq!(events(&o, "claimed")[0]["data"][0]["a"], le_hex(A_SEC));
    let s = get_swap(&env, &id).await?;
    assert_eq!(s["state"], "Claimed");
    assert_eq!(
        s["a"],
        le_hex(A_SEC),
        "the maker reads a from contract state"
    );
    assert_eq!(s["payout"], "due");
    assert_eq!(s["recipient"], t.id);
    assert_backed(&env, "after claim").await?;

    // on_payout is private.
    let o = env
        .maker
        .call(env.escrow.id(), "on_payout")
        .args_json(json!({ "id": id }))
        .transact()
        .await?;
    assert!(o.is_failure(), "on_payout must reject outside callers");
    assert!(format!("{:?}", o.into_result().err()).contains("is private"));

    let o = okr(
        "withdraw by the taker (25 Tgas)",
        withdraw(&taker_acc, &env, &id, GAS_WITHDRAW).await?,
    );
    let p = events(&o, "paid");
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["data"][0]["recipient"], t.id);
    assert_eq!(p[0]["data"][0]["amount"], AMOUNT.to_string());
    assert_eq!(get_swap(&env, &id).await?["payout"], "paid");
    assert_eq!(vbal(&env, &t.id).await?, AMOUNT);
    assert_eq!(vbal(&env, env.escrow.id().as_str()).await?, 0);
    assert_eq!(owed(&env).await?, 0);
    assert_backed(&env, "after payout").await?;

    // A second withdraw is refused and pays nothing.
    let o = withdraw(&env.maker, &env, &id, GAS_WITHDRAW).await?;
    report("withdraw again (refused)", &o);
    assert!(format!("{:?}", o.into_result().err()).contains("payout is not Due"));
    assert_eq!(vbal(&env, &t.id).await?, AMOUNT);

    // The taker key signs one ft_withdraw intent; the taker submits it with the stipend.
    let destination = env.worker.dev_create_account().await?;
    ok(
        "token.storage_deposit (destination)",
        env.maker
            .call(env.token.id(), "storage_deposit")
            .args_json(json!({ "account_id": destination.id() }))
            .deposit(NearToken::from_millinear(10))
            .transact()
            .await?,
    );
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
        "intents.execute_intents (ft_withdraw) by the taker",
        taker_acc
            .call(env.intents.id(), "execute_intents")
            .args_json(json!({ "signed": [signed] }))
            .gas(Gas::from_tgas(50))
            .transact()
            .await?,
    );
    assert_eq!(ft_balance(&env.token, destination.id()).await?, AMOUNT);
    assert_eq!(vbal(&env, &t.id).await?, 0);
    Ok(())
}

/// With enough gas, claim pays in the same call. Measures the smallest prepaid gas for a
/// claim that pays, for a claim that only publishes, and for withdraw, and checks that the
/// clients' values (claim 25 Tgas, withdraw 25 Tgas) are on the right side of them.
#[tokio::test]
async fn gas_needed_for_claim_and_withdraw() -> R<()> {
    let env = setup().await?;
    let prober = env.worker.dev_create_account().await?;
    // claim that publishes a: the smallest prepaid gas that works.
    let mut seed = 20u8;
    let mut claim_min = None;
    for g in [17u64, 18, 19, 20] {
        seed += 1;
        let t = taker(seed);
        let id = open(&env, seed, &t, ONE).await?;
        set_ready(&env, &id).await?;
        ok(
            "commit",
            call(
                &prober,
                &env,
                "commit_claim",
                commit_args(&env, &t, &id).await?,
                GAS_COMMIT,
            )
            .await?,
        );
        let o = claim(&prober, &env, &id, g).await?;
        let st = get_swap(&env, &id).await?;
        println!(
            "claim with {g} Tgas: ok={} state={} payout={}",
            o.is_success(),
            st["state"],
            st["payout"]
        );
        if o.is_success() {
            report(&format!("claim {g} Tgas (publishes a, payout Due)"), &o);
            assert_eq!(st["payout"], "due");
            claim_min.get_or_insert(g);
        }
        // withdraw: smallest prepaid gas that pays.
        if st["payout"] == "due" && g == 20 {
            let mut wmin = None;
            for wg in [14u64, 16, 18, 19, 20, 21, 22, 25] {
                let o = withdraw(&prober, &env, &id, wg).await?;
                let st = get_swap(&env, &id).await?;
                println!(
                    "withdraw with {wg} Tgas: ok={} payout={}",
                    o.is_success(),
                    st["payout"]
                );
                if st["payout"] == "paid" {
                    report(&format!("withdraw {wg} Tgas (pays)"), &o);
                    wmin = Some(wg);
                    break;
                }
                assert_eq!(
                    st["payout"], "due",
                    "a withdraw without the gas changes nothing"
                );
            }
            let wmin = wmin.ok_or("no withdraw paid")?;
            println!("SMALLEST withdraw prepaid gas tried that pays: {wmin} Tgas");
            assert!(
                wmin <= GAS_WITHDRAW,
                "clients attach {GAS_WITHDRAW} Tgas to withdraw"
            );
        }
    }
    let claim_min = claim_min.ok_or("no claim worked")?;
    println!("SMALLEST claim prepaid gas tried that works: {claim_min} Tgas");
    // claim that also pays.
    let mut paying_min = None;
    for g in [26u64, 30, 32, 34, 35, 36, 38, 40] {
        seed += 1;
        let t = taker(seed);
        let id = open(&env, seed, &t, ONE).await?;
        set_ready(&env, &id).await?;
        ok(
            "commit",
            call(
                &prober,
                &env,
                "commit_claim",
                commit_args(&env, &t, &id).await?,
                GAS_COMMIT,
            )
            .await?,
        );
        let o = okr(
            &format!("claim {g} Tgas"),
            claim(&prober, &env, &id, g).await?,
        );
        let st = get_swap(&env, &id).await?;
        println!("claim with {g} Tgas: payout={}", st["payout"]);
        if st["payout"] == "paid" {
            assert_eq!(events(&o, "paid").len(), 1);
            assert_eq!(vbal(&env, &t.id).await?, ONE);
            paying_min = Some(g);
            break;
        }
        assert_eq!(st["payout"], "due");
        assert!(g > GAS_CLAIM || g == 26);
    }
    let paying_min = paying_min.ok_or("no claim paid")?;
    println!("SMALLEST claim prepaid gas tried that also pays: {paying_min} Tgas");
    assert!(
        paying_min > GAS_CLAIM,
        "the client's 25 Tgas claim leaves the payout Due"
    );
    assert_backed(&env, "end").await?;
    Ok(())
}

// ================================================================== refunds

/// Refunds pay the maker inside the Verifier, and the maker can take plain tokens out.
#[tokio::test]
async fn refund_paths_pay_the_maker_in_the_verifier() -> R<()> {
    let env = setup().await?;
    let maker_v0 = vbal(&env, env.maker.id().as_str()).await?;

    // (1) Open, refunded by the maker before t0 (early refund). Paid in the same call.
    let t = taker(1);
    let id = open(&env, 1, &t, AMOUNT).await?;
    let o = okr(
        "refund before t0 (50 Tgas)",
        refund(&env.maker, &env, &id, GAS_REFUND).await?,
    );
    assert_eq!(events(&o, "refunded")[0]["data"][0]["b"], le_hex(B_SEC));
    assert_eq!(
        events(&o, "paid")[0]["data"][0]["recipient"],
        env.maker.id().to_string()
    );
    let s = get_swap(&env, &id).await?;
    assert_eq!(
        (s["state"].clone(), s["payout"].clone()),
        (json!("Refunded"), json!("paid"))
    );
    assert_eq!(vbal(&env, env.maker.id().as_str()).await?, maker_v0);

    // (2) Ready, nobody commits; refund in [t1, t2) with little gas: payout Due, then
    // anyone's withdraw pays the maker.
    let now = now_s(&env.worker).await?;
    let t2_ = taker(2);
    let msg = terms(2, &t2_, now + 30, now + 60, now + 6000, 600).to_string();
    let (id, o) = open_msg(&env, 2, msg, AMOUNT).await?;
    ok("open", o);
    set_ready(&env, &id).await?;
    advance_to(&env.worker, now + 61).await?;
    ok(
        "refund in [t1, t2) (20 Tgas)",
        refund(&env.maker, &env, &id, 20).await?,
    );
    assert_eq!(get_swap(&env, &id).await?["payout"], "due");
    assert_backed(&env, "refund due").await?;
    let stranger = env.worker.dev_create_account().await?;
    ok(
        "withdraw by a stranger",
        withdraw(&stranger, &env, &id, GAS_WITHDRAW).await?,
    );
    assert_eq!(get_swap(&env, &id).await?["payout"], "paid");
    assert_eq!(vbal(&env, env.maker.id().as_str()).await?, maker_v0);
    assert_eq!(vbal(&env, stranger.id().as_str()).await?, 0);

    // (3) Locked, the taker does not reveal: refund from locked_at + W.
    let now = now_s(&env.worker).await?;
    let t3 = taker(3);
    let msg = terms(3, &t3, now + 600, now + 1200, now + 1800, 40).to_string();
    let (id, o) = open_msg(&env, 3, msg, AMOUNT).await?;
    ok("open", o);
    set_ready(&env, &id).await?;
    ok(
        "commit",
        call(
            &env.maker,
            &env,
            "commit_claim",
            commit_args(&env, &t3, &id).await?,
            GAS_COMMIT,
        )
        .await?,
    );
    let o = refund(&env.maker, &env, &id, GAS_REFUND).await?;
    assert!(o.is_failure(), "no refund while the reveal window is open");
    let locked_at = get_swap(&env, &id).await?["locked_at"].as_u64().unwrap();
    advance_to(&env.worker, locked_at + 41).await?;
    ok(
        "refund after locked_at + W",
        refund(&env.maker, &env, &id, GAS_REFUND).await?,
    );
    assert_eq!(get_swap(&env, &id).await?["payout"], "paid");
    assert_eq!(vbal(&env, env.maker.id().as_str()).await?, maker_v0);
    assert_eq!(owed(&env).await?, 0);
    assert_eq!(vbal(&env, env.escrow.id().as_str()).await?, 0);

    // The maker takes plain wNEAR out of the Verifier when it wants (ft_withdraw).
    let before = ft_balance(&env.token, env.maker.id()).await?;
    ok(
        "maker ft_withdraw from the Verifier",
        env.maker
            .call(env.intents.id(), "ft_withdraw")
            .args_json(json!({ "token": env.token.id(), "receiver_id": env.maker.id(), "amount": AMOUNT.to_string() }))
            .deposit(NearToken::from_yoctonear(1))
            .gas(Gas::from_tgas(50))
            .transact()
            .await?,
    );
    assert_eq!(
        ft_balance(&env.token, env.maker.id()).await? - before,
        AMOUNT
    );
    Ok(())
}

// ================================================================== bad opens

/// A bad open leaves no swap, and the Verifier returns the tokens to the maker in the same
/// transaction. Nobody but the Verifier can call mt_on_transfer.
#[tokio::test]
async fn bad_open_is_refunded_by_the_verifier() -> R<()> {
    let env = setup().await?;
    let t = taker(10);
    let maker_v0 = vbal(&env, env.maker.id().as_str()).await?;
    let now = now_s(&env.worker).await?;
    let cases = [
        (
            "t1 <= t0",
            terms(1, &t, now + 600, now + 600, now + 1800, 600).to_string(),
        ),
        (
            "t0 in the past",
            terms(1, &t, now - 10, now + 600, now + 1800, 600).to_string(),
        ),
        (
            "W = 0",
            terms(1, &t, now + 600, now + 1200, now + 1800, 0).to_string(),
        ),
        ("not JSON", "hello".to_string()),
        ("unknown field", {
            let mut v = terms(1, &t, now + 600, now + 1200, now + 1800, 600);
            v["relayer_fee"] = json!("1");
            v.to_string()
        }),
    ];
    for (label, msg) in cases {
        let (id, o) = open_msg(&env, 1, msg, AMOUNT).await?;
        report(&format!("open with {label} (refunded by the Verifier)"), &o);
        assert!(
            o.is_success(),
            "the Verifier's call succeeds; the escrow's receipt fails"
        );
        assert_eq!(failures_at(&o, env.escrow.id()).len(), 1);
        assert_eq!(
            o.json::<Vec<String>>()?,
            vec!["0".to_string()],
            "{label}: nothing used"
        );
        assert!(get_swap(&env, &id).await?.is_null());
        assert_eq!(
            vbal(&env, env.maker.id().as_str()).await?,
            maker_v0,
            "{label}"
        );
        assert_eq!(vbal(&env, env.escrow.id().as_str()).await?, 0, "{label}");
        assert_eq!(owed(&env).await?, 0);
    }

    // A maker with no storage funds cannot open; its tokens come back.
    let poor = env.worker.dev_create_account().await?;
    ok(
        "poor: near_deposit",
        poor.call(env.token.id(), "near_deposit")
            .deposit(NearToken::from_near(2))
            .transact()
            .await?,
    );
    let poor_env = Env {
        worker: env.worker.clone(),
        token: env.token.clone(),
        intents: env.intents.clone(),
        escrow: env.escrow.clone(),
        maker: poor.clone(),
        vadmin: env.vadmin.clone(),
    };
    ok(
        "poor: deposit into the Verifier",
        maker_deposit(&poor_env, ONE).await?,
    );
    let (id, o) = open_msg(
        &poor_env,
        1,
        terms(1, &t, now + 600, now + 1200, now + 1800, 600).to_string(),
        ONE,
    )
    .await?;
    report("open without storage funds", &o);
    assert!(failures_at(&o, env.escrow.id())[0].contains("InsufficientStorage"));
    assert!(get_swap(&env, &id).await?.is_null());
    assert_eq!(vbal(&env, poor.id().as_str()).await?, ONE);

    // A contract that is not the Verifier calls mt_on_transfer and names the maker.
    let fake = env.worker.dev_create_account().await?;
    let o = fake
        .call(env.escrow.id(), "mt_on_transfer")
        .args_json(json!({
            "sender_id": env.maker.id(),
            "previous_owner_ids": [env.maker.id()],
            "token_ids": [token_id(&env)],
            "amounts": [AMOUNT.to_string()],
            "msg": terms(2, &t, now + 600, now + 1200, now + 1800, 600).to_string(),
        }))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    report("mt_on_transfer from another account (rejected)", &o);
    assert!(format!("{:?}", o.into_result().err()).contains("NotVerifier"));

    // A good open, then the same id again: refunded.
    let id = open(&env, 2, &t, AMOUNT).await?;
    let (_, o) = open_msg(
        &env,
        2,
        terms(2, &t, now + 600, now + 1200, now + 1800, 600).to_string(),
        AMOUNT,
    )
    .await?;
    report("open with a used swap id (refunded)", &o);
    assert!(failures_at(&o, env.escrow.id())[0].contains("swap id already used"));
    assert_eq!(
        vbal(&env, env.maker.id().as_str()).await?,
        maker_v0 - AMOUNT
    );
    assert_eq!(vbal(&env, env.escrow.id().as_str()).await?, AMOUNT);
    assert_eq!(owed(&env).await?, AMOUNT);
    assert_eq!(get_swap(&env, &id).await?["amount"], AMOUNT.to_string());

    // Two tokens in one batch: refused and refunded.
    let o = env
        .maker
        .call(env.intents.id(), "mt_batch_transfer_call")
        .args_json(json!({
            "receiver_id": env.escrow.id(),
            "token_ids": [token_id(&env), token_id(&env)],
            "amounts": ["1", "1"],
            "msg": terms(3, &t, now + 600, now + 1200, now + 1800, 600).to_string(),
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(GAS_OPEN))
        .transact()
        .await?;
    report("open with two tokens (refunded)", &o);
    assert!(failures_at(&o, env.escrow.id())[0].contains("exactly one token"));
    assert_eq!(
        vbal(&env, env.maker.id().as_str()).await?,
        maker_v0 - AMOUNT
    );
    assert_backed(&env, "end").await?;
    Ok(())
}

// ================================================================== stipend

/// A fresh implicit account holding exactly the stipend completes commit_claim, claim and
/// the payout alone, and ends with the tokens under its key in the Verifier. The test
/// prints the NEAR balance before and after each call, and the cost that NEAR checks
/// before it accepts each transaction (read from a rejection with a too-small balance).
#[tokio::test]
async fn stipend_alone_completes_commit_claim_and_payout() -> R<()> {
    let env = setup().await?;
    println!(
        "sandbox gas price: {} yoctoNEAR per gas",
        env.worker.gas_price().await?.as_yoctonear()
    );
    for (seed, stipend_milli) in [(40u8, STIPEND_MILLINEAR), (41, 30)] {
        let t = taker(seed);
        let id = open(&env, seed, &t, AMOUNT).await?;
        set_ready(&env, &id).await?;
        let o = okr(
            &format!("stipend {stipend_milli} mNEAR (creates the implicit account)"),
            env.maker
                .transfer_near(&t.id.parse()?, NearToken::from_millinear(stipend_milli))
                .await?,
        );
        let burnt: u128 = o
            .outcomes()
            .iter()
            .map(|x| x.tokens_burnt.as_yoctonear())
            .sum();
        println!("STIPEND {stipend_milli} mNEAR: the maker's transfer burnt {burnt} yoctoNEAR on top of the stipend");
        let acc = taker_account(&t, &env.worker)?;
        let settle = || tokio::time::sleep(std::time::Duration::from_secs(3));
        let b0 = near_balance(&env.worker, &t.id).await?;
        let o = okr(
            "taker commit_claim",
            call(
                &acc,
                &env,
                "commit_claim",
                commit_args(&env, &t, &id).await?,
                GAS_COMMIT,
            )
            .await?,
        );
        let _ = o;
        settle().await;
        let b1 = near_balance(&env.worker, &t.id).await?;
        ok("taker claim", claim(&acc, &env, &id, GAS_CLAIM).await?);
        settle().await;
        let b2 = near_balance(&env.worker, &t.id).await?;
        assert_eq!(get_swap(&env, &id).await?["payout"], "due");
        ok(
            "taker withdraw",
            withdraw(&acc, &env, &id, GAS_WITHDRAW).await?,
        );
        settle().await;
        let b3 = near_balance(&env.worker, &t.id).await?;
        println!(
            "TAKER stipend {stipend_milli} mNEAR: start {b0}; after commit {b1} (-{}); after claim {b2} (-{}); after withdraw {b3} (-{}); total spent {}",
            b0 - b1,
            b1 - b2,
            b2 - b3,
            b0 - b3
        );
        assert_eq!(get_swap(&env, &id).await?["payout"], "paid");
        assert_eq!(
            vbal(&env, &t.id).await?,
            AMOUNT,
            "tokens under the taker's key in the Verifier"
        );
    }

    // The cost NEAR checks before it accepts a transaction: read it from the rejection of
    // a transaction sent by an account that holds 1 milliNEAR.
    let t = taker(42);
    ok(
        "1 mNEAR",
        env.maker
            .transfer_near(&t.id.parse()?, NearToken::from_millinear(1))
            .await?,
    );
    let acc = taker_account(&t, &env.worker)?;
    for (method, g) in [
        ("commit_claim", GAS_COMMIT),
        ("claim", GAS_CLAIM),
        ("withdraw", GAS_WITHDRAW),
    ] {
        let r = acc
            .call(env.escrow.id(), method)
            .args_json(json!({ "id": hex::encode([0u8; 32]) }))
            .gas(Gas::from_tgas(g))
            .transact()
            .await;
        let e = format!("{:?}", r.err());
        let cost = e
            .split("cost: NearToken { inner: ")
            .nth(1)
            .and_then(|s| s.split(' ').next())
            .ok_or("no NotEnoughBalance cost in the error")?
            .to_string();
        println!("UPFRONT COST {method} with {g} Tgas attached: {cost} yoctoNEAR");
        let cost: u128 = cost.parse()?;
        // Attached gas at 1e9 yoctoNEAR per gas (min_gas_purchase_price) is most of it.
        assert!(cost >= u128::from(g) * 1_000_000_000_000 * 1_000_000_000);
        assert!(cost < (u128::from(g) + 3) * 1_000_000_000_000 * 1_000_000_000);
    }
    Ok(())
}

/// The taker's backing check holds through many swaps in any order.
#[tokio::test]
async fn escrow_balance_backs_every_unpaid_swap() -> R<()> {
    let env = setup().await?;
    let prober = env.worker.dev_create_account().await?;
    let mut ids = Vec::new();
    for s in 1..=3u8 {
        ids.push((
            taker(s),
            open(&env, s, &taker(s), u128::from(s) * ONE).await?,
        ));
        assert_backed(&env, "open").await?;
    }
    // Pay swap 2 to the taker; swap 1 refunded with Due payout; swap 3 still open.
    let (t2, id2) = &ids[1];
    set_ready(&env, id2).await?;
    ok(
        "commit",
        call(
            &prober,
            &env,
            "commit_claim",
            commit_args(&env, t2, id2).await?,
            GAS_COMMIT,
        )
        .await?,
    );
    ok("claim with payout", claim(&prober, &env, id2, 60).await?);
    let (bal, o) = assert_backed(&env, "after claim 2").await?;
    assert_eq!((bal, o), (4 * ONE, 4 * ONE));
    ok(
        "refund 1 (Due)",
        refund(&env.maker, &env, &ids[0].1, 20).await?,
    );
    let (bal, o) = assert_backed(&env, "after refund 1 Due").await?;
    assert_eq!((bal, o), (4 * ONE, 4 * ONE));
    ok(
        "withdraw 1",
        withdraw(&prober, &env, &ids[0].1, GAS_WITHDRAW).await?,
    );
    let (bal, o) = assert_backed(&env, "after payout 1").await?;
    assert_eq!((bal, o), (3 * ONE, 3 * ONE));
    // Someone sends tokens to the escrow outside a swap: a surplus nobody can take.
    ok(
        "plain mt_transfer to the escrow",
        env.maker
            .call(env.intents.id(), "mt_transfer")
            .args_json(json!({ "receiver_id": env.escrow.id(), "token_id": token_id(&env), "amount": "5" }))
            .deposit(NearToken::from_yoctonear(1))
            .transact()
            .await?,
    );
    let (bal, o) = assert_backed(&env, "after a gift").await?;
    assert_eq!((bal, o), (3 * ONE + 5, 3 * ONE));
    Ok(())
}

// ================================================================== Verifier refuses

/// The Verifier's operators pause mt_transfer or lock the escrow's Verifier account. The
/// payout fails, nothing moves, the payout is Due again; after they stop, one withdraw
/// pays once.
#[tokio::test]
async fn verifier_refuses_payout_then_one_retry_pays_once() -> R<()> {
    let env = setup().await?;
    let prober = env.worker.dev_create_account().await?;
    for (seed, how) in [(1u8, "pause"), (2, "lock")] {
        let (t, id) = claimed(&env, &prober, seed, AMOUNT, GAS_CLAIM).await?;
        match how {
            "pause" => {
                vadmin_call(&env, "pa_pause_feature", json!({ "key": "mt_transfer" })).await?
            }
            _ => {
                vadmin_call(
                    &env,
                    "force_lock_account",
                    json!({ "account_id": env.escrow.id() }),
                )
                .await?
            }
        }
        let o = withdraw(&prober, &env, &id, GAS_WITHDRAW).await?;
        report(&format!("withdraw while the Verifier refuses ({how})"), &o);
        assert_eq!(events(&o, "payout_failed").len(), 1);
        assert_eq!(get_swap(&env, &id).await?["payout"], "due");
        assert_eq!(vbal(&env, &t.id).await?, 0);
        assert_eq!(vbal(&env, env.escrow.id().as_str()).await?, AMOUNT);
        assert_backed(&env, how).await?;
        match how {
            "pause" => {
                vadmin_call(&env, "pa_unpause_feature", json!({ "key": "mt_transfer" })).await?
            }
            _ => {
                vadmin_call(
                    &env,
                    "force_unlock_account",
                    json!({ "account_id": env.escrow.id() }),
                )
                .await?
            }
        }
        let o = okr(
            "withdraw after",
            withdraw(&prober, &env, &id, GAS_WITHDRAW).await?,
        );
        assert_eq!(events(&o, "paid").len(), 1);
        let o = withdraw(&prober, &env, &id, GAS_WITHDRAW).await?;
        assert!(o.is_failure(), "paid once");
        assert_eq!(vbal(&env, &t.id).await?, AMOUNT);
        assert_eq!(owed(&env).await?, 0);
    }
    Ok(())
}

/// Two withdraws of the same Due payout in the same block: one starts, the other is
/// refused; the taker is paid once.
#[tokio::test]
async fn concurrent_withdraws_pay_once() -> R<()> {
    let env = setup().await?;
    let prober = env.worker.dev_create_account().await?;
    let other = env.worker.dev_create_account().await?;
    let (t, id) = claimed(&env, &prober, 5, AMOUNT, GAS_CLAIM).await?;
    let (a, b) = tokio::join!(
        withdraw(&prober, &env, &id, GAS_WITHDRAW),
        withdraw(&other, &env, &id, GAS_WITHDRAW)
    );
    let (a, b) = (a?, b?);
    report("withdraw A", &a);
    report("withdraw B", &b);
    let paid = events(&a, "paid").len() + events(&b, "paid").len();
    assert_eq!(paid, 1);
    assert_eq!(vbal(&env, &t.id).await?, AMOUNT);
    assert_eq!(owed(&env).await?, 0);
    Ok(())
}

// ============================================================================ deployment

/// Review-1 NV-01. `new` is private, and the documented deployment (one transaction:
/// deploy, `new`, delete the key) leaves an escrow with the reviewed code, no access keys
/// and the configured Intents account, which is what a taker checks.
#[tokio::test]
async fn new_is_private_and_deployment_leaves_no_keys() -> R<()> {
    let worker = near_workspaces::sandbox().await?;
    let intents = worker.dev_create_account().await?;
    let attacker = worker.dev_create_account().await?;

    let escrow = worker.dev_deploy(escrow_wasm()).await?;
    let o = attacker
        .call(escrow.id(), "new")
        .args_json(json!({ "intents_account": attacker.id() }))
        .transact()
        .await?;
    report("escrow.new by another account (rejected)", &o);
    let e = format!("{:?}", o.into_result().err());
    assert!(
        e.contains("is private"),
        "expected a private-method error, got {e}"
    );
    ok(
        "escrow.new by the escrow account",
        escrow
            .call("new")
            .args_json(json!({ "intents_account": intents.id() }))
            .transact()
            .await?,
    );
    let got: AccountId = escrow.view("get_intents_account").await?.json()?;
    assert_eq!(&got, intents.id());

    let escrow = deploy_escrow(&worker, intents.id()).await?;
    let keys = escrow.view_access_keys().await?;
    assert!(
        keys.is_empty(),
        "escrow still has access keys: {}",
        keys.len()
    );
    assert_eq!(escrow.view_code().await?, escrow_wasm());
    let got: AccountId = escrow.view("get_intents_account").await?.json()?;
    assert_eq!(&got, intents.id());
    let o = attacker
        .call(escrow.id(), "new")
        .args_json(json!({ "intents_account": attacker.id() }))
        .transact()
        .await?;
    report("escrow.new again (rejected)", &o);
    assert!(o.is_failure());
    Ok(())
}
