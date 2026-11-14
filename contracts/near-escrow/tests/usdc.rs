//! Sandbox tests of the escrow with the real USDC code from NEAR mainnet
//! (`17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`, code hash
//! `DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q`) and the live Verifier code.
//!
//! The escrow keeps its tokens in the Verifier and never calls the token contract. USDC's
//! pause and blocklist (docs/security/nv02-usdc.md) can therefore only act where a USDC
//! call happens: when the maker deposits USDC into its own Verifier account before it
//! opens, and when the maker takes plain USDC out of the Verifier after a refund. These
//! tests apply each restriction at each of those points, and also while swaps are open,
//! claimed and refunded.
//!
//! Run from `contracts/near-escrow/`:
//!
//! ```sh
//! source ../../.tools/env.sh
//! ./fetch-usdc-wasm.sh          # once: writes $ZECRET_WASM_DIR/usdc.near.wasm
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test usdc -- --nocapture --test-threads 1
//! ```
//!
//! Without `usdc.near.wasm` every test fails with a message that names
//! `fetch-usdc-wasm.sh`. With `ZECRET_SKIP_USDC=1` each test instead prints `SKIPPED` and
//! passes; such a run did not test anything.

mod common;

use common::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract};
use serde_json::{json, Value};

const USDC_CODE_HASH: &str = "DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q";
/// 1 USDC (6 decimals).
const UNIT: u128 = 1_000_000;
const AMT: u128 = 1000 * UNIT;
/// Another open swap: "other swaps' funds", which must stay intact.
const OTHER: u128 = 1000 * UNIT;
/// What the maker deposits into the Verifier at setup.
const DEPOSIT: u128 = 20_000 * UNIT;
const MINTED: u128 = 100_000 * UNIT;

fn usdc_wasm() -> Option<Vec<u8>> {
    std::fs::read(format!("{}/usdc.near.wasm", wasm_dir())).ok()
}

/// The USDC code. Without it the test fails, so that a green run always means the tests
/// ran. `ZECRET_SKIP_USDC=1` turns the failure into a `SKIPPED` line and a pass.
macro_rules! usdc_or_skip {
    () => {
        match usdc_wasm() {
            Some(w) => w,
            None => {
                let msg = format!(
                    "{}/usdc.near.wasm is missing. Run ./fetch-usdc-wasm.sh in \
                     contracts/near-escrow to download the USDC code from mainnet.",
                    wasm_dir()
                );
                if std::env::var("ZECRET_SKIP_USDC").as_deref() == Ok("1") {
                    eprintln!("SKIPPED (ZECRET_SKIP_USDC=1): {msg}");
                    return Ok(());
                }
                panic!("{msg} To skip these tests on purpose, set ZECRET_SKIP_USDC=1.");
            }
        }
    };
}

