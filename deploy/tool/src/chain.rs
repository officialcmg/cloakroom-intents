//! Signing accounts, sending transactions and timing them, and the deployment record.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use near_workspaces::network::{Mainnet, Sandbox, Testnet};
use near_workspaces::operations::TransactionStatus;
use near_workspaces::types::SecretKey;
use near_workspaces::{Account, AccountId, Worker};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::Res;
use crate::config::Network;
use crate::rpc::{Rpc, truncate};

/// The near-workspaces worker, used only to sign and send transactions.
pub enum W {
    Sandbox(Worker<Sandbox>),
    Testnet(Worker<Testnet>),
    Mainnet(Worker<Mainnet>),
}

impl W {
    /// A worker for a public network through `rpc_url`.
    pub async fn remote(network: Network, rpc_url: &str) -> Res<Self> {
        Ok(match network {
            Network::Testnet => W::Testnet(near_workspaces::testnet().rpc_addr(rpc_url).await?),
            Network::Mainnet => W::Mainnet(near_workspaces::mainnet().rpc_addr(rpc_url).await?),
            Network::Sandbox => return Err("the sandbox is not a remote network".into()),
        })
    }

    pub fn account(&self, id: &str, sk: SecretKey) -> Res<Account> {
        let id: AccountId = id.parse()?;
        Ok(match self {
            W::Sandbox(w) => Account::from_secret_key(id, sk, w),
            W::Testnet(w) => Account::from_secret_key(id, sk, w),
            W::Mainnet(w) => Account::from_secret_key(id, sk, w),
        })
    }
}

/// A transaction that the network never included (the RPC node dropped it). Nothing
/// happened on chain, so the step can be sent again.
#[derive(Debug)]
pub struct Dropped(pub String);
impl std::fmt::Display for Dropped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Dropped {}

/// sha256 of a file's bytes, base58: the form NEAR shows as an account's `code_hash`.
pub fn code_hash(wasm: &[u8]) -> String {
    bs58::encode(Sha256::digest(wasm)).into_string()
}

/// NEAR shows this code hash for an account without a contract.
pub const NO_CODE: &str = "11111111111111111111111111111111";

/// Storage price on NEAR: 10^19 yoctoNEAR per byte (1 NEAR per 100 kB).
pub const BYTE_COST: u128 = 10_000_000_000_000_000_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TxRec {
    pub step: String,
    pub hash: String,
    pub signer: String,
    pub receiver: String,
    pub block_height: u64,
    pub block_hash: String,
    pub gas_burnt_tgas: f64,
    /// Milliseconds from just before signing to the first status poll that shows the
    /// transaction included in a block.
    pub included_ms: Option<u64>,
    /// ... to the first poll that shows the transaction's block final.
    pub included_final_ms: Option<u64>,
    /// ... to the first poll that shows every receipt executed in final blocks.
    pub final_ms: Option<u64>,
}

pub struct TxResult {
    pub rec: TxRec,
    pub ok: bool,
    pub failure: Option<String>,
    pub receipt_failures: Vec<String>,
    pub logs: Vec<String>,
    pub value: Option<Vec<u8>>,
}

impl TxResult {
    pub fn json(&self) -> Res<Value> {
        let v = self.value.as_ref().ok_or("transaction returned no value")?;
        Ok(serde_json::from_slice(v)?)
    }

    pub fn events(&self, standard: &str, event: &str) -> Vec<Value> {
        self.logs
            .iter()
            .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
            .filter_map(|j| serde_json::from_str::<Value>(j).ok())
            .filter(|e| e["standard"] == standard && e["event"] == event)
            .collect()
    }

