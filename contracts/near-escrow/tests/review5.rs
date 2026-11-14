//! Review 5: sandbox tests for the light payout (tokens kept in the Verifier, one
//! `mt_transfer` per payout). See docs/security/review-5-raw.md.
//!
//! Run from `contracts/near-escrow/`:
//!
//! ```sh
//! source ../../.tools/env.sh
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test review5 -- --nocapture --test-threads 1
//! ```

mod common;

use common::*;
use near_workspaces::types::{Gas, NearToken};
use serde_json::json;

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
    // A maker with the longest named account id the root account can make (64
    // characters), so the escrow's events and swap records have their largest size.
    let root = worker.root_account()?;
    let sub = "m".repeat(64 - root.id().as_str().len() - 1);
    let maker = root
        .create_subaccount(&sub)
        .initial_balance(NearToken::from_near(100))
        .transact()
        .await?
        .into_result()?;
    assert_eq!(maker.id().as_str().len(), 64);
    name_account(maker.id(), "maker");
    ok(
        "token.near_deposit",
        maker
            .call(token.id(), "near_deposit")
            .deposit(NearToken::from_near(60))
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
        "escrow.add_storage_funds (5 NEAR)",
        maker
            .call(escrow.id(), "add_storage_funds")
            .args_json(json!({ "token": token.id() }))
            .deposit(NearToken::from_near(5))
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
    ok(
        "maker deposit 50 wNEAR into its Verifier account",
        maker_deposit(&env, 50 * ONE).await?,
    );
    Ok(env)
}

async fn call_gas(
    by: &near_workspaces::Account,
    env: &Env,
    method: &str,
    args: serde_json::Value,
    gas: u64,
) -> R<near_workspaces::result::ExecutionFinalResult> {
    Ok(by
        .call(env.escrow.id(), method)
        .args_json(args)
        .gas(Gas::from_gas(gas))
        .transact()
        .await?)
}

/// One probe: the outcome of a call with `gas` and what it left behind.
#[derive(Debug)]
struct Probe {
    gas: f64,
    ok: bool,
    payout: String,
    error: String,
}

fn assert_monotone(label: &str, probes: &[Probe]) {
    for p in probes {
        println!(
            "PROBE {label} {:>6.2} Tgas ok={} payout={} {}",
            p.gas, p.ok, p.payout, p.error
        );
    }
    // Once a call with some gas succeeds, a call with more gas must not fail. A band where
    // more gas fails means the payout start is taken but the receipt then runs out of gas,
    // and the whole call (with its key share) is reverted while the share is already
    // public in the transaction's arguments.
    let first_ok = probes.iter().position(|p| p.ok);
    if let Some(i) = first_ok {
        let bad: Vec<&Probe> = probes[i..].iter().filter(|p| !p.ok).collect();
        assert!(
            bad.is_empty(),
            "{label}: calls with more gas than a successful call fail: {:?}",
            bad.iter()
                .map(|p| (p.gas, p.error.clone()))
                .collect::<Vec<_>>()
        );
    }
}

