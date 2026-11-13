//! A small NEAR JSON-RPC client for views, transaction status and the testnet faucet.
//!
//! The tool signs and sends transactions with near-workspaces. Everything it reads, it
//! reads here, so views and status work the same on sandbox, testnet and mainnet.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::time::Duration;

use crate::Res;

pub struct Rpc {
    pub url: String,
    http: reqwest::Client,
}

/// An error answer of the RPC node, kept apart from transport errors so callers can
/// treat "unknown account" or "not yet observed" as a state, not a failure.
#[derive(Debug)]
pub struct RpcError {
    pub name: String,
    pub cause: String,
    pub text: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC error {} / {}: {}", self.name, self.cause, self.text)
    }
}
impl std::error::Error for RpcError {}

impl Rpc {
    pub fn new(url: &str) -> Res<Self> {
        let mut b = reqwest::Client::builder().timeout(Duration::from_secs(60));
        if url.contains("127.0.0.1") || url.contains("localhost") {
            b = b.no_proxy();
        }
        Ok(Self {
            url: url.to_string(),
            http: b.build()?,
        })
    }

    /// One JSON-RPC call. Retries transport errors and HTTP 429/5xx a few times.
    pub async fn call(&self, method: &str, params: Value) -> Res<Result<Value, RpcError>> {
        let body = json!({ "jsonrpc": "2.0", "id": "zecret-deploy", "method": method, "params": params });
        let mut last = String::new();
        for attempt in 0..6u64 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
            }
            let resp = match self.http.post(&self.url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = e.to_string();
                    continue;
                }
            };
            let status = resp.status();
            if status.as_u16() == 429 || status.is_server_error() {
                last = format!("HTTP {status}");
                continue;
            }
            let v: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    last = e.to_string();
                    continue;
                }
            };
            if let Some(err) = v.get("error") {
                let name = err["name"].as_str().unwrap_or("").to_string();
                let cause = err["cause"]["name"].as_str().unwrap_or("").to_string();
                // Some nodes report overload as an internal error; retry those.
                if name == "INTERNAL_ERROR" && cause != "UNKNOWN_BLOCK" {
                    last = err.to_string();
                    continue;
                }
                return Ok(Err(RpcError {
                    name,
                    cause,
                    text: err.to_string(),
                }));
            }
            return Ok(Ok(v["result"].clone()));
        }
        Err(format!("RPC {method} failed after retries: {last}").into())
    }

    async fn query(&self, params: Value) -> Res<Result<Value, RpcError>> {
        let r = self.call("query", params).await?;
        // Older nodes put view errors in result.error instead of error.
        if let Ok(v) = &r
            && let Some(e) = v.get("error").and_then(|e| e.as_str())
        {
            return Ok(Err(RpcError {
                name: "HANDLER_ERROR".into(),
                cause: "VIEW_ERROR".into(),
                text: e.to_string(),
            }));
        }
        Ok(r)
    }

    /// `view_account` at final finality, or `None` if the account does not exist.
    pub async fn account(&self, id: &str) -> Res<Option<Value>> {
        match self
            .query(json!({ "request_type": "view_account", "finality": "final", "account_id": id }))
            .await?
        {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.cause == "UNKNOWN_ACCOUNT" || e.text.contains("does not exist") => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The account's balance in yoctoNEAR (0 if it does not exist).
    pub async fn balance(&self, id: &str) -> Res<u128> {
        Ok(match self.account(id).await? {
            Some(v) => v["amount"].as_str().unwrap_or("0").parse()?,
            None => 0,
        })
    }

    /// The account's code hash (base58), `11111111111111111111111111111111` if no code.
    pub async fn code_hash(&self, id: &str) -> Res<Option<String>> {
        Ok(self
            .account(id)
            .await?
            .map(|v| v["code_hash"].as_str().unwrap_or("").to_string()))
    }

    /// The public keys of the account's access keys, at final finality.
    pub async fn access_keys(&self, id: &str) -> Res<Vec<String>> {
        let v = self
            .query(json!({ "request_type": "view_access_key_list", "finality": "final", "account_id": id }))
            .await?
            .map_err(|e| -> crate::Err { e.into() })?;
        Ok(v["keys"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|k| k["public_key"].as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A view call at final finality. `Ok(Err(..))` if the contract panicked or the method
    /// is missing.
    pub async fn view(&self, contract: &str, method: &str, args: &Value) -> Res<Result<Value, RpcError>> {
        let r = self
            .query(json!({
                "request_type": "call_function",
                "finality": "final",
                "account_id": contract,
                "method_name": method,
                "args_base64": B64.encode(args.to_string()),
            }))
            .await?;
        Ok(match r {
            Ok(v) => {
                let bytes: Vec<u8> = v["result"]
                    .as_array()
                    .map(|a| a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect())
                    .unwrap_or_default();
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(j) => Ok(j),
                    Err(e) => Err(RpcError {
                        name: "PARSE".into(),
                        cause: "JSON".into(),
                        text: format!("{method}: {e}"),
                    }),
                }
            }
            Err(e) => Err(e),
        })
    }

    /// A view that must succeed.
    pub async fn view_ok(&self, contract: &str, method: &str, args: &Value) -> Res<Value> {
        self.view(contract, method, args)
            .await?
            .map_err(|e| format!("view {contract}.{method}: {e}").into())
    }

    /// A block by hash, or the latest block at a finality ("final", "optimistic").
    pub async fn block(&self, hash: Option<&str>, finality: &str) -> Res<Value> {
        let p = match hash {
            Some(h) => json!({ "block_id": h }),
            None => json!({ "finality": finality }),
        };
        self.call("block", p)
            .await?
            .map_err(|e| format!("block: {e}").into())
    }

    /// Height of the block with this hash.
    pub async fn block_height(&self, hash: &str) -> Res<u64> {
        let b = self.block(Some(hash), "final").await?;
        b["header"]["height"]
            .as_u64()
            .ok_or_else(|| "block without height".into())
    }

    /// Timestamp of the latest final block, in seconds.
    pub async fn final_timestamp_s(&self) -> Res<u64> {
        let b = self.block(None, "final").await?;
        let ns: u64 = b["header"]["timestamp_nanosec"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| b["header"]["timestamp"].as_u64())
            .ok_or("block without timestamp")?;
        Ok(ns / 1_000_000_000)
    }

    /// Transaction status without waiting (`wait_until: NONE`). `None` while the node has
    /// not seen the transaction.
    pub async fn tx_status(&self, hash: &str, sender: &str) -> Res<Option<Value>> {
        match self
            .call(
                "tx",
                json!({ "tx_hash": hash, "sender_account_id": sender, "wait_until": "NONE" }),
            )
            .await?
        {
            Ok(v) => Ok(Some(v)),
            Err(e)
                if e.cause == "UNKNOWN_TRANSACTION"
                    || e.cause == "TIMEOUT_ERROR"
                    || e.name == "TIMEOUT_ERROR"
                    || e.text.contains("doesn't exist")
                    || e.text.contains("NotObserved")
                    || e.text.contains("Pending") =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Creates a top-level `.testnet` account through the testnet faucet (the NEAR helper
    /// service). The helper funds it with its standard amount.
    pub async fn faucet_create(&self, helper_url: &str, account_id: &str, public_key: &str) -> Res<()> {
        let url = format!("{}/account", helper_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .json(&json!({ "newAccountId": account_id, "newAccountPublicKey": public_key }))
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("faucet answered HTTP {status}: {}", truncate(&text, 300)).into());
        }
        // The helper answers with the creating transaction; wait until the account is
        // visible at final finality.
        for _ in 0..60 {
            if self.account(account_id).await?.is_some() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(format!("faucet: account {account_id} did not appear").into())
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut end = n;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
