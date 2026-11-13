//! Sets up a local NEAR sandbox for tests: accounts, the wrap.near token code, the live
//! intents.near code and the escrow, all through the core's own transaction code.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::json;
use zecret_core::near::{tx::Action, At, NearKey, NearRpc, Signer, ONE_NEAR, TGAS};

#[derive(Serialize)]
pub struct Devnet {
    pub near_rpc: String,
    pub token: String,
    pub intents: String,
    pub escrow: String,
    pub maker: String,
    pub maker_public_key: String,
}

pub struct Setup<'a> {
    pub rpc: &'a NearRpc,
    pub root: Signer,
    pub wrap_wasm: Vec<u8>,
    pub intents_wasm: Vec<u8>,
    pub escrow_wasm: Vec<u8>,
    pub maker_key: NearKey,
    /// wNEAR the maker holds after setup (yoctoNEAR).
    pub maker_wnear: u128,
}

fn ok(what: &str, out: zecret_core::near::TxOutcome) -> Result<zecret_core::near::TxOutcome> {
    out.ok().with_context(|| what.to_string())
}

impl Setup<'_> {
    fn create(&self, name: &str, balance: u128, key: &NearKey, code: Option<&[u8]>) -> Result<String> {
        let id = format!("{name}.{}", self.root.account_id);
        if self.rpc.account(&id, &At::Optimistic)?.is_some() {
            return Err(anyhow!("account {id} exists already; start from a fresh sandbox"));
        }
        let mut actions = vec![Action::CreateAccount, Action::Transfer { deposit: balance }, Action::AddFullAccessKey { public_key: key.public_bytes() }];
        if let Some(c) = code {
            actions.push(Action::DeployContract { code: c.to_vec() });
        }
        ok(&format!("create {id}"), self.rpc.transact(&self.root, &id, actions)?)?;
        Ok(id)
    }

    pub fn run(&self) -> Result<Devnet> {
        let ck = &self.root.key;
        let token = self.create("wrap", 50 * ONE_NEAR, ck, Some(&self.wrap_wasm))?;
        let intents = self.create("intents", 50 * ONE_NEAR, ck, Some(&self.intents_wasm))?;
        let escrow = self.create("escrow", 50 * ONE_NEAR, ck, Some(&self.escrow_wasm))?;
        let maker = self.create("maker", self.maker_wnear + 100 * ONE_NEAR, &self.maker_key, None)?;
        let as_contract = |id: &str| Signer::new(id, ck.clone());
        ok("wrap.new", self.rpc.transact(&as_contract(&token), &token, vec![Action::call("new", &json!({}), 30 * TGAS, 0)])?)?;
        ok(
            "intents.new",
            self.rpc.transact(
                &as_contract(&intents),
                &intents,
                vec![Action::call("new", &json!({"config": {"wnear_id": token, "fees": {"fee": 0, "fee_collector": intents}}}), 300 * TGAS, 0)],
            )?,
        )?;
        ok("escrow.new", self.rpc.transact(&as_contract(&escrow), &escrow, vec![Action::call("new", &json!({"intents_account": intents}), 30 * TGAS, 0)])?)?;
        // Token storage for the escrow and Intents, then the maker's wNEAR.
        for who in [&escrow, &intents] {
            ok(
                &format!("storage_deposit {who}"),
                self.rpc.transact(&self.root, &token, vec![Action::call("storage_deposit", &json!({"account_id": who}), 30 * TGAS, ONE_NEAR / 80)])?,
            )?;
        }
        let m = Signer::new(&maker, self.maker_key.clone());
        ok("near_deposit", self.rpc.transact(&m, &token, vec![Action::call("near_deposit", &json!({}), 30 * TGAS, self.maker_wnear)])?)?;
        Ok(Devnet {
            near_rpc: self.rpc.url.clone(),
            token,
            intents,
            escrow,
            maker,
            maker_public_key: self.maker_key.public_key_str(),
        })
    }
}