struct U {
    env: Env,
    pausers: [Account; 2],
    /// Blocklister and third pauser on USDC.
    restrictor: Account,
    /// The other swap (OTHER, Open).
    other_id: String,
    prober: Account,
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

async fn setup(code: &[u8]) -> R<U> {
    let worker = near_workspaces::sandbox().await?;
    let mut acc = Vec::new();
    for _ in 0..12 {
        acc.push(worker.dev_create_account().await?);
    }
    let [admin1, admin2, mm1, mm2, owner1, pauser1, pauser2, ctrl1, ctrl2, minter, vadmin, restrictor]: [Account; 12] =
        acc.try_into().map_err(|_| "accounts")?;
    let usdc = worker.dev_deploy(code).await?;
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
    name_account(usdc.id(), "usdc");
    let wnear: AccountId = "wrap.test.near".parse()?;
    let intents = deploy_verifier(&worker, &wnear, &vadmin).await?;
    let escrow = deploy_escrow(&worker, intents.id()).await?;
    let maker = worker.dev_create_account().await?;
    name_account(maker.id(), "maker");
    for who in [intents.id(), maker.id()] {
        ok(
            "usdc.storage_deposit",
            minter
                .call(usdc.id(), "storage_deposit")
                .args_json(json!({ "account_id": who }))
                .deposit(NearToken::from_millinear(10))
                .transact()
                .await?,
        );
    }
    ok(
        "usdc.mint (maker)",
        usdc_call(
            &minter,
            &usdc,
            "mint",
            json!({ "to": maker.id(), "amount": MINTED.to_string() }),
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
    let env = Env {
        worker,
        token: usdc,
        intents,
        escrow,
        maker,
        vadmin,
    };
    let o = okr(
        "maker deposits USDC into its Verifier account",
        maker_deposit(&env, DEPOSIT).await?,
    );
    let _ = o;
    assert_eq!(vbal(&env, env.maker.id().as_str()).await?, DEPOSIT);
    let other_id = open(&env, 200, &taker(200), OTHER).await?;
    let prober = env.worker.dev_create_account().await?;
    Ok(U {
        env,
        pausers: [pauser1, pauser2],
        restrictor,
        other_id,
        prober,
    })
}

#[derive(Clone, Copy, Debug)]
enum Restriction {
    Pause,
    BlocklistVerifier,
    BlocklistMaker,
    BlocklistEscrow,
}

impl Restriction {
    fn panic_text(self) -> &'static str {
        match self {
            Restriction::Pause => "FiatToken: paused",
            _ => "is blocklisted",
        }
    }
    fn target(self, u: &U) -> AccountId {
        match self {
            Restriction::BlocklistVerifier => u.env.intents.id().clone(),
            Restriction::BlocklistMaker => u.env.maker.id().clone(),
            _ => u.env.escrow.id().clone(),
        }
    }
}

/// Prepares a restriction (for a pause, a Pause request approved by two pausers) and
/// returns the signer and the call that applies it.
async fn prepare(u: &U, r: Restriction) -> R<(Account, &'static str, Value)> {
    Ok(match r {
        Restriction::Pause => {
            let id = multisig_request(&u.env.token, [&u.pausers[0], &u.pausers[1]], json!("Pause"))
                .await?;
            (
                u.pausers[1].clone(),
                "execute_multisig_request",
                json!({ "request_id": id }),
            )
        }
        _ => (
            u.restrictor.clone(),
            "blocklist",
            json!({ "account_id": r.target(u) }),
        ),
    })
}

async fn restrict(u: &U, r: Restriction) -> R<()> {
    let (signer, method, args) = prepare(u, r).await?;
    ok(
        &format!("apply {r:?}"),
        usdc_call(&signer, &u.env.token, method, args).await?,
    );
    Ok(())
}

async fn lift(u: &U, r: Restriction) -> R<()> {
    match r {
        Restriction::Pause => {
            multisig(
                &u.env.token,
                [&u.pausers[0], &u.pausers[1]],
                json!("Unpause"),
            )
            .await?
        }
        _ => ok(
            "unblocklist",
            usdc_call(
                &u.restrictor,
                &u.env.token,
                "unblocklist",
                json!({ "account_id": r.target(u) }),
            )
            .await?,
        ),
    }
    assert!(
        u.env.token.view("ft_total_supply").await.is_ok(),
        "USDC still paused"
    );
    Ok(())
}

/// The maker's USDC: plain balance plus Verifier balance. No USDC is created or lost.
async fn maker_total(u: &U) -> R<u128> {
    Ok(ft_balance(&u.env.token, u.env.maker.id()).await?
        + vbal(&u.env, u.env.maker.id().as_str()).await?)
}

/// The other swap is refunded to the maker inside the Verifier: its funds are intact, and
/// the escrow then holds exactly what it still owes.
async fn assert_other_swap_intact(u: &U) -> R<()> {
    let before = vbal(&u.env, u.env.maker.id().as_str()).await?;
    let o = okr(
        "refund the other swap",
        refund(&u.env.maker, &u.env, &u.other_id, GAS_REFUND).await?,
    );
    assert_eq!(
        events(&o, "paid")[0]["data"][0]["amount"],
        OTHER.to_string()
    );
    assert_eq!(
        vbal(&u.env, u.env.maker.id().as_str()).await? - before,
        OTHER
    );
    let (bal, owed) = assert_backed(&u.env, "after the other swap's refund").await?;
    assert_eq!(
        bal, owed,
        "nothing is left in the escrow that it does not owe"
    );
    Ok(())
}

// ===================================================================== normal path

/// Open, claim and payout with USDC: the escrow never calls USDC. Prints the gas.
#[tokio::test]
async fn normal_swap_with_usdc() -> R<()> {
    let code = usdc_or_skip!();
    let u = setup(&code).await?;
    let env = &u.env;
    let (t, id) = claimed(env, &u.prober, 1, AMT, GAS_CLAIM).await?;
    let o = okr(
        "withdraw (USDC)",
        withdraw(&u.prober, env, &id, GAS_WITHDRAW).await?,
    );
    assert!(
        o.receipt_outcomes()
            .iter()
            .all(|r| &r.executor_id != env.token.id()),
        "no USDC receipt"
    );
    assert_eq!(vbal(env, &t.id).await?, AMT);
    assert_eq!(get_swap(env, &id).await?["payout"], "paid");
    assert_eq!(vbal(env, env.escrow.id().as_str()).await?, OTHER);
    assert_eq!(
        ft_balance(&env.token, env.escrow.id()).await.unwrap_or(0),
        0
    );
    assert_backed(env, "normal").await?;
    assert_other_swap_intact(&u).await
}

// ===================================================================== restrictions on open swaps

/// Each USDC restriction is in place while swaps are claimed, paid and refunded. None of
/// them changes anything: the escrow's payouts are inside the Verifier.
#[tokio::test]
async fn usdc_restrictions_do_not_touch_the_escrow() -> R<()> {
    let code = usdc_or_skip!();
    let u = setup(&code).await?;
    let env = &u.env;
    for (i, r) in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistEscrow,
        Restriction::BlocklistMaker,
    ]
    .into_iter()
    .enumerate()
    {
        let seed = 10 + i as u8;
        let t = taker(seed);
        let id = open(env, seed, &t, AMT).await?;
        let rid = open(env, seed + 50, &taker(seed + 50), AMT).await?;
        restrict(&u, r).await?;
        set_ready(env, &id).await?;
        ok(
            "commit",
            call(
                &u.prober,
                env,
                "commit_claim",
                commit_args(env, &t, &id).await?,
                GAS_COMMIT,
            )
            .await?,
        );
        ok(
            &format!("claim with payout while {r:?}"),
            claim(&u.prober, env, &id, 60).await?,
        );
        assert_eq!(get_swap(env, &id).await?["payout"], "paid", "{r:?}");
        assert_eq!(vbal(env, &t.id).await?, AMT, "{r:?}");
        let before = vbal(env, env.maker.id().as_str()).await?;
        ok(
            &format!("refund while {r:?}"),
            refund(&env.maker, env, &rid, GAS_REFUND).await?,
        );
        assert_eq!(get_swap(env, &rid).await?["payout"], "paid", "{r:?}");
        assert_eq!(
            vbal(env, env.maker.id().as_str()).await? - before,
            AMT,
            "{r:?}"
        );
        assert_backed(env, &format!("{r:?}")).await?;
        lift(&u, r).await?;
    }
    assert_other_swap_intact(&u).await
}

// ===================================================================== maker deposit

/// The restriction is in place before the maker's deposit into the Verifier: USDC refuses
/// it and nothing moves. An open for more than the maker's Verifier balance fails in the
/// Verifier and leaves no swap. After the restriction is lifted, the deposit works.
#[tokio::test]
async fn restriction_before_the_maker_deposit_fails_closed() -> R<()> {
    let code = usdc_or_skip!();
    let u = setup(&code).await?;
    let env = &u.env;
    for r in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistMaker,
    ] {
        let total = maker_total(&u).await?;
        let v0 = vbal(env, env.maker.id().as_str()).await?;
        restrict(&u, r).await?;
        let o = maker_deposit(env, 5 * AMT).await?;
        report(&format!("maker deposit while {r:?}"), &o);
        assert!(
            failures_at(&o, env.token.id())
                .iter()
                .any(|f| f.contains(r.panic_text())),
            "{r:?}"
        );
        assert_eq!(
            vbal(env, env.maker.id().as_str()).await?,
            v0,
            "{r:?}: nothing credited"
        );
        // An open for more than the Verifier balance: refused by the Verifier, no swap.
        let now = now_s(&env.worker).await?;
        let msg = terms(90, &taker(90), now + 6000, now + 12000, now + 18000, 6000).to_string();
        let (id, o) = open_msg(env, 90, msg, v0 + 1).await?;
        report("open above the maker's Verifier balance", &o);
        assert!(o.is_failure());
        assert!(get_swap(env, &id).await?.is_null());
        lift(&u, r).await?;
        assert_eq!(
            maker_total(&u).await?,
            total,
            "{r:?}: no USDC created or lost"
        );
    }
    ok("deposit after lift", maker_deposit(env, 5 * AMT).await?);
    assert_backed(env, "end").await?;
    Ok(())
}

