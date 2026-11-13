//! NEAR JSON-RPC over the core's `Transport`: blocks, view calls, accounts, access keys,
//! and sending transactions.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::cell::Cell;

use super::tx::{Action, NearKey, SignedTx, Transaction};
use crate::net::SharedTransport;

/// Which state a view reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    /// The latest final block. Protocol decisions read here.
    Final,
    /// The head. Used for nonces and to see a transaction quickly.
    Optimistic,
    /// A given block (base58 hash).
    Block(String),
}

impl At {
    fn params(&self) -> Value {
        match self {
            At::Final => json!({"finality": "final"}),
            At::Optimistic => json!({"finality": "optimistic"}),
            At::Block(h) => json!({"block_id": h}),
        }
    }
}

/// How long `send_tx` waits (NEAR's `wait_until`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitUntil {
    None,
    Included,
    ExecutedOptimistic,
    IncludedFinal,
    Executed,
    Final,
}

impl WaitUntil {
    fn as_str(self) -> &'static str {
        match self {
            WaitUntil::None => "NONE",
            WaitUntil::Included => "INCLUDED",
            WaitUntil::ExecutedOptimistic => "EXECUTED_OPTIMISTIC",
            WaitUntil::IncludedFinal => "INCLUDED_FINAL",
            WaitUntil::Executed => "EXECUTED",
            WaitUntil::Final => "FINAL",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub height: u64,
    pub hash: String,
    pub hash_bytes: [u8; 32],
    /// Block timestamp in nanoseconds.
    pub timestamp_ns: u64,
}

impl BlockInfo {
    /// Block time in seconds, as the escrow's `now()` computes it.
    pub fn secs(&self) -> u64 {
        self.timestamp_ns / 1_000_000_000
    }
}

#[derive(Clone, Debug)]
pub struct AccountInfo {
    pub amount: u128,
    pub storage_usage: u64,
}

#[derive(Clone, Debug)]
pub struct AccessKeyInfo {
    pub nonce: u64,
    pub full_access: bool,
}

/// The outcome of a transaction, as far as `send_tx` or `tx` saw it.
#[derive(Clone, Debug)]
pub struct TxOutcome {
    pub hash: String,
    /// `final_execution_status` from the RPC.
    pub status: String,
    /// Some(Ok(value)) when executed successfully (the decoded SuccessValue, possibly
    /// empty), Some(Err(failure)) when the transaction or any receipt failed, None when
    /// not executed yet.
    pub result: Option<std::result::Result<Vec<u8>, String>>,
    pub logs: Vec<String>,
}

impl TxOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self.result, Some(Ok(_)))
    }
    pub fn failure(&self) -> Option<&str> {
        match &self.result {
            Some(Err(e)) => Some(e),
            _ => None,
        }
    }
    /// Ok if executed with no failure; an error naming the failure otherwise.
    pub fn ok(self) -> Result<TxOutcome> {
        match &self.result {
            Some(Ok(_)) => Ok(self),
            Some(Err(e)) => bail!("transaction {} failed: {e}", self.hash),
            None => bail!("transaction {} not executed yet (status {})", self.hash, self.status),
        }
    }

    fn parse(hash: &str, v: &Value) -> TxOutcome {
        let status = v["final_execution_status"].as_str().unwrap_or("").to_string();
        let mut logs = vec![];
        let mut failure: Option<String> = None;
        for o in std::iter::once(&v["transaction_outcome"]).chain(v["receipts_outcome"].as_array().into_iter().flatten()) {
            let out = &o["outcome"];
            for l in out["logs"].as_array().into_iter().flatten() {
                if let Some(s) = l.as_str() {
                    logs.push(s.to_string());
                }
            }
            if let Some(f) = out["status"].get("Failure") {
                failure.get_or_insert_with(|| format!("{} at {}", f, o["outcome"]["executor_id"].as_str().unwrap_or("?")));
            }
        }
        let result = match v.get("status") {
            None => None,
            Some(st) => {
                if let Some(f) = failure.or_else(|| st.get("Failure").map(|f| f.to_string())) {
                    Some(Err(f))
                } else if let Some(b64) = st.get("SuccessValue").and_then(|x| x.as_str()) {
                    Some(Ok(base64::engine::general_purpose::STANDARD.decode(b64).unwrap_or_default()))
                } else if st.get("SuccessReceiptId").is_some() {
                    Some(Ok(vec![]))
                } else {
                    None
                }
            }
        };
        TxOutcome { hash: hash.to_string(), status, result, logs }
    }
}

/// An RPC error, with the cause name NEAR gives (e.g. `UNKNOWN_ACCOUNT`, `TIMEOUT_ERROR`).
#[derive(Debug)]
pub struct RpcError {
    pub name: String,
    pub cause: String,
    pub detail: Value,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NEAR RPC error {} / {}: {}", self.name, self.cause, self.detail)
    }
}