/// Gas check (no finding). `refund(b)` in Open before t0, swept across prepaid gas. The
/// payout starts only when `GAS_PAYOUT_START` (18 Tgas) is left after the checks; the
/// test checks that no prepaid gas above the smallest working one makes the call fail.
#[tokio::test]
async fn r5_refund_gas_sweep_has_no_failing_band() -> R<()> {
    let env = setup().await?;
    let mut probes = Vec::new();
    let mut salt = 0u8;
    let mut g = 14_000_000_000_000u64;
    while g <= 42_000_000_000_000 {
        salt += 1;
        let t = taker(salt);
        let id = open(&env, salt, &t, ONE).await?;
        let o = call_gas(
            &env.maker,
            &env,
            "refund",
            json!({ "id": id, "b": le_hex(B_SEC) }),
            g,
        )
        .await?;
        let s = get_swap(&env, &id).await?;
        probes.push(Probe {
            gas: g as f64 / 1e12,
            ok: o.is_success(),
            payout: s["payout"].to_string(),
            error: o
                .clone()
                .into_result()
                .err()
                .map(|e| format!("{e:?}").chars().take(160).collect())
                .unwrap_or_default(),
        });
        if s["payout"] == "due" {
            ok(
                "withdraw",
                withdraw(&env.maker, &env, &id, GAS_WITHDRAW).await?,
            );
        } else if !o.is_success() {
            // Nothing happened: refund again with plenty of gas to keep the escrow clean.
            ok(
                "refund again",
                refund(&env.maker, &env, &id, GAS_REFUND).await?,
            );
        }
        g += if (30_000_000_000_000..38_000_000_000_000).contains(&g) {
            250_000_000_000
        } else {
            1_000_000_000_000
        };
    }
    assert_backed(&env, "after sweep").await?;
    assert_eq!(owed(&env).await?, 0);
    assert_monotone("refund", &probes);
    Ok(())
}

/// Gas check (no finding) for `claim(a)`.
#[tokio::test]
async fn r5_claim_gas_sweep_has_no_failing_band() -> R<()> {
    let env = setup().await?;
    let prober = env.worker.dev_create_account().await?;
    let mut probes = Vec::new();
    let mut seed = 0u8;
    let mut g = 16_000_000_000_000u64;
    while g <= 44_000_000_000_000 {
        seed += 1;
        let t = taker(seed);
        let id = open(&env, seed, &t, ONE / 100).await?;
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
        let o = call_gas(
            &prober,
            &env,
            "claim",
            json!({ "id": id, "a": le_hex(A_SEC) }),
            g,
        )
        .await?;
        let s = get_swap(&env, &id).await?;
        probes.push(Probe {
            gas: g as f64 / 1e12,
            ok: o.is_success(),
            payout: s["payout"].to_string(),
            error: o
                .clone()
                .into_result()
                .err()
                .map(|e| format!("{e:?}").chars().take(160).collect())
                .unwrap_or_default(),
        });
        if s["payout"] == "due" {
            ok(
                "withdraw",
                withdraw(&prober, &env, &id, GAS_WITHDRAW).await?,
            );
        } else if !o.is_success() {
            ok("claim again", claim(&prober, &env, &id, 60).await?);
        }
        g += if (30_000_000_000_000..40_000_000_000_000).contains(&g) {
            250_000_000_000
        } else {
            1_000_000_000_000
        };
    }
    assert_backed(&env, "after sweep").await?;
    assert_monotone("claim", &probes);
    Ok(())
}