/// The race of nv02-usdc.md moved to the maker's deposit: the restriction is sent d blocks
/// after the deposit. At some delays USDC reports failure after the Verifier credited the
/// maker. For every d: the maker's USDC (plain plus Verifier) is unchanged, the maker can
/// open exactly from its Verifier balance, and the escrow's balance backs every swap.
#[tokio::test]
async fn race_during_the_maker_deposit_at_each_delay() -> R<()> {
    let code = usdc_or_skip!();
    for r in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistMaker,
    ] {
        let u = setup(&code).await?;
        let env = &u.env;
        let mut hits = Vec::new();
        for d in 0..=5u64 {
            let total = maker_total(&u).await?;
            let v0 = vbal(env, env.maker.id().as_str()).await?;
            let (signer, method, args) = prepare(&u, r).await?;
            let start = height(&env.worker).await?;
            let (o, applied) = tokio::join!(maker_deposit(env, AMT), async {
                wait_blocks(&env.worker, start, d).await?;
                usdc_call(&signer, &env.token, method, args).await
            });
            let o = o?;
            ok("restriction", applied?);
            let credited = vbal(env, env.maker.id().as_str()).await? - v0;
            let failed = failures_at(&o, env.token.id())
                .iter()
                .any(|f| f.contains(r.panic_text()));
            println!(
                "{r:?} d={d}: USDC call failed: {failed}; Verifier credited the maker {credited}"
            );
            if failed && credited == AMT {
                hits.push(d);
            }
            assert!(
                credited == 0 || credited == AMT,
                "{r:?} d={d}: partial credit {credited}"
            );
            lift(&u, r).await?;
            assert_eq!(
                maker_total(&u).await?,
                total,
                "{r:?} d={d}: USDC created or lost"
            );
            // The maker opens from what the Verifier shows, not from the call's result.
            let t = taker(30 + d as u8);
            let id = open(env, 30 + d as u8, &t, AMT).await?;
            assert_eq!(get_swap(env, &id).await?["amount"], AMT.to_string());
            assert_backed(env, &format!("{r:?} d={d}")).await?;
        }
        println!("{r:?}: delays where USDC reported failure after the Verifier credited the maker: {hits:?}");
        assert!(!hits.is_empty(), "{r:?}: the race was not exercised");
    }
    Ok(())
}

