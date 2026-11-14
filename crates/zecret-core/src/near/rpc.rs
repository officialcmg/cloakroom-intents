//! NEAR JSON-RPC over the core's `Transport`: blocks, view calls, accounts, access keys,
//! and sending transactions.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

use super::tx::{Action, NearKey, SignedTx, Transaction};
use crate::{config::RpcUrls, net::SharedTransport};

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
    /// base58 sha256 of the deployed code; `11111111111111111111111111111111` if none.
    pub code_hash: String,
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
/// It comes from a node that answered: the chain said no. It is never retried on another
/// endpoint.
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

/// No endpoint gave an answer: each was rate-limited, down, too slow or behind, or is
/// backing off after such a failure. Nothing was learned about the chain.
#[derive(Debug, Clone)]
pub struct Unavailable {
    /// One line per endpoint.
    pub detail: String,
    /// Wall-clock time (ms) at which the first endpoint ends its back-off.
    pub retry_at_ms: u64,
    /// True when no request was sent because every endpoint is backing off.
    pub skipped: bool,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NEAR RPC unavailable: {}", self.detail)
    }
}

impl std::error::Error for Unavailable {}

/// The `Unavailable` inside an `anyhow` error, if the error is one.
pub fn unavailable(e: &anyhow::Error) -> Option<&Unavailable> {
    e.chain().find_map(|c| c.downcast_ref::<Unavailable>())
}

/// An endpoint URL without what may be a secret: user and password, query string
/// (`?apiKey=`) and fragment. For logs and `/health`.
pub fn public_url(url: &str) -> String {
    let cut = url.find(['?', '#']).map(|i| &url[..i]).unwrap_or(url);
    match (cut.find("://"), cut.rfind('@')) {
        (Some(s), Some(at)) if at > s => format!("{}{}", &cut[..s + 3], &cut[at + 1..]),
        _ => cut.to_string(),
    }
}

/// nearcore 2.14.1 answers `view_access_key` for a missing key with a result that holds
/// an `error` string ("access key ... does not exist while viewing"), not a typed error.
fn missing(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains("does not exist while viewing")
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

/// How long a failing endpoint is left alone. After the n-th failure in a row it backs
/// off `base_ms · 2^(n-1)`, at most `max_ms`, with ±20 % jitter. A `Retry-After` header
/// (seconds) lengthens the back-off, up to `retry_after_max_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub base_ms: u64,
    pub max_ms: u64,
    pub retry_after_max_ms: u64,
}

impl Default for Backoff {
    fn default() -> Backoff {
        Backoff { base_ms: 2_000, max_ms: 60_000, retry_after_max_ms: 300_000 }
    }
}

impl Backoff {
    /// The back-off after `fails` failures in a row, before jitter.
    pub fn delay_ms(&self, fails: u32) -> u64 {
        let shift = fails.saturating_sub(1).min(20);
        self.base_ms.saturating_mul(1u64 << shift).min(self.max_ms)
    }
}

/// What the client knows about one endpoint.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct EndpointHealth {
    pub url: String,
    /// Failures in a row (0 after an answer).
    pub fails: u32,
    /// No request goes to this endpoint before this wall-clock time (ms), except a
    /// deadline-critical one when every other endpoint failed too.
    pub backoff_until_ms: u64,
    pub last_ok_ms: Option<u64>,
    pub last_error: Option<String>,
    pub requests: u64,
    pub rate_limited: u64,
}

/// How long a final block read stays valid for later reads (ms of the transport clock).
/// One tick decides every swap at the same final block, so a tick with several swaps
/// reads it once.
pub const FINAL_CACHE_MS: u64 = 2_000;

/// Why an attempt at one endpoint gave no answer.
enum Fault {
    /// HTTP 429, or a JSON-RPC error that says so. Seconds from `Retry-After`, if sent.
    Limited(Option<u64>),
    /// No response, HTTP 5xx, or a body that is not a JSON-RPC answer.
    Down,
    /// The node is behind (`UNKNOWN_BLOCK` and the like): try the next one, no back-off.
    Lagging,
}

/// The result of one attempt.
enum Answer {
    Ok(Value),
    /// The node answered with an error about the request or the chain.
    Chain(anyhow::Error),
    Fault(Fault, String),
}

