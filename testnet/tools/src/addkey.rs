//! Adds a full-access key to a NEAR testnet account, signed with a key the account
//! already has. Used to give the hosted maker (docs/HOSTING.md) its own key on
//! maker.zecret-3f109eea1e.testnet: the hosted service creates the key on its volume and
//! prints only the public key.
//! usage: zecret-testnet-addkey SIGNER_KEY_FILE ACCOUNT NEW_PUBLIC_KEY
//! SIGNER_KEY_FILE holds `ed25519:...` or a NEAR key JSON with `private_key`. The secret
//! is never printed.
use anyhow::{anyhow, bail, Result};
use std::rc::Rc;
use zecret_core::near::tx::parse_public_key;
use zecret_core::near::{Action, At, NearKey, NearRpc, Signer};

const RPC: &str = "https://rpc.testnet.fastnear.com";

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        bail!("usage: zecret-testnet-addkey SIGNER_KEY_FILE ACCOUNT NEW_PUBLIC_KEY");
    }
    let raw = std::fs::read_to_string(&a[1])?;
    let secret = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) if v.is_object() => v["private_key"].as_str().ok_or(anyhow!("no private_key in {}", a[1]))?.to_string(),
        _ => raw.trim().to_string(),
    };
    let key = NearKey::from_secret_str(&secret)?;
    let account = &a[2];
    let new_pk = parse_public_key(&a[3])?;
    if !account.ends_with(".testnet") {
        bail!("{account} is not a testnet account");
    }
    let rpc = NearRpc::new(Rc::new(zecret_node::io::Http::new()?), RPC);
    if rpc.access_key(account, &a[3], &At::Final)?.is_some() {
        println!("{} is already a key of {account}", a[3]);
        return Ok(());
    }
    println!("signer key {} of {account}", key.public_key_str());
    let out = rpc.transact(&Signer::new(account, key), account, vec![Action::AddFullAccessKey { public_key: new_pk }])?;
    println!("add_key tx {} status {}", out.hash, out.status);
    out.ok()?;
    let keys = rpc.access_keys(account, &At::Optimistic)?;
    println!("keys of {account} now: {}", keys.join(", "));
    Ok(())
}