// ===================================================================== refund withdrawal

/// After a refund the maker takes plain USDC out of the Verifier with ft_withdraw. A
/// restriction in place before, or sent d blocks after, makes USDC's transfer fail; the
/// Verifier then gives the maker its balance back. For every d, the maker's USDC (plain
/// plus Verifier) is unchanged.
#[tokio::test]
async fn restriction_during_the_maker_withdrawal_at_each_delay() -> R<()> {
    let code = usdc_or_skip!();
    let u = setup(&code).await?;
    let env = &u.env;
    ok(
        "refund the other swap",
        refund(&env.maker, env, &u.other_id, GAS_REFUND).await?,
    );
    let ft_withdraw = || {
        env.maker
            .call(env.intents.id(), "ft_withdraw")
            .args_json(json!({ "token": env.token.id(), "receiver_id": env.maker.id(), "amount": AMT.to_string() }))
            .deposit(NearToken::from_yoctonear(1))
            .gas(Gas::from_tgas(50))
            .transact()
    };
    for r in [
        Restriction::Pause,
        Restriction::BlocklistVerifier,
        Restriction::BlocklistMaker,
    ] {
        for d in 0..=4u64 {
            let total = maker_total(&u).await?;
            let v0 = vbal(env, env.maker.id().as_str()).await?;
            let (signer, method, args) = prepare(&u, r).await?;
            let start = height(&env.worker).await?;
            let (o, applied) = tokio::join!(ft_withdraw(), async {
                wait_blocks(&env.worker, start, d).await?;
                usdc_call(&signer, &env.token, method, args).await
            });
            let o = o?;
            ok("restriction", applied?);
            let out = v0 - vbal(env, env.maker.id().as_str()).await?;
            println!(
                "{r:?} d={d}: ft_withdraw ok={}; left the Verifier: {out}; USDC failures {:?}",
                o.is_success(),
                failures_at(&o, env.token.id())
            );
            lift(&u, r).await?;
            assert_eq!(
                maker_total(&u).await?,
                total,
                "{r:?} d={d}: USDC created or lost"
            );
        }
    }
    assert_backed(env, "end").await?;
    Ok(())
}

