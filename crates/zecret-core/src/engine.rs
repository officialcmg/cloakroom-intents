//! What the maker and taker engines share: the result of a `tick`, the events they emit,
//! Zcash transactions waiting for broadcast, and the wiring of the chain clients.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

use crate::{
    config::Chains,
    net::SharedTransport,
    wallet::{self, Backend, BuiltTx, Net, Wallet},
    zcash::{Lwd, Rpc},
};

/// When to call `tick` again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "at", rename_all = "snake_case")]
pub enum Next {
    /// At or after this Unix time (seconds). A time at or before `now` means "at once".
    At(u64),
    /// Nothing more to do: the swap reached a terminal stage (or there is no swap).
    Done,
}

/// The result of one `tick`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tick {
    pub next: Next,
    pub events: Vec<Event>,
}

/// Something that happened during a tick. Serialized with a `type` field, so a worker can
/// post it to the page as JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The swap moved to a new stage (the stage names are listed in docs/ENGINE.md).
    Stage { swap: String, stage: String },
    /// A human-readable line for a progress log.
    Log { swap: String, line: String },
    /// A Zcash transaction was broadcast.
    ZcashTx { swap: String, what: String, txid: String },
    /// A NEAR transaction was executed.
    NearTx { swap: String, what: String, hash: String },
    /// The shielded balance of the party's own wallet changed (zatoshis, spendable).
    Balance { zats: u64 },
    /// The maker's quote passed every check. Amounts are strings (u128).
    Quote { swap: String, price: String, premium: u64, stipend: String, joint_address: String },
    /// The escrow is open, final and matches the quote; the stipend arrived.
    EscrowVerified { swap: String, escrow_id: String, t0: u64, t1: u64, t2: u64 },
    /// The payout is in `intents.near`, owned by `account` (the taker's NEAR key).
    Payout { swap: String, account: String, token: String, amount: String },
    /// What the engine starts now, emitted in its own tick before blocking work (a proof
    /// takes about 25 s in a browser), so a page can show progress while the worker is busy.
    /// `kind` is `proving`; `estimated_secs` is a typical browser duration.
    Activity { swap: String, kind: String, label: String, estimated_secs: u64 },
    /// Maker only: a swap needs attention (near t1 or t2 and not finished).
    Alert { swap: String, message: String },
    /// A transient problem (network, node); the engine retries on the next tick.
    Warning { swap: String, message: String },
    /// The swap reached a terminal stage.
    Done { swap: String, outcome: String },
    /// Taker only: the maker answered the request with a signed refusal. `code` is
    /// `over_limit`, `under_minimum`, `busy` or `unsupported`; the limits are the maker's.
    /// The swap then ends in `Aborted`; nothing was funded and no premium was paid.
    Refused { swap: String, code: String, min_zats: u64, max_zats: u64 },
    /// Taker only: the claim is final, but the payout is not in the Verifier after
    /// `payout_wait_secs`. Any NEAR account can send this call: `receiver` is the escrow,
    /// `args` is the JSON of the call, `gas` is in gas units, `deposit` is 0. The taker's
    /// stipend does not pay for it. `credit` is what the escrow owes (token base units).
    PayoutPending { swap: String, receiver: String, method: String, args: String, gas: String, deposit: String, credit: String },
}

/// A built Zcash transaction kept in a swap record until it is mined. It is written to
/// the store before its first broadcast, and broadcast again (harmless) after a restart.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingZ {
    pub what: String,
    pub txid: String,
    pub raw_hex: String,
    pub expiry_height: u64,
    /// True once a node accepted it.
    #[serde(default)]
    pub broadcast: bool,
}

impl PendingZ {
    pub fn new(what: &str, tx: &BuiltTx) -> PendingZ {
        PendingZ { what: what.to_string(), txid: tx.txid.clone(), raw_hex: hex::encode(&tx.raw), expiry_height: tx.expiry_height, broadcast: false }
    }
    pub fn tx(&self) -> Result<BuiltTx> {
        Ok(BuiltTx { txid: self.txid.clone(), raw: hex::decode(&self.raw_hex)?, expiry_height: self.expiry_height })
    }
}

/// What happened to a pending transaction.
#[derive(Debug, PartialEq, Eq)]
pub enum PendingState {
    /// Mined at this height.
    Mined(u64),
    /// In flight: a node has it, or the broadcast is to be retried.
    Waiting,
    /// A node refused it, or it expired unmined: it will never be mined.
    Dead(String),
}

/// Broadcast (again) and report where a pending transaction stands. Call after a sync.
pub fn drive_pending(w: &mut Wallet, p: &mut PendingZ) -> Result<PendingState> {
    w.track(&p.txid);
    if let Some(h) = w.mined_height(&p.txid) {
        return Ok(PendingState::Mined(h));
    }
    if w.height > p.expiry_height {
        return Ok(PendingState::Dead(format!("expired unmined at height {}", p.expiry_height)));
    }
    match w.broadcast(&p.tx()?)? {
        wallet::Broadcast::Accepted => {
            p.broadcast = true;
            Ok(PendingState::Waiting)
        }
        // A refusal after an earlier acceptance (for example "already spent" once a block
        // holds it) is not proof that it died: wait for the scan or for its expiry.
        wallet::Broadcast::Refused(m) if p.broadcast => {
            let _ = m;
            Ok(PendingState::Waiting)
        }
        wallet::Broadcast::Refused(m) => Ok(PendingState::Dead(format!("refused by the node: {m}"))),
    }
}

/// The Zcash wallet for these chain settings, over `t`.
pub fn wallet_for(chains: &Chains, t: SharedTransport) -> Result<Wallet> {
    if let Some(n) = &chains.network {
        wallet::set_network(n)?;
    }
    match (&chains.lightwalletd, &chains.zebra_rpc) {
        (Some(url), _) => {
            let birthday = chains.birthday.ok_or_else(|| anyhow!("birthday is required with lightwalletd"))?;
            let net = match chains.network.as_deref() {
                Some("test") | Some("testnet") => Net::Test,
                other => bail!("lightwalletd is supported on testnet only (network {other:?})"),
            };
            Ok(Wallet::new(Backend::Lwd(Lwd::new(t, url, birthday)), net))
        }
        (None, Some(url)) => Ok(Wallet::new(Backend::Zebra(Rpc::new(t, url)), Net::Local(wallet::regtest_params(chains.nu7_height)))),
        (None, None) => bail!("set zebra_rpc or lightwalletd"),
    }
}

/// Short form of a swap id for logs.
pub fn short(id: &str) -> String {
    id.chars().take(8).collect()
}
