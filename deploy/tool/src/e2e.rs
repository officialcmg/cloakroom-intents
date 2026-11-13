//! End-to-end check of a deployment at the contract level, with real transactions:
//! the taker's checks, open, set_ready, commit_claim and claim by a fresh taker key with
//! a 0.03 NEAR stipend, withdraw into the Verifier, and a signed `ft_withdraw` built with
//! `zecret-intents` that moves the tokens to a NEAR account. Prints every transaction
//! and its timings.

use ed25519_dalek::{Signer as _, SigningKey};
use near_workspaces::types::{Gas, NearToken, SecretKey};
use serde::Serialize;
use serde_json::json;
use zecret_intents::direct::{EXECUTE_INTENTS_GAS, execute_intents_args};
use zecret_intents::{IntentDraft, IntentSigner, Nonce, Salt, Standard};

use crate::Res;
use crate::chain::TxRec;
use crate::config::{ONE_NEAR, fmt_near};
use crate::deploy::{Ctx, Ids, ensure_registered, ft_balance};
use crate::verify::verify;

// Key shares from contracts/test/CloakroomEscrow.t.sol in the escrow's encodings
// (the same vectors as contracts/near-escrow/tests/sandbox.rs). Public test values.
const A_SEC_BE: &str = "195661e2e4789b7f57137b2e777736d79720270638d12e4882fd95e52a73d6b3";
const A_PUB: &str = "4c303c9656ca22dedd75957bc56c364c6f5461cb0fadca1e589495ebb182a197";
const B_PUB: &str = "950e78fd9d3129d85922bdef19d501fbec46b8c0c0f563d8df66f5e5b86d3a18";

/// 1 token unit of a 6-decimal token.
const AMOUNT: u128 = 1_000_000;
const STIPEND: u128 = 3 * ONE_NEAR / 100;

fn le_hex(be: &str) -> Res<String> {
    let mut b = hex::decode(be)?;
    b.reverse();
    Ok(hex::encode(b))
}

#[derive(Serialize)]
pub struct E2eReport {
    pub network: String,
    pub started_at: String,
    pub escrow: String,
    pub token: String,
    pub verifier: String,
    pub maker: String,
    pub taker: String,
    pub destination: String,
    pub swap_id: String,
    pub amount: String,
    pub taker_near_left_yocto: String,
    pub transactions: Vec<TxRec>,
    pub timing_summary: Timing,
}

#[derive(Serialize, Default)]
pub struct Timing {
    pub samples: usize,
    pub included_s: [f64; 3],
    pub included_final_s: [f64; 3],
    pub final_s: [f64; 3],
}