/// Causes that mean "this node does not have that data yet", not "the chain says no".
const LAGGING_CAUSES: &[&str] = &["UNKNOWN_BLOCK", "NOT_SYNCED_YET", "NO_SYNCED_BLOCKS", "UNAVAILABLE_SHARD", "UNKNOWN_CHUNK"];

fn limit_text(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    s.contains("rate limit") || s.contains("limit exceeded") || s.contains("too many requests")
}

fn snippet(b: &[u8]) -> String {
    String::from_utf8_lossy(&b[..b.len().min(200)]).replace('\n', " ")
}

fn classify(method: &str, r: Result<crate::net::Response>) -> Answer {
    let resp = match r {
        Ok(r) => r,
        Err(e) => return Answer::Fault(Fault::Down, format!("no response ({e:#})")),
    };
    let retry_after = resp.header("retry-after").and_then(|v| v.trim().parse::<u64>().ok());
    if resp.status == 429 {
        return Answer::Fault(Fault::Limited(retry_after), format!("HTTP 429 {}", snippet(&resp.body)));
    }
    let v: Value = match serde_json::from_slice(&resp.body) {
        Ok(v) => v,
        Err(_) => return Answer::Fault(Fault::Down, format!("HTTP {} {}", resp.status, snippet(&resp.body))),
    };
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        let code = e["code"].as_i64();
        let text = format!("{} {}", e["message"].as_str().unwrap_or(""), e["data"]);
        if code == Some(-429) || limit_text(&text) {
            return Answer::Fault(Fault::Limited(retry_after), format!("rate limited: {}", snippet(e.to_string().as_bytes())));
        }
        let name = e["name"].as_str().unwrap_or("").to_string();
        let cause = e["cause"]["name"].as_str().unwrap_or("").to_string();
        if LAGGING_CAUSES.contains(&cause.as_str()) {
            return Answer::Fault(Fault::Lagging, format!("{name} / {cause}"));
        }
        if name.is_empty() || name == "INTERNAL_ERROR" || resp.status >= 500 {
            // Not a NEAR node's answer about the request (a gateway or a broken node).
            return Answer::Fault(Fault::Down, format!("HTTP {} error {}", resp.status, snippet(e.to_string().as_bytes())));
        }
        return Answer::Chain(
            RpcError {
                name,
                cause,
                detail: e.get("data").filter(|d| !d.is_null()).or_else(|| e.get("cause").and_then(|c| c.get("info")).filter(|i| i.as_object().map(|o| !o.is_empty()).unwrap_or(true))).cloned().unwrap_or_else(|| e.clone()),
            }
            .into(),
        );
    }
    if resp.status != 200 || v.get("result").is_none() {
        return Answer::Fault(Fault::Down, format!("HTTP {} without a result: {}", resp.status, snippet(&resp.body)));
    }
    // A view call that panicked comes back as a result with an `error` string.
    if let Some(err) = v["result"].get("error").and_then(|e| e.as_str()) {
        return Answer::Chain(anyhow!("NEAR RPC {method}: {err}"));
    }
    Answer::Ok(v["result"].clone())
}

/// A NEAR JSON-RPC client over an ordered list of endpoints.
///
/// Each request goes to the first endpoint that is not backing off. On a rate limit, a
/// server error, no response or a node that is behind, the same request goes to the
/// next endpoint at once; the failed one backs off ([`Backoff`]). A request never goes
/// to the same endpoint twice, and the client never sleeps: when every endpoint is
/// backing off, a read fails at once with [`Unavailable`] (no request sent) and the
/// engine tries again on a later tick.
///
/// Inside [`NearRpc::critical`] (the sends that protect funds and the reads they need)
/// the client also tries the endpoints that are backing off, after the others, so a
/// refund is never held back by a back-off timer while its window is open.
pub struct NearRpc {
    t: SharedTransport,
    /// The first endpoint (kept for callers that print or pass on one URL).
    pub url: String,
    eps: RefCell<Vec<EndpointHealth>>,
    backoff: Backoff,
    critical: Cell<bool>,
    /// The engine's time (ms), for transports without a clock.
    tick_ms: Cell<u64>,
    final_cache: RefCell<Option<(u64, BlockInfo)>>,
    counts: RefCell<BTreeMap<String, u64>>,
    total: Cell<u64>,
}

