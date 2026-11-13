//! The taker of `zecret-core` over an in-memory store, as the worker drives it.
//!
//! The worker loads every stored record into the [`Session`], calls one method at a time,
//! and after each call writes [`Session::dirty`] to IndexedDB and waits for the write to
//! complete before it calls again. The core never acts on a record in the same call that
//! created it (docs/ENGINE.md §1.2), so a record is durable before anything depends on it.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use zecret_core::{
    config::TakerConfig,
    near::{At, Escrow},
    net::SharedTransport,
    store::{MemStore, Store},
    taker::{self, Taker},
};

pub struct Session {
    pub taker: Taker,
    pub store: MemStore,
    pub transport: SharedTransport,
}

impl Session {
    /// Opens the taker over `records` (key, UTF-8 value). Creates the wallet key if there is
    /// none; that new record is then dirty.
    pub fn open(cfg: TakerConfig, transport: SharedTransport, records: Vec<(String, String)>) -> Result<Session> {
        let mut store = MemStore::new();
        for (k, v) in records {
            store.load(&k, v.into_bytes());
        }
        let taker = Taker::open(cfg, transport.clone(), &mut store)?;
        Ok(Session { taker, store, transport })
    }

    /// The records changed since the last call, as UTF-8 text.
    pub fn dirty(&mut self) -> Vec<(String, String)> {
        self.store.take_dirty().into_iter().map(|(k, v)| (k, String::from_utf8_lossy(&v).into_owned())).collect()
    }

    pub fn record(&self, key: &str) -> Option<String> {
        self.store.get(key).ok().flatten().map(|v| String::from_utf8_lossy(&v).into_owned())
    }

    /// Starts a swap of `zats` with this swap's minimum price (token base units).
    pub fn start(&mut self, zats: u64, min_price: u128, now: u64) -> Result<Value> {
        let s = self.taker.start_with_min_price(zats, min_price, now, &mut self.store)?;
        Ok(serde_json::to_value(s)?)
    }

    /// One tick of the state machine: `{next, events}`.
    pub fn tick(&mut self, now: u64) -> Value {
        let mut events = self.taker.take_events();
        let t = self.taker.tick(now, &mut self.store);
        events.extend(t.events);
        json!({"next": t.next, "events": events})
    }

    /// See `Taker::reconcile`.
    pub fn reconcile(&mut self, now: u64) -> Result<bool> {
        self.taker.reconcile(now, &mut self.store)
    }

    pub fn swap(&self) -> Option<Value> {
        self.taker.swap().and_then(|s| serde_json::to_value(s).ok())
    }

    /// What the worker needs besides the events to draw the website's stages: the
    /// wallet's spendable balance and scan height, and the heights at which the funding
    /// and the recovery sweep were mined.
    pub fn facts(&self) -> Value {
        let w = &self.taker.wallet;
        let s = self.taker.swap();
        let h = |t: Option<&String>| t.and_then(|t| w.mined_height(t));
        json!({
            "balance": self.taker.balance(),
            "height": w.height,
            "fundHeight": h(s.and_then(|s| s.fund_txid.as_ref())),
            "sweepHeight": h(s.and_then(|s| s.sweep_txid.as_ref())),
        })
    }

    /// The height a transaction was mined at, once it is tracked and mined.
    pub fn tx_height(&mut self, txid: &str) -> Option<u64> {
        self.taker.wallet.track(txid);
        self.taker.wallet.mined_height(txid)
    }

    pub fn sync(&mut self) -> Result<u64> {
        self.taker.wallet.sync()?;
        Ok(self.taker.wallet.height)
    }

    /// The escrow's state (`Open`, `Ready`, `Locked`, ...) at a final block, or `None` if
    /// the swap does not exist there.
    pub fn escrow_state(&self, escrow_id: &str) -> Result<Option<String>> {
        let id: [u8; 32] = hex::decode(escrow_id)?.try_into().map_err(|_| anyhow!("escrow id length"))?;
        let c = &self.taker.cfg.chains;
        let e = Escrow { rpc: &self.taker.near, escrow: c.escrow.clone(), token: c.token.clone(), intents: c.intents.clone() };
        Ok(e.get_swap(&id, &At::Final)?.map(|v| format!("{:?}", v.state)))
    }

    /// The token balance of `account` inside the Verifier, at a final block.
    pub fn intents_balance(&self, account: &str) -> Result<u128> {
        let c = &self.taker.cfg.chains;
        let e = Escrow { rpc: &self.taker.near, escrow: c.escrow.clone(), token: c.token.clone(), intents: c.intents.clone() };
        e.intents_balance(account, &At::Final)
    }

    /// The swap token's balance of `account` (a NEAR account), at a final block.
    pub fn token_balance(&self, account: &str) -> Result<u128> {
        let c = &self.taker.cfg.chains;
        let e = Escrow { rpc: &self.taker.near, escrow: c.escrow.clone(), token: c.token.clone(), intents: c.intents.clone() };
        e.ft_balance(account, &At::Final)
    }

    /// Sends what is left in the temporary wallet to `ua`, minus the fee. Proves once.
    pub fn return_leftover(&mut self, ua: &str) -> Result<String> {
        let txid = self.taker.return_leftover(ua)?;
        self.taker.wallet.track(&txid);
        Ok(txid)
    }
}

/// What the user must pay into the temporary wallet for a swap of `zats`.
pub fn required_deposit(zats: u64, max_premium: u64) -> u64 {
    taker::required_deposit(zats, max_premium)
}