fn min_med_max(mut v: Vec<f64>) -> [f64; 3] {
    if v.is_empty() {
        return [0.0; 3];
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = if v.len() % 2 == 1 {
        v[v.len() / 2]
    } else {
        (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
    };
    let r = |x: f64| (x * 100.0).round() / 100.0;
    [r(v[0]), r(med), r(v[v.len() - 1])]
}

pub async fn run(ctx: &mut Ctx, ids: &Ids) -> Res<E2eReport> {
    let started_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    println!("== taker's checks before funding");
    let rep = verify(&ctx.chain, &ctx.cfg, &ctx.base, ids).await?;
    if !rep.passed() {
        return Err("the deployment does not pass the taker's checks; not running the swap".into());
    }

    let maker_sk = ctx
        .keys
        .get(&ids.maker)?
        .ok_or_else(|| format!("no key for maker {}", ids.maker))?;
    let maker = ctx.chain.w.account(&ids.maker, maker_sk)?;
    let token_id: near_workspaces::AccountId = ids.token.parse()?;
    let escrow_id: near_workspaces::AccountId = ids.escrow.parse()?;
    let intents_id: near_workspaces::AccountId = ids.intents.parse()?;

    // A fresh taker key, as the browser makes one. Its NEAR account and its Intents
    // account are both the hex of the public key.
    let mut seed = [0u8; 32];
    zecret_intents::random_bytes(&mut seed)?;
    let signer = IntentSigner::from_seed(&seed);
    let taker_id = signer.account_id();
    let dalek = SigningKey::from_bytes(&seed);
    let mut kp = seed.to_vec();
    kp.extend_from_slice(&dalek.verifying_key().to_bytes());
    let taker_sk: SecretKey = format!("ed25519:{}", bs58::encode(kp).into_string()).parse()?;
    let taker = ctx.chain.w.account(&taker_id, taker_sk)?;
    println!("taker key (implicit account) {taker_id}");

    let mut txs: Vec<TxRec> = Vec::new();
    println!("== swap");
    let o = ctx
        .chain
        .track(
            "maker -> taker stipend 0.03 NEAR",
            false,
            maker
                .batch(taker.id())
                .transfer(NearToken::from_yoctonear(STIPEND))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    txs.push(o.rec);

    let now = ctx.chain.rpc.final_timestamp_s().await?;
    let mut salt = [0u8; 32];
    zecret_intents::random_bytes(&mut salt)?;
    let salt = hex::encode(salt);
    let terms = json!({
        "salt": salt,
        "taker_pk": taker_id,
        "a": A_PUB,
        "b": B_PUB,
        "t0": now + 600,
        "t1": now + 1200,
        "t2": now + 1800,
        "reveal_window": 600,
    });
    let o = ctx
        .chain
        .track(
            "open: token.ft_transfer_call -> escrow",
            false,
            maker
                .call(&token_id, "ft_transfer_call")
                .args_json(json!({
                    "receiver_id": ids.escrow,
                    "amount": AMOUNT.to_string(),
                    "msg": terms.to_string(),
                }))
                .deposit(NearToken::from_yoctonear(1))
                .gas(Gas::from_tgas(100))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    if o.events("near_escrow", "opened").len() != 1 || o.json()? != json!(AMOUNT.to_string()) {
        return Err(format!(
            "open: no opened event or wrong amount used; logs {:?}",
            o.logs
        )
        .into());
    }
    txs.push(o.rec);
    let id = ctx
        .chain
        .rpc
        .view_ok(
            &ids.escrow,
            "swap_id",
            &json!({ "maker": ids.maker, "salt": salt }),
        )
        .await?;
    let id = id.as_str().ok_or("swap_id")?.to_string();
    println!("swap id {id}");

    let o = ctx
        .chain
        .track(
            "set_ready (maker)",
            false,
            maker
                .call(&escrow_id, "set_ready")
                .args_json(json!({ "id": id }))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    txs.push(o.rec);

    let a = le_hex(A_SEC_BE)?;
    let commitment = ctx
        .chain
        .rpc
        .view_ok(&ids.escrow, "commitment_of", &json!({ "id": id, "a": a }))
        .await?;
    let msg = ctx
        .chain
        .rpc
        .view_ok(
            &ids.escrow,
            "commit_message",
            &json!({ "id": id, "commitment": commitment }),
        )
        .await?;
    let sig = dalek.sign(&hex::decode(msg.as_str().ok_or("commit_message")?)?);
    let o = ctx
        .chain
        .track(
            "commit_claim (taker key, 10 Tgas)",
            false,
            taker
                .call(&escrow_id, "commit_claim")
                .args_json(json!({
                    "id": id,
                    "commitment": commitment,
                    "signature": hex::encode(sig.to_bytes()),
                }))
                .gas(Gas::from_tgas(10))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    txs.push(o.rec);

    let o = ctx
        .chain
        .track(
            "claim(a) (taker key, 25 Tgas)",
            false,
            taker
                .call(&escrow_id, "claim")
                .args_json(json!({ "id": id, "a": a }))
                .gas(Gas::from_tgas(25))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    let claimed = o.events("near_escrow", "claimed");
    if claimed.len() != 1 || claimed[0]["data"][0]["a"] != json!(a) {
        return Err(format!("claim: no claimed event with a; logs {:?}", o.logs).into());
    }
    txs.push(o.rec);
    let swap = ctx
        .chain
        .rpc
        .view_ok(&ids.escrow, "get_swap", &json!({ "id": id }))
        .await?;
    if swap["state"] != "Claimed" || swap["a"] != json!(a) {
        return Err(format!("swap after claim: {swap}").into());
    }
    println!("swap state Claimed; a is readable from get_swap");

    let o = ctx
        .chain
        .track(
            "withdraw (taker credit -> Verifier, sent by maker)",
            false,
            maker
                .call(&escrow_id, "withdraw")
                .args_json(json!({ "token": ids.token, "owner": taker_id }))
                .gas(Gas::from_tgas(100))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    let w = o.events("near_escrow", "withdrawn");
    if w.len() != 1 || w[0]["data"][0]["amount"] != json!(AMOUNT.to_string()) {
        return Err(format!("withdraw: unexpected events {w:?}; logs {:?}", o.logs).into());
    }
    txs.push(o.rec);
    let token_mt = format!("nep141:{}", ids.token);
    let held = ctx
        .chain
        .rpc
        .view_ok(
            &ids.intents,
            "mt_balance_of",
            &json!({ "account_id": taker_id, "token_id": token_mt }),
        )
        .await?;
    if held != json!(AMOUNT.to_string()) {
        return Err(format!("taker holds {held} in the Verifier, expected {AMOUNT}").into());
    }
    println!(
        "taker holds {AMOUNT} {token_mt} inside the Verifier {}",
        ids.intents
    );

    // The exit: one ft_withdraw signed by the taker key, built by zecret-intents.
    let destination = ctx.owner.id().to_string();
    ensure_registered(ctx, &ids.token, &destination).await?;
    let before = ft_balance(ctx, &ids.token, &destination).await?;
    let salt_hex = ctx
        .chain
        .rpc
        .view_ok(&ids.intents, "current_salt", &json!({}))
        .await?;
    let salt = Salt::from_hex(salt_hex.as_str().ok_or("current_salt")?)?;
    let now_t = chrono::Utc::now();
    let deadline = now_t + chrono::Duration::minutes(10);
    let nonce = Nonce::versioned_random(salt, deadline)?;
    let signed = IntentDraft::new(&taker_id, ids.intents.as_str(), deadline, nonce)
        .ft_withdraw(&ids.token, &destination, AMOUNT)
        .sign(&signer, Standard::RawEd25519, now_t)?;
    let o = ctx
        .chain
        .track(
            "execute_intents(ft_withdraw signed by taker key)",
            false,
            maker
                .call(&intents_id, "execute_intents")
                .args_json(execute_intents_args(&[signed]))
                .gas(Gas::from_gas(EXECUTE_INTENTS_GAS))
                .transact_async(),
        )
        .await?
        .require_ok()?;
    txs.push(o.rec);
    let after = ft_balance(ctx, &ids.token, &destination).await?;
    let left = ctx
        .chain
        .rpc
        .view_ok(
            &ids.intents,
            "mt_balance_of",
            &json!({ "account_id": taker_id, "token_id": token_mt }),
        )
        .await?;
    if after != before + AMOUNT || left != json!("0") {
        return Err(format!(
            "exit: destination {before} -> {after}, taker left {left} in the Verifier"
        )
        .into());
    }
    println!("destination {destination} received {AMOUNT}; taker holds 0 in the Verifier");
    let taker_left = ctx.chain.rpc.balance(&taker_id).await?;
    println!(
        "taker key NEAR left after commit_claim and claim: {} of {}",
        fmt_near(taker_left),
        fmt_near(STIPEND)
    );

    let timing = Timing {
        samples: txs.len(),
        included_s: min_med_max(
            txs.iter()
                .filter_map(|t| t.included_ms)
                .map(|m| m as f64 / 1000.0)
                .collect(),
        ),
        included_final_s: min_med_max(
            txs.iter()
                .filter_map(|t| t.included_final_ms)
                .map(|m| m as f64 / 1000.0)
                .collect(),
        ),
        final_s: min_med_max(
            txs.iter()
                .filter_map(|t| t.final_ms)
                .map(|m| m as f64 / 1000.0)
                .collect(),
        ),
    };
    println!(
        "== timings over {} transactions (seconds: min / median / max)",
        timing.samples
    );
    println!("  submitted -> included:          {:?}", timing.included_s);
    println!(
        "  submitted -> included, final:   {:?}",
        timing.included_final_s
    );
    println!("  submitted -> all receipts final: {:?}", timing.final_s);

    Ok(E2eReport {
        network: ctx.chain.network.as_str().into(),
        started_at,
        escrow: ids.escrow.clone(),
        token: ids.token.clone(),
        verifier: ids.intents.clone(),
        maker: ids.maker.clone(),
        taker: taker_id,
        destination,
        swap_id: id,
        amount: AMOUNT.to_string(),
        taker_near_left_yocto: taker_left.to_string(),
        transactions: txs,
        timing_summary: timing,
    })
}