/// Finding R5-01 (low). The taker's check (escrow Verifier balance >= get_owed at one final
/// block) reads "not backed" in the blocks from the one in which a payout's `mt_transfer`
/// ran up to the one before its `on_payout`: the balance has dropped by the amount, `owed`
/// has not. A third party can call `withdraw` on any Due payout to open this window.
///
/// The escrow is not changed for it (docs/security/review-5.md): a check that fails closed
/// for a few blocks costs only a re-read, and the taker engine reads again until its
/// funding window nearly closes (`taker_funds_only_when_the_escrow_balance_backs_the_swap`
/// in crates/zecret-core/tests/taker_fixes.rs). This test pins down what the client relies
/// on: the shortfall is exactly the amount in flight, it lasts only until `on_payout`, and
/// from that block on the check passes again.
#[tokio::test]
async fn r5_backing_check_fails_closed_only_while_a_payout_is_in_flight() -> R<()> {
    let env = setup().await?;
    let stranger = env.worker.dev_create_account().await?;
    // Swap 1: the one a taker is checking. Swap 2: claimed with low gas, payout Due.
    let t1 = taker(1);
    let _id1 = open(&env, 1, &t1, AMOUNT).await?;
    let (_t2, id2) = claimed(&env, &stranger, 2, ONE, GAS_CLAIM).await?;
    assert_eq!(get_swap(&env, &id2).await?["payout"], "due");
    assert_backed(&env, "before withdraw").await?;

    let o = okr(
        "withdraw by a stranger",
        withdraw(&stranger, &env, &id2, GAS_WITHDRAW).await?,
    );
    let mt = o
        .receipt_outcomes()
        .iter()
        .find(|r| r.executor_id == *env.intents.id())
        .cloned()
        .ok_or("no Verifier receipt")?;
    let cb = o
        .receipt_outcomes()
        .iter()
        .rev()
        .find(|r| {
            r.executor_id == *env.escrow.id() && r.logs.iter().any(|l| l.contains("\"paid\""))
        })
        .cloned()
        .ok_or("no on_payout receipt")?;
    let mt_block = env.worker.view_block().block_hash(mt.block_hash).await?;
    let cb_block = env.worker.view_block().block_hash(cb.block_hash).await?;
    println!(
        "mt_transfer ran in block {}, on_payout in block {}",
        mt_block.height(),
        cb_block.height()
    );
    let mut violations = Vec::new();
    for h in mt_block.height()..=cb_block.height() + 2 {
        let bal: String = env
            .intents
            .view("mt_balance_of")
            .block_height(h)
            .args_json(json!({ "account_id": env.escrow.id(), "token_id": token_id(&env) }))
            .await?
            .json()?;
        let ow: String = env
            .escrow
            .view("get_owed")
            .block_height(h)
            .args_json(json!({ "token": env.token.id() }))
            .await?
            .json()?;
        let (bal, ow): (u128, u128) = (bal.parse()?, ow.parse()?);
        println!("block {h}: escrow Verifier balance {bal}, owed {ow}");
        if bal < ow {
            violations.push((h, bal, ow));
        }
    }
    println!("blocks where the check reads 'not backed': {violations:?}");
    for (h, bal, ow) in &violations {
        assert!(
            *h < cb_block.height(),
            "the check still fails at block {h}, after on_payout ran"
        );
        assert_eq!(
            bal + ONE,
            *ow,
            "the shortfall is exactly the payout in flight"
        );
    }
    assert_backed(&env, "after on_payout").await?;
    Ok(())
}

/// Checks the margin of GAS_ON_PAYOUT (8 Tgas) for the largest swap record and event
/// (64-character maker id). If `on_payout` ran out of gas, the payout would stay Sending
/// forever and `owed` would never drop.
#[tokio::test]
async fn r5_on_payout_gas_margin_with_longest_ids() -> R<()> {
    let env = setup().await?;
    let t = taker(9);
    let id = open(&env, 9, &t, AMOUNT).await?;
    let o = okr(
        "refund (pays the 64-char maker)",
        refund(&env.maker, &env, &id, GAS_REFUND).await?,
    );
    let cb = o
        .receipt_outcomes()
        .iter()
        .find(|r| {
            r.executor_id == *env.escrow.id() && r.logs.iter().any(|l| l.contains("\"paid\""))
        })
        .cloned()
        .ok_or("no on_payout receipt")?;
    let mt = o
        .receipt_outcomes()
        .iter()
        .find(|r| r.executor_id == *env.intents.id())
        .cloned()
        .ok_or("no mt_transfer receipt")?;
    println!(
        "on_payout burnt {:.3} Tgas (reserved 8); mt_transfer burnt {:.3} Tgas (reserved 7)",
        tgas(cb.gas_burnt),
        tgas(mt.gas_burnt)
    );
    assert!(tgas(cb.gas_burnt) < 4.0, "on_payout margin below 2x");
    assert!(
        tgas(mt.gas_burnt) < 7.0,
        "mt_transfer above its reservation"
    );
    Ok(())
}