    /// Fails unless the transaction and all its receipts succeeded.
    pub fn require_ok(self) -> Res<Self> {
        if !self.ok || !self.receipt_failures.is_empty() {
            return Err(format!(
                "{} ({}) failed: {} {:?}",
                self.rec.step,
                self.rec.hash,
                self.failure.as_deref().unwrap_or(""),
                self.receipt_failures
            )
            .into());
        }
        Ok(self)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ContractRec {
    pub account_id: String,
    pub code_hash: String,
    #[serde(default)]
    pub wasm_bytes: u64,
    #[serde(default)]
    pub deployed_by_this_tool: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EscrowRec {
    pub account_id: String,
    pub code_hash: String,
    pub wasm_bytes: u64,
    pub intents_account: String,
    pub access_keys: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MakerRec {
    pub account_id: String,
    pub public_key: String,
    pub token_balance: String,
    pub storage_funds_yocto: String,
}

/// `deployment.<network>.json`. Public ids, hashes and heights only. No keys.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Deployment {
    pub network: String,
    pub rpc_url: String,
    pub updated_at: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub escrow: Option<EscrowRec>,
    #[serde(default)]
    pub token: Option<ContractRec>,
    #[serde(default)]
    pub intents: Option<ContractRec>,
    #[serde(default)]
    pub maker: Option<MakerRec>,
    #[serde(default)]
    pub transactions: Vec<TxRec>,
}

pub struct Chain {
    pub network: Network,
    pub rpc: Rpc,
    pub w: W,
    pub record: Deployment,
    pub record_path: PathBuf,
    /// Print every transaction with its timings.
    pub verbose: bool,
}

impl Chain {
    pub fn save(&mut self) -> Res<()> {
        self.record.network = self.network.as_str().into();
        self.record.rpc_url = if self.network == Network::Sandbox {
            "local sandbox".into()
        } else {
            self.rpc.url.clone()
        };
        self.record.updated_at =
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let text = serde_json::to_string_pretty(&self.record)? + "\n";
        let tmp = self.record_path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.record_path)?;
        Ok(())
    }

    /// Sends a transaction (the future from `transact_async`) and follows it until all
    /// its receipts are final. Records it in the deployment file when `record` is true.
    pub async fn track<F>(&mut self, step: &str, record: bool, send: F) -> Res<TxResult>
    where
        F: Future<Output = near_workspaces::Result<TransactionStatus>>,
    {
        let start = Instant::now();
        let status = send
            .await
            .map_err(|e| format!("{step}: send failed: {e}"))?;
        let hash = status.hash().to_string();
        let sender = status.sender_id().to_string();
        let (mut included, mut included_final) = (None, None);
        let deadline = start + Duration::from_secs(180);
        let unseen_limit = start + Duration::from_secs(40);
        let v = loop {
            if Instant::now() > deadline {
                return Err(format!("{step}: transaction {hash} not final after 180 s").into());
            }
            let st = self.rpc.tx_status(&hash, &sender).await?;
            if st.is_none() && Instant::now() > unseen_limit {
                return Err(Box::new(Dropped(format!(
                    "{step}: transaction {hash} not seen by the network after 40 s; it was dropped. \
                     Each step checks the chain first, so running the command again is safe."
                ))));
            }
            if let Some(v) = st {
                let st = v["final_execution_status"]
                    .as_str()
                    .unwrap_or("NONE")
                    .to_string();
                let ms = start.elapsed().as_millis() as u64;
                if included.is_none()
                    && matches!(
                        st.as_str(),
                        "INCLUDED"
                            | "EXECUTED_OPTIMISTIC"
                            | "INCLUDED_FINAL"
                            | "EXECUTED"
                            | "FINAL"
                    )
                {
                    included = Some(ms);
                }
                if included_final.is_none()
                    && matches!(st.as_str(), "INCLUDED_FINAL" | "EXECUTED" | "FINAL")
                {
                    included_final = Some(ms);
                }
                if st == "FINAL" {
                    break (v, ms);
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        let (v, final_ms) = v;

        let txo = &v["transaction_outcome"];
        let block_hash = txo["block_hash"].as_str().unwrap_or("").to_string();
        let block_height = self.rpc.block_height(&block_hash).await?;
        let mut gas: u64 = txo["outcome"]["gas_burnt"].as_u64().unwrap_or(0);
        let mut logs = Vec::new();
        let mut receipt_failures = Vec::new();
        for r in v["receipts_outcome"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            gas += r["outcome"]["gas_burnt"].as_u64().unwrap_or(0);
            for l in r["outcome"]["logs"].as_array().cloned().unwrap_or_default() {
                logs.push(l.as_str().unwrap_or("").to_string());
            }
            if let Some(f) = r["outcome"]["status"].get("Failure") {
                receipt_failures.push(format!(
                    "{}: {}",
                    r["outcome"]["executor_id"].as_str().unwrap_or("?"),
                    truncate(&f.to_string(), 400)
                ));
            }
        }
        let (ok, failure, value) = match &v["status"] {
            s if s.get("SuccessValue").is_some() => {
                let b = B64.decode(s["SuccessValue"].as_str().unwrap_or(""))?;
                (true, None, Some(b))
            }
            s if s.get("Failure").is_some() => {
                (false, Some(truncate(&s["Failure"].to_string(), 600)), None)
            }
            s => (false, Some(format!("unexpected status {s}")), None),
        };
        let rec = TxRec {
            step: step.to_string(),
            hash: hash.clone(),
            signer: v["transaction"]["signer_id"]
                .as_str()
                .unwrap_or(&sender)
                .to_string(),
            receiver: v["transaction"]["receiver_id"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            block_height,
            block_hash,
            gas_burnt_tgas: (gas as f64 / 1e12 * 100.0).round() / 100.0,
            included_ms: included,
            included_final_ms: included_final,
            final_ms: Some(final_ms),
        };
        if self.verbose {
            println!(
                "  tx {:<46} {}  block {}  {:>6.2} Tgas  included {:>5.1} s  final {:>5.1} s  {}",
                step,
                hash,
                block_height,
                rec.gas_burnt_tgas,
                included.unwrap_or(0) as f64 / 1000.0,
                final_ms as f64 / 1000.0,
                if ok && receipt_failures.is_empty() {
                    "ok"
                } else {
                    "FAILED"
                }
            );
            for f in &receipt_failures {
                println!("     receipt failure: {f}");
            }
            if let Some(f) = &failure {
                println!("     failure: {f}");
            }
        }
        if record {
            self.record.transactions.push(rec.clone());
            self.save()?;
        }
        Ok(TxResult {
            rec,
            ok,
            failure,
            receipt_failures,
            logs,
            value,
        })
    }
}
