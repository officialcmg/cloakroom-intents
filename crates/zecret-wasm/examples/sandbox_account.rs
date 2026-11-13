//! Test helper for the browser scenarios (harness/browser/): creates `<name>.<root>` in a
//! NEAR sandbox with 1 NEAR and registers it with a NEP-141 token, so it can receive the
//! direct exit. Prints the account id.
//!
//! ```sh
//! cargo run -p zecret-wasm --example sandbox_account -- RPC_URL validator_key.json NAME TOKEN
//! ```

use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::rc::Rc;
use zecret_core::near::{tx::Action, At, NearKey, NearRpc, Signer, ONE_NEAR, TGAS};
use zecret_node::io::Http;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let [_, rpc_url, key_file, name, token] = &a[..] else {
        return Err(anyhow!("usage: sandbox_account RPC_URL validator_key.json NAME TOKEN"));
    };
    let key: serde_json::Value = serde_json::from_slice(&std::fs::read(key_file)?)?;
    let root_id = key["account_id"].as_str().ok_or_else(|| anyhow!("no account_id"))?;
    let root = Signer::new(root_id, NearKey::from_secret_str(key["secret_key"].as_str().ok_or_else(|| anyhow!("no secret_key"))?)?);
    let rpc = NearRpc::new(Rc::new(Http::new()?), rpc_url);
    let id = format!("{name}.{root_id}");
    if rpc.account(&id, &At::Optimistic)?.is_none() {
        rpc.transact(&root, &id, vec![Action::CreateAccount, Action::Transfer { deposit: ONE_NEAR }, Action::AddFullAccessKey { public_key: root.key.public_bytes() }])?
            .ok()
            .context("create account")?;
    }
    rpc.transact(&root, token, vec![Action::call("storage_deposit", &json!({"account_id": id}), 30 * TGAS, ONE_NEAR / 80)])?
        .ok()
        .context("storage_deposit")?;
    println!("{id}");
    Ok(())
}
