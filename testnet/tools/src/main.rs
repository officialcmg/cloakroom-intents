//! Testnet exit: the taker key signs one ft_withdraw intent in the stand-in Verifier;
//! the maker account relays it with execute_intents (as deploy's e2e does).
//! usage: exit SWAP_RECORD.json MAKER_NEAR_KEY_FILE DESTINATION [--dry]
use anyhow::{anyhow, Result};
use serde_json::json;
use std::rc::Rc;
use zecret_core::near::{Action, NearKey, NearRpc, Signer, At};
use zecret_intents::direct::{execute_intents_args, EXECUTE_INTENTS_GAS};
use zecret_intents::{IntentDraft, IntentSigner, Nonce, Salt, Standard};

const RPC: &str = "https://rpc.testnet.fastnear.com";
const VERIFIER: &str = "intents.zecret-3f109eea1e.testnet";
const TOKEN: &str = "usdc.zecret-3f109eea1e.testnet";
const MAKER: &str = "maker.zecret-3f109eea1e.testnet";

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let rec: serde_json::Value = serde_json::from_slice(&std::fs::read(&a[1])?)?;
    let seed: [u8; 32] = hex::decode(rec["near_sk"].as_str().ok_or(anyhow!("near_sk"))?)?
        .try_into().map_err(|_| anyhow!("seed length"))?;
    let taker = IntentSigner::from_seed(&seed);
    let taker_id = taker.account_id();
    assert_eq!(taker_id, rec["taker_account"].as_str().unwrap_or_default());
    let dest = a[3].clone();
    let dry = a.get(4).map(|s| s == "--dry").unwrap_or(false);
    let rpc = NearRpc::new(Rc::new(zecret_node::io::Http::new()?), RPC);
    let tok = format!("nep141:{TOKEN}");
    let held: u128 = rpc.view(VERIFIER, "mt_balance_of", &json!({"account_id": taker_id, "token_id": tok}), &At::Final)?
        .as_str().ok_or(anyhow!("balance"))?.parse()?;
    let dest_before = rpc.view(TOKEN, "ft_balance_of", &json!({"account_id": dest}), &At::Final)?;
    println!("taker {taker_id} holds {held} {tok} in {VERIFIER}; destination {dest} holds {dest_before}");
    if held == 0 || dry { return Ok(()); }
    let salt_hex = rpc.view(VERIFIER, "current_salt", &json!({}), &At::Final)?;
    let salt = Salt::from_hex(salt_hex.as_str().ok_or(anyhow!("salt"))?)?;
    let now = chrono::Utc::now();
    let deadline = now + chrono::Duration::minutes(10);
    let nonce = Nonce::versioned_random(salt, deadline)?;
    let signed = IntentDraft::new(&taker_id, VERIFIER, deadline, nonce)
        .ft_withdraw(TOKEN, &dest, held)
        .sign(&taker, Standard::RawEd25519, now)?;
    let mk = NearKey::from_secret_str(std::fs::read_to_string(&a[2])?.trim())?;
    let relayer = Signer::new(MAKER, mk);
    let t0 = std::time::Instant::now();
    let out = rpc.transact(&relayer, VERIFIER, vec![Action::call("execute_intents", &execute_intents_args(&[signed]), EXECUTE_INTENTS_GAS, 0)])?;
    println!("execute_intents tx {} status {} ({} ms)", out.hash, out.status, t0.elapsed().as_millis());
    let out = out.ok()?;
    println!("logs: {:?}", out.logs);
    let left = rpc.view(VERIFIER, "mt_balance_of", &json!({"account_id": taker_id, "token_id": tok}), &At::Optimistic)?;
    let dest_after = rpc.view(TOKEN, "ft_balance_of", &json!({"account_id": dest}), &At::Optimistic)?;
    println!("after: taker left {left} in the Verifier; destination {dest} holds {dest_after}");
    Ok(())
}