// ===================================================================== payout races

/// Payouts of several swaps race with each other and with the Verifier pausing and
/// unpausing mt_transfer d blocks later. Every swap ends paid exactly once to its own
/// recipient, and the escrow ends holding exactly what it still owes.
#[tokio::test]
async fn payouts_race_with_a_verifier_pause_at_each_delay() -> R<()> {
    let code = usdc_or_skip!();
    let u = setup(&code).await?;
    let env = &u.env;
    let other = env.worker.dev_create_account().await?;
    for d in 0..=3u64 {
        let base = 100 + 10 * d as u8;
        let (ta, ida) = claimed(env, &u.prober, base, AMT, GAS_CLAIM).await?;
        let (tb, idb) = claimed(env, &u.prober, base + 1, 2 * AMT, GAS_CLAIM).await?;
        let start = height(&env.worker).await?;
        let (a, a2, b, p) = tokio::join!(
            withdraw(&u.prober, env, &ida, GAS_WITHDRAW),
            withdraw(&other, env, &ida, GAS_WITHDRAW),
            withdraw(&u.prober, env, &idb, GAS_WITHDRAW),
            async {
                wait_blocks(&env.worker, start, d).await?;
                vadmin_call(env, "pa_pause_feature", json!({ "key": "mt_transfer" })).await
            }
        );
        p?;
        let paid: usize = [a?, a2?, b?].iter().map(|o| events(o, "paid").len()).sum();
        println!("d={d}: payouts that succeeded before the pause: {paid}");
        assert_backed(env, &format!("paused d={d}")).await?;
        vadmin_call(env, "pa_unpause_feature", json!({ "key": "mt_transfer" })).await?;
        for id in [&ida, &idb] {
            if get_swap(env, id).await?["payout"] == "due" {
                ok(
                    "retry withdraw",
                    withdraw(&u.prober, env, id, GAS_WITHDRAW).await?,
                );
            }
            assert_eq!(get_swap(env, id).await?["payout"], "paid");
            assert!(
                withdraw(&u.prober, env, id, GAS_WITHDRAW)
                    .await?
                    .is_failure(),
                "paid once"
            );
        }
        assert_eq!(vbal(env, &ta.id).await?, AMT, "d={d}");
        assert_eq!(vbal(env, &tb.id).await?, 2 * AMT, "d={d}");
        let (bal, owed) = assert_backed(env, &format!("d={d}")).await?;
        assert_eq!((bal, owed), (OTHER, OTHER));
    }
    assert_other_swap_intact(&u).await
}