impl NearRpc {
    /// A client of one endpoint.
    pub fn new(t: SharedTransport, url: &str) -> NearRpc {
        NearRpc::with_urls(t, &RpcUrls::from(url))
    }

    /// A client of an ordered list of endpoints.
    pub fn with_urls(t: SharedTransport, urls: &RpcUrls) -> NearRpc {
        NearRpc {
            t,
            url: urls.primary().to_string(),
            eps: RefCell::new(urls.as_slice().iter().map(|u| EndpointHealth { url: u.clone(), ..Default::default() }).collect()),
            backoff: Backoff::default(),
            critical: Cell::new(false),
            tick_ms: Cell::new(0),
            final_cache: RefCell::new(None),
            counts: RefCell::new(BTreeMap::new()),
            total: Cell::new(0),
        }
    }

    pub fn set_backoff(&mut self, b: Backoff) {
        self.backoff = b;
    }

    /// The engine's current time (seconds). Used only when the transport has no clock.
    pub fn set_time(&self, now_secs: u64) {
        self.tick_ms.set(now_secs.saturating_mul(1000));
    }

    pub fn now_ms(&self) -> u64 {
        self.t.now_ms().unwrap_or_else(|| self.tick_ms.get())
    }

    /// Runs `f` as deadline-critical work: its requests also try endpoints that are
    /// backing off (after the others).
    pub fn critical<T>(&self, f: impl FnOnce() -> T) -> T {
        let before = self.critical.replace(true);
        let r = f();
        self.critical.set(before);
        r
    }

    /// Sets the deadline-critical mode (see [`NearRpc::critical`]); returns the old value.
    pub fn set_critical(&self, on: bool) -> bool {
        self.critical.replace(on)
    }

    /// Requests sent so far (all endpoints).
    pub fn requests(&self) -> u64 {
        self.total.get()
    }

    /// Requests sent so far, by JSON-RPC method (`query` is split by request type).
    pub fn requests_by_method(&self) -> BTreeMap<String, u64> {
        self.counts.borrow().clone()
    }

    pub fn health(&self) -> Vec<EndpointHealth> {
        self.eps.borrow().clone()
    }

    /// The last time (ms) any endpoint answered.
    pub fn last_ok_ms(&self) -> Option<u64> {
        self.eps.borrow().iter().filter_map(|e| e.last_ok_ms).max()
    }

    /// Forgets the cached final block (a test, or after a long pause).
    pub fn forget_final(&self) {
        *self.final_cache.borrow_mut() = None;
    }

    /// The order in which this request tries the endpoints.
    fn order(&self, now: u64) -> Vec<usize> {
        let eps = self.eps.borrow();
        let mut ready: Vec<usize> = (0..eps.len()).filter(|&i| eps[i].backoff_until_ms <= now).collect();
        if self.critical.get() {
            let mut waiting: Vec<usize> = (0..eps.len()).filter(|&i| eps[i].backoff_until_ms > now).collect();
            waiting.sort_by_key(|&i| eps[i].backoff_until_ms);
            ready.extend(waiting);
        }
        ready
    }

    fn jitter(&self, ms: u64) -> u64 {
        let r = u16::from_le_bytes(crate::random_bytes::<2>()) as u64; // 0..65535
        // 0.8 .. 1.2
        ms.saturating_mul(80 + r * 40 / 65_535) / 100
    }

    fn mark_ok(&self, i: usize) {
        let now = self.now_ms();
        let mut eps = self.eps.borrow_mut();
        eps[i].fails = 0;
        eps[i].backoff_until_ms = 0;
        eps[i].last_ok_ms = Some(now);
    }

    fn mark_fault(&self, i: usize, f: &Fault, msg: &str) {
        let now = self.now_ms();
        let mut eps = self.eps.borrow_mut();
        let e = &mut eps[i];
        e.last_error = Some(msg.to_string());
        match f {
            Fault::Lagging => {}
            Fault::Down | Fault::Limited(None) => {
                e.fails = e.fails.saturating_add(1);
                e.backoff_until_ms = now + self.jitter(self.backoff.delay_ms(e.fails));
            }
            Fault::Limited(Some(secs)) => {
                e.fails = e.fails.saturating_add(1);
                let asked = secs.saturating_mul(1000).min(self.backoff.retry_after_max_ms);
                e.backoff_until_ms = now + self.jitter(self.backoff.delay_ms(e.fails)).max(asked);
            }
        }
        if matches!(f, Fault::Limited(_)) {
            e.rate_limited += 1;
        }
    }

