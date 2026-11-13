//! The token in a NEAR sandbox (nearcore 2.14.1): `new` is private, the faucet mints, and
//! `ft_transfer_call` to a receiver without `ft_on_transfer` returns the tokens through
//! the standard `ft_resolve_transfer`.
//!
//! ```sh
//! cd /home/user/zecret && source .tools/env.sh && cd deploy
//! cargo build -p test-usdc --target wasm32-unknown-unknown --release
//! LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p test-usdc --test sandbox
//! ```
#![cfg(not(target_arch = "wasm32"))]

use near_workspaces::types::{Gas, NearToken};
use serde_json::json;

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn wasm() -> Vec<u8> {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../target/wasm32-unknown-unknown/release/test_usdc.wasm"
    );
    std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}; build the wasm first"))
}

async fn balance(token: &near_workspaces::Contract, who: &near_workspaces::AccountId) -> R<u128> {
    let v: String = token
        .view("ft_balance_of")
        .args_json(json!({ "account_id": who }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

#[tokio::test]
async fn token_in_sandbox() -> R<()> {
    let worker = near_workspaces::sandbox().await?;
    let token = worker.dev_deploy(&wasm()).await?;
    let alice = worker.dev_create_account().await?;
    let empty = worker.dev_create_account().await?;

    // new is private.
    let o = alice.call(token.id(), "new").transact().await?;
    assert!(o.is_failure());
    assert!(format!("{:?}", o.into_result().err()).contains("Method new is private"));
    token.call("new").transact().await?.into_result()?;

    // Faucet: registers alice with the attached deposit.
    let min: serde_json::Value = token.view("storage_balance_bounds").await?.json()?;
    let min: u128 = min["min"].as_str().unwrap().parse()?;
    alice
        .call(token.id(), "mint")
        .args_json(json!({ "amount": "5000000" }))
        .deposit(NearToken::from_yoctonear(min))
        .transact()
        .await?
        .into_result()?;
    assert_eq!(balance(&token, alice.id()).await?, 5_000_000);

    // Over the cap: refused.
    let o = alice
        .call(token.id(), "mint")
        .args_json(json!({ "amount": "10000000001" }))
        .transact()
        .await?;
    assert!(format!("{:?}", o.into_result().err()).contains("at most"));

    // ft_transfer_call to an account without code: the receiver call fails and the
    // resolve step gives all tokens back.
    alice
        .call(token.id(), "storage_deposit")
        .args_json(json!({ "account_id": empty.id() }))
        .deposit(NearToken::from_yoctonear(min))
        .transact()
        .await?
        .into_result()?;
    let o = alice
        .call(token.id(), "ft_transfer_call")
        .args_json(json!({ "receiver_id": empty.id(), "amount": "1000000", "msg": "" }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        o.is_success(),
        "the outer call succeeds: {:?}",
        o.failures()
    );
    let used: String = o.json()?;
    assert_eq!(used, "0");
    assert_eq!(balance(&token, alice.id()).await?, 5_000_000);
    assert_eq!(balance(&token, empty.id()).await?, 0);
    Ok(())
}