impl std::error::Error for RpcError {}

/// The cause name of a NEAR RPC error inside an `anyhow` error, if it is one.
pub fn rpc_cause(e: &anyhow::Error) -> Option<&str> {
    e.downcast_ref::<RpcError>().map(|r| r.cause.as_str())
}

/// A NEAR account that signs transactions.
pub struct Signer {
    pub account_id: String,
    pub key: NearKey,
    /// The highest nonce this process has used, so that two transactions sent before the
    /// first is visible never share a nonce.
    last_nonce: Cell<u64>,
}

impl Signer {
    pub fn new(account_id: &str, key: NearKey) -> Signer {
        Signer { account_id: account_id.to_string(), key, last_nonce: Cell::new(0) }
    }
}

pub struct NearRpc {
    t: SharedTransport,
    pub url: String,
}

impl NearRpc {
    pub fn new(t: SharedTransport, url: &str) -> NearRpc {
        NearRpc { t, url: url.to_string() }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let req = json!({"jsonrpc": "2.0", "id": "zecret", "method": method, "params": params});
        let r = self
            .t
            .post(&self.url, &[("content-type", "application/json")], req.to_string().as_bytes())
            .with_context(|| format!("NEAR RPC {method}"))?;
        let v: Value = serde_json::from_slice(&r.body)
            .with_context(|| format!("NEAR RPC {method}: HTTP {} {}", r.status, String::from_utf8_lossy(&r.body[..r.body.len().min(200)])))?;
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(RpcError {
                name: e["name"].as_str().unwrap_or("").to_string(),
                cause: e["cause"]["name"].as_str().unwrap_or("").to_string(),
                detail: e.get("cause").and_then(|c| c.get("info")).cloned().unwrap_or_else(|| e.clone()),
            }
            .into());
        }
        // A view call that panicked comes back as a result with an `error` string.
        if let Some(err) = v["result"].get("error").and_then(|e| e.as_str()) {
            bail!("NEAR RPC {method}: {err}");
        }
        Ok(v["result"].clone())
    }

    pub fn block(&self, at: &At) -> Result<BlockInfo> {
        let v = self.call("block", at.params())?;
        let h = &v["header"];
        let hash = h["hash"].as_str().ok_or_else(|| anyhow!("block without hash"))?.to_string();
        let hb = bs58::decode(&hash).into_vec()?;
        Ok(BlockInfo {
            height: h["height"].as_u64().ok_or_else(|| anyhow!("block without height"))?,
            hash_bytes: hb.try_into().map_err(|_| anyhow!("block hash length"))?,
            hash,
            timestamp_ns: h["timestamp"].as_u64().or_else(|| h["timestamp_nanosec"].as_str().and_then(|s| s.parse().ok())).ok_or_else(|| anyhow!("block without timestamp"))?,
        })
    }

    /// Calls a view method with JSON arguments and decodes its JSON result.
    pub fn view(&self, contract: &str, method: &str, args: &Value, at: &At) -> Result<Value> {
        let mut p = at.params();
        p["request_type"] = json!("call_function");
        p["account_id"] = json!(contract);
        p["method_name"] = json!(method);
        p["args_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(args.to_string()));
        let v = self.call("query", p).with_context(|| format!("view {contract}.{method}"))?;
        let bytes: Vec<u8> = v["result"].as_array().ok_or_else(|| anyhow!("view {method}: no result"))?.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect();
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).with_context(|| format!("view {contract}.{method}: result is not JSON"))
    }

    /// None if the account does not exist.
    pub fn account(&self, account: &str, at: &At) -> Result<Option<AccountInfo>> {
        let mut p = at.params();
        p["request_type"] = json!("view_account");
        p["account_id"] = json!(account);
        match self.call("query", p) {
            Ok(v) => Ok(Some(AccountInfo {
                amount: v["amount"].as_str().ok_or_else(|| anyhow!("account without amount"))?.parse()?,
                storage_usage: v["storage_usage"].as_u64().unwrap_or(0),
            })),
            Err(e) if rpc_cause(&e) == Some("UNKNOWN_ACCOUNT") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// None if the account or the key does not exist.
    pub fn access_key(&self, account: &str, public_key: &str, at: &At) -> Result<Option<AccessKeyInfo>> {
        let mut p = at.params();
        p["request_type"] = json!("view_access_key");
        p["account_id"] = json!(account);
        p["public_key"] = json!(public_key);
        match self.call("query", p) {
            Ok(v) => Ok(Some(AccessKeyInfo {
                nonce: v["nonce"].as_u64().ok_or_else(|| anyhow!("access key without nonce"))?,
                full_access: v["permission"].as_str() == Some("FullAccess"),
            })),
            Err(e) if matches!(rpc_cause(&e), Some("UNKNOWN_ACCOUNT") | Some("UNKNOWN_ACCESS_KEY")) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Builds and signs a transaction with the next nonce of `signer`'s key and a recent
    /// final block hash.
    pub fn sign(&self, signer: &Signer, receiver: &str, actions: Vec<Action>) -> Result<SignedTx> {
        let pk = signer.key.public_key_str();
        let ak = self
            .access_key(&signer.account_id, &pk, &At::Optimistic)?
            .ok_or_else(|| anyhow!("{} has no access key {pk}", signer.account_id))?;
        let nonce = ak.nonce.max(signer.last_nonce.get()) + 1;
        let block = self.block(&At::Final)?;
        let tx = Transaction {
            signer_id: signer.account_id.clone(),
            public_key: signer.key.public_bytes(),
            nonce,
            receiver_id: receiver.to_string(),
            block_hash: block.hash_bytes,
            actions,
        };
        let s = tx.sign(&signer.key)?;
        signer.last_nonce.set(nonce);
        Ok(s)
    }

    /// Sends a signed transaction and waits as asked. A wait that times out at the RPC is
    /// reported as an outcome with no result (status as far as known); query it with
    /// [`NearRpc::tx_status`].
    pub fn send(&self, tx: &SignedTx, wait: WaitUntil) -> Result<TxOutcome> {
        let hash = tx.hash_str();
        match self.call("send_tx", json!({"signed_tx_base64": tx.base64(), "wait_until": wait.as_str()})) {
            Ok(v) => Ok(TxOutcome::parse(&hash, &v)),
            Err(e) if rpc_cause(&e) == Some("TIMEOUT_ERROR") => Ok(TxOutcome { hash, status: "TIMEOUT".into(), result: None, logs: vec![] }),
            Err(e) => Err(e.context(format!("send_tx {hash}"))),
        }
    }

    pub fn tx_status(&self, hash: &str, sender: &str, wait: WaitUntil) -> Result<TxOutcome> {
        match self.call("tx", json!({"tx_hash": hash, "sender_account_id": sender, "wait_until": wait.as_str()})) {
            Ok(v) => Ok(TxOutcome::parse(hash, &v)),
            Err(e) if matches!(rpc_cause(&e), Some("TIMEOUT_ERROR") | Some("UNKNOWN_TRANSACTION")) => {
                Ok(TxOutcome { hash: hash.to_string(), status: rpc_cause(&e).unwrap_or("").into(), result: None, logs: vec![] })
            }
            Err(e) => Err(e),
        }
    }

    /// Signs, sends and waits until executed (all receipts) at optimistic finality.
    pub fn transact(&self, signer: &Signer, receiver: &str, actions: Vec<Action>) -> Result<TxOutcome> {
        let tx = self.sign(signer, receiver, actions)?;
        self.wait_executed(&tx, &signer.account_id)
    }

    /// Sends (or re-sends) `tx` and waits until all its receipts executed.
    pub fn wait_executed(&self, tx: &SignedTx, sender: &str) -> Result<TxOutcome> {
        let mut out = self.send(tx, WaitUntil::Executed)?;
        for _ in 0..6 {
            if out.result.is_some() {
                break;
            }
            out = self.tx_status(&out.hash, sender, WaitUntil::Executed)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_parsing_finds_receipt_failures() {
        let ok = json!({
            "final_execution_status": "EXECUTED",
            "status": {"SuccessValue": "IjEwIg=="},
            "transaction_outcome": {"outcome": {"logs": [], "status": {"SuccessReceiptId": "x"}, "executor_id": "a"}},
            "receipts_outcome": [{"outcome": {"logs": ["EVENT_JSON:{}"], "status": {"SuccessValue": ""}, "executor_id": "b"}}]
        });
        let o = TxOutcome::parse("h", &ok);
        assert_eq!(o.result, Some(Ok(b"\"10\"".to_vec())));
        assert_eq!(o.logs, vec!["EVENT_JSON:{}"]);
        // ft_transfer_call: the top-level status succeeds, but ft_on_transfer failed
        let mut bad = ok.clone();
        bad["receipts_outcome"][0]["outcome"]["status"] = json!({"Failure": {"ActionError": {"kind": "FunctionCallError"}}});
        let o = TxOutcome::parse("h", &bad);
        assert!(o.failure().unwrap().contains("FunctionCallError"));
        assert!(o.ok().is_err());
        // not executed yet
        let none = json!({"final_execution_status": "NONE"});
        assert!(TxOutcome::parse("h", &none).result.is_none());
    }
}