    fn count(&self, i: usize, what: &str) {
        self.total.set(self.total.get() + 1);
        *self.counts.borrow_mut().entry(what.to_string()).or_insert(0) += 1;
        self.eps.borrow_mut()[i].requests += 1;
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let what = match params.get("request_type").and_then(|r| r.as_str()) {
            Some(rt) => format!("query/{rt}"),
            None => method.to_string(),
        };
        let req = json!({"jsonrpc": "2.0", "id": "zecret", "method": method, "params": params}).to_string();
        let order = self.order(self.now_ms());
        if order.is_empty() {
            let eps = self.eps.borrow();
            let now = self.now_ms();
            let retry_at_ms = eps.iter().map(|e| e.backoff_until_ms).min().unwrap_or(now);
            let detail = eps.iter().map(|e| format!("{} backs off for {}s after: {}", e.url, e.backoff_until_ms.saturating_sub(now).div_ceil(1000), e.last_error.as_deref().unwrap_or("?"))).collect::<Vec<_>>().join("; ");
            return Err(anyhow::Error::new(Unavailable { detail, retry_at_ms, skipped: true }).context(format!("NEAR RPC {method}")));
        }
        let mut errs = vec![];
        for i in order {
            let url = self.eps.borrow()[i].url.clone();
            self.count(i, &what);
            match classify(method, self.t.post(&url, &[("content-type", "application/json")], req.as_bytes())) {
                Answer::Ok(v) => {
                    self.mark_ok(i);
                    return Ok(v);
                }
                Answer::Chain(e) => {
                    self.mark_ok(i);
                    return Err(e);
                }
                Answer::Fault(f, msg) => {
                    self.mark_fault(i, &f, &msg);
                    errs.push(format!("{url}: {msg}"));
                }
            }
        }
        let retry_at_ms = self.eps.borrow().iter().map(|e| e.backoff_until_ms).min().unwrap_or(0);
        Err(anyhow::Error::new(Unavailable { detail: errs.join("; "), retry_at_ms, skipped: false }).context(format!("NEAR RPC {method}")))
    }

    pub fn block(&self, at: &At) -> Result<BlockInfo> {
        if *at == At::Final
            && let Some((t, b)) = self.final_cache.borrow().as_ref()
        {
            // Without a transport clock, only within the same tick.
            let fresh = match self.t.now_ms() {
                Some(n) => n.saturating_sub(*t) <= FINAL_CACHE_MS,
                None => *t == self.tick_ms.get(),
            };
            if fresh {
                return Ok(b.clone());
            }
        }
        let b = self.read_block(at)?;
        if *at == At::Final {
            *self.final_cache.borrow_mut() = Some((self.now_ms(), b.clone()));
        }
        Ok(b)
    }