/// Finding R5-02 (info). The Verifier's DAO / UnrestrictedWithdrawer role can move tokens out of the escrow's
/// Verifier account with `mt_force_transfer` at any time, also after a taker checked the
/// backing and sent ZEC. Afterwards the payouts of the escrow's swaps fail and stay Due.
/// This passes: it documents a trust assumption (already listed in fixes-2.md), not a
/// bug in the escrow.
#[tokio::test]
async fn r5_verifier_operator_can_drain_the_escrow() -> R<()> {
    let env = setup().await?;
    // Give vadmin the UnrestrictedWithdrawer role (it is super admin).
    ok(
        "acl_grant_role",
        env.vadmin
            .call(env.intents.id(), "acl_grant_role")
            .args_json(json!({ "role": "UnrestrictedWithdrawer", "account_id": env.vadmin.id() }))
            .gas(Gas::from_tgas(30))
            .transact()
            .await?,
    );
    let prober = env.worker.dev_create_account().await?;
    let (t, id) = claimed(&env, &prober, 3, AMOUNT, GAS_CLAIM).await?;
    assert_backed(&env, "taker checked").await?;
    ok(
        "mt_force_transfer escrow -> vadmin",
        env.vadmin
            .call(env.intents.id(), "mt_force_transfer")
            .args_json(json!({
                "owner_id": env.escrow.id(),
                "receiver_id": env.vadmin.id(),
                "token_id": token_id(&env),
                "amount": AMOUNT.to_string(),
            }))
            .deposit(NearToken::from_yoctonear(1))
            .gas(Gas::from_tgas(30))
            .transact()
            .await?,
    );
    let o = withdraw(&prober, &env, &id, GAS_WITHDRAW).await?;
    assert_eq!(events(&o, "payout_failed").len(), 1);
    assert_eq!(vbal(&env, &t.id).await?, 0);
    assert_eq!(get_swap(&env, &id).await?["payout"], "due");
    Ok(())
}

/// Gas check close-up (no finding): bisects the prepaid gas of `refund` down to 0.01 Tgas around the point
/// where the payout starts in the same call (only `GAS_PAYOUT_START` left after the
/// checks). At every probe the call must succeed: payout Due below the point, Paid above.
#[tokio::test]
async fn r5_refund_at_the_exact_payout_threshold() -> R<()> {
    let env = setup().await?;
    let mut salt = 100u8;
    // (gas, ok, payout)
    let mut probe = |g: u64| {
        salt += 1;
        let s = salt;
        let env = &env;
        async move {
            let t = taker(s);
            let id = open(env, s, &t, ONE / 100).await?;
            let o = call_gas(
                &env.maker,
                env,
                "refund",
                json!({ "id": id, "b": le_hex(B_SEC) }),
                g,
            )
            .await?;
            let p = get_swap(env, &id).await?["payout"].to_string();
            println!(
                "PROBE refund {:>9.4} Tgas ok={} payout={p}",
                g as f64 / 1e12,
                o.is_success()
            );
            if p == "\"due\"" {
                ok(
                    "withdraw",
                    withdraw(&env.maker, env, &id, GAS_WITHDRAW).await?,
                );
            }
            Ok::<_, Box<dyn std::error::Error>>((o.is_success(), p))
        }
    };
    let (mut lo, mut hi) = (30_000_000_000_000u64, 40_000_000_000_000u64);
    let (ok_lo, p_lo) = probe(lo).await?;
    let (ok_hi, p_hi) = probe(hi).await?;
    assert!(ok_lo && p_lo == "\"due\"" && ok_hi && p_hi == "\"paid\"");
    while hi - lo > 10_000_000_000 {
        let mid = (lo + hi) / 2;
        let (ok, p) = probe(mid).await?;
        assert!(ok, "refund with {mid} gas failed");
        if p == "\"paid\"" {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    // Every 0.001 Tgas across the last step.
    let mut g = lo;
    while g <= hi {
        let (ok, _) = probe(g).await?;
        assert!(ok, "refund with {g} gas failed");
        g += 1_000_000_000;
    }
    println!("payout starts between {lo} and {hi} gas");
    Ok(())
}
