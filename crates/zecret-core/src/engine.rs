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
                Some("main") | Some("mainnet") => Net::Main,
                other => bail!("lightwalletd needs network test or main (network {other:?})"),
            };
            Ok(Wallet::new(Backend::Lwd(Lwd::new(t, url, birthday)), net))
        }
        (None, Some(url)) => Ok(Wallet::new(Backend::Zebra(Rpc::new(t, url)), Net::Local(wallet::regtest_params(chains.nu7_height)))),
        (None, None) => bail!("set zebra_rpc or lightwalletd"),
    }
}

/// Holds back repeated warnings and alerts, so that a failing endpoint or a stuck swap
/// does not fill the log (incident 2026-11-14: the same alert once a second for hours).
///
/// Two lines are "the same" when they differ only in numbers and long hashes. A line
/// passes when no same line passed in the last `every` seconds; when it passes again it
/// says how many were held back.
#[derive(Default)]
pub struct Quiet {
    seen: std::collections::HashMap<String, (u64, u64)>,
}

/// A line with its numbers and long hashes replaced, for [`Quiet`].
pub fn line_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut String| {
        if w.len() >= 16 {
            out.push('#');
        } else {
            // "15s" and "7s" are the same line.
            let mut digits = false;
            for c in w.chars() {
                if c.is_ascii_digit() {
                    if !digits {
                        out.push('#');
                    }
                    digits = true;
                } else {
                    out.push(c);
                    digits = false;
                }
            }
        }
        w.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

impl Quiet {
    /// True when the line with this key may be emitted now. `held` is set to the number of
    /// same lines held back since it last passed.
    pub fn pass(&mut self, key: &str, now: u64, every: u64, held: &mut u64) -> bool {
        match self.seen.get_mut(key) {
            Some((last, n)) if now < last.saturating_add(every) => {
                *n += 1;
                false
            }
            Some((last, n)) => {
                *held = *n;
                *last = now;
                *n = 0;
                true
            }
            None => {
                *held = 0;
                self.seen.insert(key.to_string(), (now, 0));
                true
            }
        }
    }

    /// Forgets a key, so its next line passes at once (the problem went away).
    pub fn reset(&mut self, key: &str) {
        self.seen.remove(key);
    }

    /// Filters a tick's events: warnings and alerts that repeat within `every` seconds are
    /// held back. Other events pass.
    pub fn filter(&mut self, events: Vec<Event>, now: u64, every: u64) -> Vec<Event> {
        let mut out = Vec::with_capacity(events.len());
        for e in events {
            let (kind, swap, msg) = match &e {
                Event::Warning { swap, message } => ("w", swap, message),
                Event::Alert { swap, message } => ("a", swap, message),
                _ => {
                    out.push(e);
                    continue;
                }
            };
            let key = format!("{kind}:{swap}:{}", line_key(msg));
            let mut held = 0;
            if self.pass(&key, now, every, &mut held) {
                out.push(match (e, held) {
                    (e, 0) => e,
                    (Event::Warning { swap, message }, n) => Event::Warning { swap, message: format!("{message} (and {n} more like it since the last report)") },
                    (Event::Alert { swap, message }, n) => Event::Alert { swap, message: format!("{message} (and {n} more like it since the last report)") },
                    (e, _) => e,
                });
            }
        }
        // Forget keys not seen for a day.
        self.seen.retain(|_, (last, _)| now < last.saturating_add(86_400));
        out
    }
}

/// Short form of a swap id for logs.
pub fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_holds_back_repeats_and_counts_them() {
        let mut q = Quiet::default();
        let w = |m: &str| Event::Warning { swap: "s".into(), message: m.into() };
        let a = q.filter(vec![w("send_tx DgSSGEXFk2Yo724QSDH6Yjw5ZAoQxTqA7KZUsWBysTtT: 429 at 1794639200")], 100, 60);
        assert_eq!(a.len(), 1);
        // the same line with another hash and time is held back
        let b = q.filter(vec![w("send_tx 2a6wqgEfWAY13H4R4d4enzhLy5ZB5bvz9yeiDUZsAHDv: 429 at 1794640418")], 110, 60);
        assert!(b.is_empty());
        let b = q.filter(vec![w("send_tx 2a6wqgEfWAY13H4R4d4enzhLy5ZB5bvz9yeiDUZsAHDv: 429 at 1794640418")], 120, 60);
        assert!(b.is_empty());
        // another line passes
        assert_eq!(q.filter(vec![w("view get_swap: 429")], 120, 60).len(), 1);
        // numbers inside words do not make a new line
        assert_eq!(line_key("a backs off for 15s after: HTTP 429"), line_key("a backs off for 7s after: HTTP 429"));
        // after the quiet time it passes with the count
        let c = q.filter(vec![w("send_tx FBMBa9SaSg2bGQkrbFK6cFoEjMDuZSgFnMLRHBPTzD93: 429 at 1794640555")], 161, 60);
        match &c[..] {
            [Event::Warning { message, .. }] => assert!(message.ends_with("(and 2 more like it since the last report)"), "{message}"),
            other => panic!("{other:?}"),
        }
        // other events always pass
        assert_eq!(q.filter(vec![Event::Balance { zats: 1 }, Event::Balance { zats: 1 }], 161, 60).len(), 2);
    }
}