    fn read_block(&self, at: &At) -> Result<BlockInfo> {
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
                code_hash: v["code_hash"].as_str().unwrap_or_default().to_string(),
            })),
            Err(e) if rpc_cause(&e) == Some("UNKNOWN_ACCOUNT") || missing(&e) => Ok(None),
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
            Err(e) if matches!(rpc_cause(&e), Some("UNKNOWN_ACCOUNT") | Some("UNKNOWN_ACCESS_KEY")) || missing(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The public keys of every access key of `account` (`view_access_key_list`).
    pub fn access_keys(&self, account: &str, at: &At) -> Result<Vec<String>> {
        let mut p = at.params();
        p["request_type"] = json!("view_access_key_list");
        p["account_id"] = json!(account);
        let v = self.call("query", p)?;
        Ok(v["keys"].as_array().ok_or_else(|| anyhow!("view_access_key_list: no keys field"))?.iter().filter_map(|k| k["public_key"].as_str().map(str::to_string)).collect())
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
    use crate::net::{Response, Transport};
    use std::{collections::HashMap, rc::Rc};

    /// What a scripted endpoint answers.
    #[derive(Clone)]
    pub(crate) enum Reply {
        /// A JSON-RPC result.
        Ok(Value),
        /// HTTP 429, optionally with Retry-After seconds.
        Http429(Option<u64>),
        /// HTTP 200 with fastnear's JSON-RPC error -429.
        Json429,
        /// HTTP 502 with an HTML body.
        Http502,
        /// No HTTP response (connection refused, timeout).
        NoAnswer,
        /// A NEAR error with this cause name.
        Cause(&'static str),
    }

    /// A transport whose endpoints answer from scripts, with a clock tests move.
    pub(crate) struct Scripted {
        pub clock: Cell<u64>,
        /// url -> reply for every request (the last entry repeats).
        pub replies: RefCell<HashMap<String, Vec<Reply>>>,
        /// (url, method) of every request, in order.
        pub log: RefCell<Vec<(String, String)>>,
    }

    impl Scripted {
        pub(crate) fn new() -> Rc<Scripted> {
            Rc::new(Scripted { clock: Cell::new(1_000_000), replies: RefCell::new(HashMap::new()), log: RefCell::new(vec![]) })
        }
        pub(crate) fn set(&self, url: &str, r: Vec<Reply>) {
            self.replies.borrow_mut().insert(url.to_string(), r);
        }
        pub(crate) fn sent_to(&self, url: &str) -> usize {
            self.log.borrow().iter().filter(|(u, _)| u == url).count()
        }
    }

    impl Transport for Scripted {
        fn post(&self, url: &str, _: &[(&str, &str)], body: &[u8]) -> Result<Response> {
            let req: Value = serde_json::from_slice(body).unwrap();
            self.log.borrow_mut().push((url.to_string(), req["method"].as_str().unwrap().to_string()));
            let mut all = self.replies.borrow_mut();
            let list = all.get_mut(url).expect("unscripted url");
            let r = if list.len() > 1 { list.remove(0) } else { list[0].clone() };
            let ok = |v: Value| Ok(Response { status: 200, body: v.to_string().into_bytes(), headers: vec![] });
            match r {
                Reply::Ok(v) => ok(json!({"jsonrpc": "2.0", "id": "zecret", "result": v})),
                Reply::Http429(ra) => Ok(Response { status: 429, body: b"Too Many Requests".to_vec(), headers: ra.map(|s| vec![("retry-after".to_string(), s.to_string())]).unwrap_or_default() }),
                Reply::Json429 => ok(json!({"jsonrpc": "2.0", "id": "zecret", "error": {"code": -429, "message": "Rate limits exceeded. For higher rate limits get a subscription at https://fastnear.com"}})),
                Reply::Http502 => Ok(Response { status: 502, body: b"<html>Bad gateway</html>".to_vec(), headers: vec![] }),
                Reply::NoAnswer => Err(anyhow!("connection refused")),
                Reply::Cause(c) => ok(json!({"jsonrpc": "2.0", "id": "zecret", "error": {"name": "HANDLER_ERROR", "cause": {"name": c, "info": {}}, "code": -32000, "message": "Server error"}})),
            }
        }
        fn now_ms(&self) -> Option<u64> {
            Some(self.clock.get())
        }
    }

    pub(crate) fn block_json(height: u64, secs: u64) -> Value {
        json!({"header": {"height": height, "hash": bs58::encode([height as u8; 32]).into_string(), "timestamp": secs * 1_000_000_000}})
    }

    const A: &str = "https://a.example";
    const B: &str = "https://b.example";
    const C: &str = "https://c.example";

    fn client(t: &Rc<Scripted>, urls: &[&str]) -> NearRpc {
        NearRpc::with_urls(t.clone(), &RpcUrls::new(urls.iter().map(|u| u.to_string()).collect()).unwrap())
    }

    fn backoff_of(rpc: &NearRpc, url: &str, now: u64) -> u64 {
        rpc.health().into_iter().find(|e| e.url == url).unwrap().backoff_until_ms.saturating_sub(now)
    }

    #[test]
    fn a_429_without_retry_after_fails_over_and_backs_off_the_first() {
        let t = Scripted::new();
        t.set(A, vec![Reply::Http429(None)]);
        t.set(B, vec![Reply::Ok(json!({"x": 1}))]);
        let rpc = client(&t, &[A, B]);
        assert_eq!(rpc.call("status", json!([])).unwrap(), json!({"x": 1}));
        assert_eq!(*t.log.borrow(), vec![(A.to_string(), "status".to_string()), (B.to_string(), "status".to_string())]);
        let now = t.clock.get();
        let b = backoff_of(&rpc, A, now);
        assert!((1_600..=2_400).contains(&b), "{b}");
        // While A backs off, a read goes to B only.
        rpc.call("status", json!([])).unwrap();
        assert_eq!(t.sent_to(A), 1);
        assert_eq!(t.sent_to(B), 2);
        // After the back-off, A is first again.
        t.set(A, vec![Reply::Ok(json!({"x": 2}))]);
        t.clock.set(now + 2_500);
        assert_eq!(rpc.call("status", json!([])).unwrap(), json!({"x": 2}));
        assert_eq!(t.sent_to(A), 2);
        assert_eq!(rpc.health()[0].fails, 0);
        assert_eq!(rpc.health()[0].rate_limited, 1);
    }

    #[test]
    fn retry_after_and_fastnears_json_429_are_honoured() {
        let t = Scripted::new();
        t.set(A, vec![Reply::Http429(Some(30))]);
        t.set(B, vec![Reply::Json429]);
        t.set(C, vec![Reply::Ok(json!(7))]);
        let rpc = client(&t, &[A, B, C]);
        assert_eq!(rpc.call("status", json!([])).unwrap(), json!(7));
        let now = t.clock.get();
        assert!(backoff_of(&rpc, A, now) >= 30_000, "Retry-After: 30 is honoured");
        assert!(backoff_of(&rpc, A, now) <= 36_000);
        let b = backoff_of(&rpc, B, now);
        assert!((1_600..=2_400).contains(&b), "fastnear's JSON -429 counts as a rate limit: {b}");
        // A huge Retry-After is capped.
        let t = Scripted::new();
        t.set(A, vec![Reply::Http429(Some(86_400))]);
        t.set(B, vec![Reply::Ok(json!(1))]);
        let rpc = client(&t, &[A, B]);
        rpc.call("status", json!([])).unwrap();
        assert!(backoff_of(&rpc, A, t.clock.get()) <= Backoff::default().retry_after_max_ms);
    }

    #[test]
    fn back_off_doubles_up_to_its_bound_with_jitter() {
        let b = Backoff::default();
        assert_eq!((1..=7).map(|n| b.delay_ms(n)).collect::<Vec<_>>(), vec![2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000]);
        assert_eq!(b.delay_ms(1_000), 60_000);
        let t = Scripted::new();
        t.set(A, vec![Reply::Http502]);
        let rpc = client(&t, &[A]);
        for n in 1..=8u32 {
            let now = t.clock.get();
            assert!(rpc.call("status", json!([])).is_err());
            let d = backoff_of(&rpc, A, now);
            let want = b.delay_ms(n);
            assert!(d >= want * 8 / 10 && d <= want * 12 / 10, "failure {n}: {d} not within 20% of {want}");
            // Reads in the back-off send nothing.
            let sent = t.sent_to(A);
            let e = rpc.call("status", json!([])).unwrap_err();
            assert!(unavailable(&e).unwrap().skipped);
            assert_eq!(t.sent_to(A), sent, "no request while backing off");
            t.clock.set(now + d + 1);
        }
    }

    #[test]
    fn every_endpoint_down_gives_unavailable_with_the_retry_time() {
        let t = Scripted::new();
        t.set(A, vec![Reply::NoAnswer]);
        t.set(B, vec![Reply::Http502]);
        let rpc = client(&t, &[A, B]);
        let e = rpc.call("block", json!({"finality": "final"})).unwrap_err();
        let u = unavailable(&e).expect("Unavailable");
        assert!(!u.skipped);
        assert!(u.detail.contains("a.example") && u.detail.contains("b.example"), "{}", u.detail);
        assert!(u.retry_at_ms > t.clock.get());
        assert_eq!(t.log.borrow().len(), 2, "one request per endpoint, no retry loop");
    }

    #[test]
    fn critical_requests_also_try_endpoints_that_back_off() {
        let t = Scripted::new();
        t.set(A, vec![Reply::Json429]);
        t.set(B, vec![Reply::Json429]);
        let rpc = client(&t, &[A, B]);
        assert!(rpc.call("status", json!([])).is_err());
        assert_eq!(t.log.borrow().len(), 2);
        // A read now sends nothing; a critical send tries both, B (shorter back-off or
        // listed later) in the order their back-off ends.
        assert!(unavailable(&rpc.call("status", json!([])).unwrap_err()).unwrap().skipped);
        t.set(B, vec![Reply::Ok(json!("sent"))]);
        let r = rpc.critical(|| rpc.call("send_tx", json!({})));
        assert_eq!(r.unwrap(), json!("sent"));
        assert!(t.log.borrow().len() <= 4);
        assert_eq!(t.log.borrow().last().unwrap().0, B);
        assert!(!rpc.set_critical(false), "critical mode ends with the closure");
    }

    #[test]
    fn a_chain_error_is_not_retried_and_a_lagging_node_is_skipped() {
        let t = Scripted::new();
        t.set(A, vec![Reply::Cause("UNKNOWN_ACCOUNT")]);
        t.set(B, vec![Reply::Ok(json!(1))]);
        let rpc = client(&t, &[A, B]);
        let e = rpc.call("query", json!({"request_type": "view_account"})).unwrap_err();
        assert_eq!(rpc_cause(&e), Some("UNKNOWN_ACCOUNT"));
        assert_eq!(t.sent_to(B), 0, "the chain's answer is final");
        assert_eq!(rpc.health()[0].backoff_until_ms, 0);
        // A node that does not know the block: next endpoint, no back-off for it.
        t.set(A, vec![Reply::Cause("UNKNOWN_BLOCK")]);
        assert_eq!(rpc.call("query", json!({"request_type": "call_function"})).unwrap(), json!(1));
        assert_eq!(rpc.health()[0].backoff_until_ms, 0);
        assert_eq!(rpc.requests_by_method()["query/call_function"], 2);
    }

    #[test]
    fn one_final_block_read_serves_a_tick() {
        let t = Scripted::new();
        t.set(A, vec![Reply::Ok(block_json(10, 500))]);
        let rpc = client(&t, &[A]);
        assert_eq!(rpc.block(&At::Final).unwrap().height, 10);
        assert_eq!(rpc.block(&At::Final).unwrap().height, 10);
        assert_eq!(rpc.requests(), 1);
        t.clock.set(t.clock.get() + FINAL_CACHE_MS + 1);
        rpc.block(&At::Final).unwrap();
        assert_eq!(rpc.requests(), 2);
        // Other block reads are not cached.
        rpc.block(&At::Optimistic).unwrap();
        rpc.block(&At::Optimistic).unwrap();
        assert_eq!(rpc.requests(), 4);
    }

    #[test]
    fn urls_lose_their_secrets_in_reports() {
        assert_eq!(public_url("https://rpc.example/x?apiKey=SECRET"), "https://rpc.example/x");
        assert_eq!(public_url("https://user:pw@rpc.example/"), "https://rpc.example/");
        assert_eq!(public_url("https://rpc.example#frag"), "https://rpc.example");
        assert_eq!(public_url("https://rpc.testnet.fastnear.com"), "https://rpc.testnet.fastnear.com");
    }

    #[test]
    fn endpoint_lists_read_one_url_or_many() {
        let one: RpcUrls = serde_json::from_value(json!("https://a")).unwrap();
        assert_eq!(one.as_slice(), ["https://a"]);
        assert_eq!(serde_json::to_value(&one).unwrap(), json!("https://a"));
        let two: RpcUrls = serde_json::from_value(json!(["https://a", "https://b"])).unwrap();
        assert_eq!(two.primary(), "https://a");
        assert_eq!(serde_json::to_value(&two).unwrap(), json!(["https://a", "https://b"]));
        assert!(serde_json::from_value::<RpcUrls>(json!([])).is_err());
        assert!(serde_json::from_value::<RpcUrls>(json!(["https://a", "https://a"])).is_err());
        assert!(serde_json::from_value::<RpcUrls>(json!("")).is_err());
    }

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
