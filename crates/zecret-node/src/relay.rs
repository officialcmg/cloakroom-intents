//! The maker's relay for the direct exit (local network and testnet).
//!
//! On networks where 1Click cannot execute the exit, the taker's browser signs one
//! Verifier intent (an `ft_withdraw` to a NEAR account the user named) and someone must
//! send it with `execute_intents`, which needs 40 Tgas attached (35 fails). NEAR accepts a
//! transaction only from a signer that holds its attached gas at 10^9 yoctoNEAR per gas:
//! about 0.041 NEAR here. What a 0.03 NEAR stipend leaves after commit, claim and
//! withdraw (0.0272 NEAR) cannot pay it; what the 0.06 NEAR stipend leaves (0.0572 NEAR)
//! can (`crates/zecret-wasm/tests/sandbox.rs`). The engine still hands the exit to this
//! relay, so the maker pays its gas.
//!
//! `POST /execute-intents` with `{"signed": [<one signed payload>]}`. Answers carry
//! `Access-Control-Allow-Origin: *`. The relay sends a payload only when all of these hold
//! (security review 3, F3-01):
//!
//! - the payload's inner message has exactly one intent, an `ft_withdraw` of the maker's
//!   token (`token`) on this maker's Verifier, with no `msg`;
//! - its signer is the taker account of one of this maker's swaps (read from the maker's
//!   store) whose record shows a payout: stage `Sweeping` or `Swept`, or a recorded
//!   `withdraw` for the taker (`taker_withdraw_at`). A swap in `Opened` or `Ready` gets
//!   "try again later" (503): the maker may not have seen the claim yet;
//! - the amount is at most what the swap paid (`price`);
//! - fewer than [`MAX_RELAYS_PER_SWAP`] transactions were sent for this swap (counted in
//!   the store under `relay/<swap id>`, written before the transaction is sent). The same
//!   signed payload is never sent twice;
//! - `simulate_intents` accepts it (a free view call).
//!
//! The signed payload moves the taker's funds as the taker signed; the relay cannot change
//! it.
//!
//! Serving (F3-02): each request must arrive within [`REQUEST_DEADLINE`] and
//! [`MAX_REQUEST_BYTES`] (one line at most [`MAX_LINE_BYTES`]); [`WORKERS`] threads serve
//! connections; while all are busy, or a source already has
//! [`MAX_CONNECTIONS_PER_SOURCE`] open, a new connection gets 503 at once. One source may
//! ask for at most [`MAX_RELAYS_PER_SOURCE_PER_MINUTE`] relays a minute. A panic in a worker
//! ends only that request. The relay runs in its own threads of `zecret maker` and shares
//! no lock with the maker loop.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Shutdown, TcpListener, TcpStream},
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    rc::Rc,
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};
use zecret_core::{
    maker::PREFIX_SWAP,
    near::{tx::Action, At, NearKey, NearRpc, Signer, TGAS},
    net::SharedTransport,
    store::{get_json, put_json, Store},
};

use crate::io::{FileStore, Http};

/// Gas attached to `execute_intents` with one `ft_withdraw`: the call fails with
/// "Exceeded the prepaid gas" at 35 Tgas and works at 40 Tgas and above, costing about
/// 0.001 NEAR (sandbox, live `intents.near` and `wrap.near` code; see
/// `crates/zecret-wasm/tests/sandbox.rs`). Unused gas is refunded.
pub const EXECUTE_GAS: u64 = 50 * TGAS;

/// Worker threads that read requests and relay them.
pub const WORKERS: usize = 4;
/// The whole request (request line, headers and body) must arrive within this time.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// At most this many bytes are read from one connection.
pub const MAX_REQUEST_BYTES: u64 = 70 * 1024;
/// At most this many bytes in the request line or one header line.
pub const MAX_LINE_BYTES: usize = 8 * 1024;
/// At most this many body bytes (`Content-Length`).
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// At most this many header lines.
pub const MAX_HEADERS: usize = 64;
/// Transactions the relay sends for one swap, at most.
pub const MAX_RELAYS_PER_SWAP: u32 = 3;
/// Open connections from one IP address, at most.
pub const MAX_CONNECTIONS_PER_SOURCE: usize = 2;
/// `POST /execute-intents` requests from one IP address per minute, at most.
pub const MAX_RELAYS_PER_SOURCE_PER_MINUTE: usize = 6;
/// Sources remembered for the limits above, at most (memory bound).
pub const MAX_SOURCES: usize = 1024;
/// Store prefix of the relay's own records (one per swap: count and sent payloads).
pub const PREFIX_RELAY: &str = "relay/";

pub struct Relay {
    pub near_rpc: String,
    pub verifier: String,
    /// The NEP-141 token the maker pays (`chains.token`): the only token whose
    /// `ft_withdraw` is relayed.
    pub token: String,
    pub maker_account: String,
    pub key: NearKey,
    pub store_dir: PathBuf,
}

/// A refusal with the HTTP status class: `retry` is answered with 503 (the browser tries
/// again later), otherwise 400.
#[derive(Debug)]
pub struct Refused {
    pub retry: bool,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Refused {}

fn refuse(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refused { retry: false, message: message.into() })
}

fn later(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Refused { retry: true, message: message.into() })
}

/// The HTTP status of an error from [`Relay::handle`] or from reading a request.
pub fn status_of(e: &anyhow::Error) -> u16 {
    match e.downcast_ref::<Refused>() {
        Some(r) if r.retry => 503,
        _ => 400,
    }
}

/// The inner message of a signed Verifier payload (`raw_ed25519` or `nep413`).
fn payload_message(p: &Value) -> Result<Value> {
    let standard = p["standard"].as_str().ok_or_else(|| anyhow!("payload without a standard"))?;
    Ok(match standard {
        "raw_ed25519" => serde_json::from_str::<Value>(p["payload"].as_str().ok_or_else(|| anyhow!("raw_ed25519 payload is not a string"))?)?,
        "nep413" => {
            let msg = p["payload"]["message"].as_str().ok_or_else(|| anyhow!("nep413 payload without a message"))?;
            serde_json::from_str::<Value>(msg)?
        }
        other => bail!("unsupported standard {other}"),
    })
}

/// The `signer_id` of a signed Verifier payload (`raw_ed25519` or `nep413`).
pub fn payload_signer(p: &Value) -> Result<String> {
    Ok(payload_message(p)?["signer_id"].as_str().ok_or_else(|| anyhow!("payload without signer_id"))?.to_string())
}

/// The one exit the relay pays for: `ft_withdraw` of `token`, without `msg`.
struct Withdraw {
    amount: u128,
}

fn check_exit(inner: &Value, verifier: &str, token: &str) -> Result<Withdraw> {
    if inner["verifying_contract"].as_str() != Some(verifier) {
        return Err(refuse(format!("the payload is not for the Verifier {verifier}")));
    }
    let intents = inner["intents"].as_array().ok_or_else(|| refuse("the payload has no intents"))?;
    if intents.len() != 1 {
        return Err(refuse(format!("the relay sends exactly one ft_withdraw intent, the payload has {}", intents.len())));
    }
    let i = intents[0].as_object().ok_or_else(|| refuse("the intent is not an object"))?;
    if i.get("intent").and_then(Value::as_str) != Some("ft_withdraw") {
        return Err(refuse("the relay sends only ft_withdraw intents"));
    }
    if let Some(k) = i.keys().find(|k| !matches!(k.as_str(), "intent" | "token" | "receiver_id" | "amount" | "memo")) {
        return Err(refuse(format!("the ft_withdraw has a field the relay does not send: {k}")));
    }
    if i.get("token").and_then(Value::as_str) != Some(token) {
        return Err(refuse(format!("the relay sends only withdrawals of {token}")));
    }
    if i.get("receiver_id").and_then(Value::as_str).is_none_or(str::is_empty) {
        return Err(refuse("the ft_withdraw has no receiver"));
    }
    let amount = i
        .get("amount")
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|a| a.parse::<u128>().ok())
        .filter(|a| *a > 0)
        .ok_or_else(|| refuse("the ft_withdraw amount is not a positive whole number"))?;
    Ok(Withdraw { amount })
}

/// A swap of this maker with a payout to the taker.
struct Paid {
    rfq_id: String,
    price: u128,
}

/// Stages in which the maker may not have seen the taker's claim yet.
const BEFORE_CLAIM_SEEN: [&str; 2] = ["Opened", "Ready"];

fn stage_name(s: &Value) -> &str {
    match &s["stage"] {
        Value::String(n) => n,
        Value::Object(o) => o.keys().next().map(String::as_str).unwrap_or(""),
        _ => "",
    }
}

fn has_payout(s: &Value) -> bool {
    matches!(stage_name(s), "Sweeping" | "Swept") || s["taker_withdraw_at"].is_u64()
}

/// The relay's record of one swap.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RelayRecord {
    /// Transactions sent (or attempted) for this swap.
    count: u32,
    sent: Vec<Sent>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Sent {
    nonce: String,
    hash: Option<String>,
    /// `None` until the outcome is known.
    error: Option<String>,
    at: u64,
}

/// Serializes the check and update of the relay records between worker threads.
static RECORDS: Mutex<()> = Mutex::new(());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding a lock must not stop the relay: the data is still usable.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Relay {
    /// The swap of `who` that has a payout, or why there is none.
    fn paid_swap(&self, store: &FileStore, who: &str) -> Result<Paid> {
        let mut waiting = None;
        let mut other = None;
        for k in store.keys(PREFIX_SWAP)? {
            let Some(s) = get_json::<Value>(store, &k)? else { continue };
            if s["taker_account"].as_str() != Some(who) {
                continue;
            }
            let id = s["rfq_id"].as_str().map(str::to_string).unwrap_or_else(|| k.trim_start_matches(PREFIX_SWAP).to_string());
            if has_payout(&s) {
                let price = s["price"].as_str().and_then(|p| p.parse::<u128>().ok()).ok_or_else(|| anyhow!("swap {id} has no price"))?;
                return Ok(Paid { rfq_id: id, price });
            }
            let stage = stage_name(&s).to_string();
            if BEFORE_CLAIM_SEEN.contains(&stage.as_str()) {
                waiting = Some(stage);
            } else {
                other = Some(stage);
            }
        }
        if let Some(stage) = waiting {
            return Err(later(format!("the maker has not seen the claim of {who}'s swap yet (stage {stage}); try again in a minute")));
        }
        if let Some(stage) = other {
            return Err(refuse(format!("{who} has no payout from this maker to relay (swap stage {stage})")));
        }
        Err(refuse(format!("{who} is not the taker of a swap with this maker")))
    }

    /// Checks and sends one request body. Returns the answer body. A refusal is a
    /// [`Refused`] error (see [`status_of`]).
    pub fn handle(&self, rpc: &NearRpc, signer: &Signer, body: &[u8]) -> Result<Value> {
        let v: Value = serde_json::from_slice(body).map_err(|e| refuse(format!("the body is not JSON: {e}")))?;
        let signed = v["signed"].as_array().ok_or_else(|| refuse("expected {\"signed\": [...]}"))?;
        if signed.len() != 1 {
            return Err(refuse(format!("exactly one signed payload is relayed, got {}", signed.len())));
        }
        let inner = payload_message(&signed[0]).map_err(|e| refuse(format!("{e:#}")))?;
        let who = inner["signer_id"].as_str().ok_or_else(|| refuse("payload without signer_id"))?.to_string();
        let nonce = inner["nonce"].as_str().ok_or_else(|| refuse("payload without a nonce"))?.to_string();
        let w = check_exit(&inner, &self.verifier, &self.token)?;
        let mut store = FileStore::new(&self.store_dir)?;
        let paid = self.paid_swap(&store, &who)?;
        if w.amount > paid.price {
            return Err(refuse(format!("the ft_withdraw moves {} but swap {} paid {}", w.amount, paid.rfq_id, paid.price)));
        }
        let key = format!("{PREFIX_RELAY}{}", paid.rfq_id);
        // Cheap checks first, so that a payload sent before is answered without a view call.
        if let Some(answer) = self.check_record(&store, &key, &nonce)? {
            return Ok(answer);
        }
        let args = json!({ "signed": signed });
        rpc.view(&self.verifier, "simulate_intents", &args, &At::Optimistic).map_err(|e| refuse(format!("simulate_intents refused the payload: {e:#}")))?;
        {
            // Count before sending, so that a crash after the send cannot give more.
            let _g = lock(&RECORDS);
            if let Some(answer) = self.check_record(&store, &key, &nonce)? {
                return Ok(answer);
            }
            let mut rec = get_json::<RelayRecord>(&store, &key)?.unwrap_or_default();
            rec.count += 1;
            rec.sent.retain(|s| s.nonce != nonce);
            rec.sent.push(Sent { nonce: nonce.clone(), hash: None, error: None, at: unix_now() });
            put_json(&mut store, &key, &rec)?;
        }
        let out = self.send(rpc, signer, &args);
        let _g = lock(&RECORDS);
        let mut rec = get_json::<RelayRecord>(&store, &key)?.unwrap_or_default();
        let answer = match &out {
            Ok(o) => {
                if let Some(s) = rec.sent.iter_mut().find(|s| s.nonce == nonce) {
                    s.hash = Some(o.hash.clone());
                    s.error = o.failure().map(str::to_string);
                }
                match o.failure() {
                    Some(f) => json!({"ok": false, "hash": o.hash, "error": f}),
                    None if o.result.is_none() => json!({"ok": false, "hash": o.hash, "error": "not executed yet", "pending": true}),
                    None => json!({"ok": true, "hash": o.hash}),
                }
            }
            Err(_) => Value::Null,
        };
        put_json(&mut store, &key, &rec)?;
        match out {
            Ok(_) => Ok(answer),
            Err(e) => Err(later(format!("execute_intents was not sent: {e:#}"))),
        }
    }

    /// `Some(answer)` when this payload was sent before (it is not sent again); an error
    /// when the swap used up its relays.
    fn check_record(&self, store: &FileStore, key: &str, nonce: &str) -> Result<Option<Value>> {
        let rec = get_json::<RelayRecord>(store, key)?.unwrap_or_default();
        if let Some(s) = rec.sent.iter().find(|s| s.nonce == nonce)
            && let Some(h) = &s.hash
        {
            return match &s.error {
                Some(f) => Err(refuse(format!("this payload was sent before in tx {h} and failed: {f}"))),
                None => Ok(Some(json!({"ok": false, "pending": true, "hash": h, "error": format!("this payload was sent before in tx {h}")}))),
            };
        }
        if rec.count >= MAX_RELAYS_PER_SWAP {
            return Err(refuse(format!("the relay already sent {} transactions for this swap; it sends at most {MAX_RELAYS_PER_SWAP}", rec.count)));
        }
        Ok(None)
    }

    fn send(&self, rpc: &NearRpc, signer: &Signer, args: &Value) -> Result<zecret_core::near::TxOutcome> {
        let mut last = None;
        for _ in 0..3 {
            match rpc.transact(signer, &self.verifier, vec![Action::call("execute_intents", args, EXECUTE_GAS, 0)]) {
                Ok(o) => return Ok(o),
                // The maker loop signs with the same key: a nonce race is retried.
                Err(e) if format!("{e:#}").contains("InvalidNonce") => {
                    last = Some(e);
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("execute_intents was not sent")))
    }

    /// Serves on `addr` until the process ends (threads of `zecret maker`).
    pub fn serve(self, addr: &str) -> Result<()> {
        let listener = TcpListener::bind(addr).with_context(|| format!("relay: bind {addr}"))?;
        self.serve_listener(listener, || Ok(Rc::new(Http::new()?) as SharedTransport))
    }

    /// Serves on a bound listener; each worker thread makes its NEAR transport with
    /// `transport` (tests pass a scripted one).
    pub fn serve_listener<F>(self, listener: TcpListener, transport: F) -> Result<()>
    where
        F: Fn() -> Result<SharedTransport> + Send + Sync + 'static,
    {
        let relay = Arc::new(self);
        let signer = Arc::new(Mutex::new(Signer::new(&relay.maker_account, relay.key.clone())));
        let sources = Arc::new(Mutex::new(Sources::default()));
        let transport = Arc::new(transport);
        let (tx, rx) = mpsc::channel::<(TcpStream, Slot)>();
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..WORKERS {
            let (relay, signer, transport, rx) = (relay.clone(), signer.clone(), transport.clone(), rx.clone());
            std::thread::spawn(move || worker(&relay, &signer, &*transport, &rx));
        }
        for conn in listener.incoming() {
            let Ok(s) = conn else { continue };
            let ip = s.peer_addr().map(|a| a.ip()).ok();
            match Slot::take(&sources, ip) {
                Ok(slot) => {
                    if tx.send((s, slot)).is_err() {
                        bail!("relay: all workers stopped");
                    }
                }
                Err(why) => {
                    let mut s = s;
                    let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                    let _ = write_response(&mut s, 503, &json!({"ok": false, "error": why}));
                    let _ = s.shutdown(Shutdown::Both);
                }
            }
        }
        Ok(())
    }
}

/// Per-source and global limits, shared by the accept loop and the workers.
#[derive(Default)]
struct Sources {
    busy: usize,
    by_ip: HashMap<IpAddr, Source>,
}

#[derive(Default)]
struct Source {
    open: usize,
    relays: VecDeque<Instant>,
}

impl Sources {
    fn prune(&mut self) {
        let minute = Duration::from_secs(60);
        self.by_ip.retain(|_, s| {
            while s.relays.front().is_some_and(|t| t.elapsed() > minute) {
                s.relays.pop_front();
            }
            s.open > 0 || !s.relays.is_empty()
        });
    }
}

/// One connection's claim on a worker and on its source's limit; released on drop (also
/// when the worker panics).
struct Slot {
    sources: Arc<Mutex<Sources>>,
    ip: Option<IpAddr>,
}

impl Slot {
    fn take(sources: &Arc<Mutex<Sources>>, ip: Option<IpAddr>) -> std::result::Result<Slot, String> {
        let mut g = lock(sources);
        if g.busy >= WORKERS {
            return Err("the relay is busy; try again in a few seconds".into());
        }
        if let Some(ip) = ip {
            if !g.by_ip.contains_key(&ip) && g.by_ip.len() >= MAX_SOURCES {
                g.prune();
                if g.by_ip.len() >= MAX_SOURCES {
                    return Err("the relay is busy; try again in a few seconds".into());
                }
            }
            let src = g.by_ip.entry(ip).or_default();
            if src.open >= MAX_CONNECTIONS_PER_SOURCE {
                return Err(format!("at most {MAX_CONNECTIONS_PER_SOURCE} open connections per address; try again in a few seconds"));
            }
            src.open += 1;
        }
        g.busy += 1;
        Ok(Slot { sources: sources.clone(), ip })
    }

    /// Counts one relay request of this source; `false` when it is over its limit.
    fn relay_allowed(&self) -> bool {
        let Some(ip) = self.ip else { return true };
        let mut g = lock(&self.sources);
        let src = g.by_ip.entry(ip).or_default();
        while src.relays.front().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
            src.relays.pop_front();
        }
        if src.relays.len() >= MAX_RELAYS_PER_SOURCE_PER_MINUTE {
            return false;
        }
        src.relays.push_back(Instant::now());
        true
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut g = lock(&self.sources);
        g.busy = g.busy.saturating_sub(1);
        if let Some(ip) = self.ip
            && let Some(src) = g.by_ip.get_mut(&ip)
        {
            src.open = src.open.saturating_sub(1);
            if src.open == 0 && src.relays.is_empty() {
                g.by_ip.remove(&ip);
            }
        }
    }
}

fn worker(relay: &Relay, signer: &Mutex<Signer>, transport: &(dyn Fn() -> Result<SharedTransport> + Send + Sync), rx: &Mutex<Receiver<(TcpStream, Slot)>>) {
    let rpc = match transport() {
        Ok(t) => NearRpc::new(t, &relay.near_rpc),
        Err(e) => {
            eprintln!("relay: worker without a NEAR transport: {e:#}");
            return;
        }
    };
    loop {
        let job = lock(rx).recv();
        let Ok((mut s, slot)) = job else { return };
        let r = catch_unwind(AssertUnwindSafe(|| serve_one(relay, &rpc, signer, &mut s, &slot)));
        if r.is_err() {
            let _ = write_response(&mut s, 500, &json!({"ok": false, "error": "the relay failed on this request"}));
        }
        // Free the slot before the client sees the connection close, so that its next
        // request is not counted against it.
        drop(slot);
        let _ = s.shutdown(Shutdown::Both);
    }
}

fn serve_one(relay: &Relay, rpc: &NearRpc, signer: &Mutex<Signer>, s: &mut TcpStream, slot: &Slot) {
    let _ = s.set_write_timeout(Some(REQUEST_DEADLINE));
    let (status, body) = match read_request(s, Instant::now() + REQUEST_DEADLINE) {
        Ok((method, _, _)) if method == "OPTIONS" => (204, Value::Null),
        Ok((method, path, body)) if method == "POST" && path == "/execute-intents" => {
            if !slot.relay_allowed() {
                (503, json!({"ok": false, "error": format!("at most {MAX_RELAYS_PER_SOURCE_PER_MINUTE} relay requests a minute per address; try again later")}))
            } else {
                // One transaction at a time with the maker's key, so that nonces stay ordered.
                let signer = lock(signer);
                match relay.handle(rpc, &signer, &body) {
                    Ok(v) => (200, v),
                    Err(e) => (status_of(&e), json!({"ok": false, "error": format!("{e:#}")})),
                }
            }
        }
        Ok((method, path, _)) => (404, json!({"ok": false, "error": format!("no route {method} {path}")})),
        Err(e) => (status_of(&e), json!({"ok": false, "error": format!("{e:#}")})),
    };
    let _ = write_response(s, status, &body);
}

/// Reads from the stream with one deadline for the whole request: every read gets only
/// the time that is left.
struct DeadlineReader<'a> {
    s: &'a TcpStream,
    until: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the request took too long"));
        }
        self.s.set_read_timeout(Some(left))?;
        let mut s = self.s;
        s.read(buf)
    }
}

fn read_line<R: BufRead>(r: &mut R) -> Result<String> {
    let mut buf = Vec::new();
    let n = r.take(MAX_LINE_BYTES as u64 + 1).read_until(b'\n', &mut buf).map_err(io_refusal)?;
    if n == 0 {
        return Err(refuse("the request ended early"));
    }
    if buf.last() != Some(&b'\n') {
        return Err(refuse(if buf.len() > MAX_LINE_BYTES { format!("a header line is longer than {MAX_LINE_BYTES} bytes") } else { "the request ended early or is too large".to_string() }));
    }
    String::from_utf8(buf).map_err(|_| refuse("the request is not UTF-8"))
}

fn io_refusal(e: io::Error) -> anyhow::Error {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => refuse(format!("the request did not arrive within {} s", REQUEST_DEADLINE.as_secs())),
        _ => refuse(format!("reading the request: {e}")),
    }
}

/// Reads one request within `deadline` and [`MAX_REQUEST_BYTES`].
fn read_request(s: &TcpStream, deadline: Instant) -> Result<(String, String, Vec<u8>)> {
    let mut r = BufReader::new(DeadlineReader { s, until: deadline }.take(MAX_REQUEST_BYTES));
    let line = read_line(&mut r)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut len = 0usize;
    let mut headers = 0;
    loop {
        let h = read_line(&mut r)?;
        if h == "\r\n" || h == "\n" {
            break;
        }
        headers += 1;
        if headers > MAX_HEADERS {
            return Err(refuse(format!("more than {MAX_HEADERS} headers")));
        }
        if let Some((k, v)) = h.split_once(':') {
            let k = k.trim();
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().map_err(|_| refuse("Content-Length is not a number"))?;
            } else if k.eq_ignore_ascii_case("transfer-encoding") {
                return Err(refuse("Transfer-Encoding is not accepted; send Content-Length"));
            }
        }
    }
    if len > MAX_BODY_BYTES {
        return Err(refuse(format!("the body is larger than {MAX_BODY_BYTES} bytes")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(io_refusal)?;
    Ok((method, path, body))
}

fn write_response(s: &mut TcpStream, status: u16, body: &Value) -> Result<()> {
    let text = if body.is_null() { String::new() } else { body.to_string() };
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Bad Request",
    };
    write!(
        s,
        "HTTP/1.1 {status} {reason}\r\naccess-control-allow-origin: *\r\naccess-control-allow-methods: POST, OPTIONS\r\naccess-control-allow-headers: content-type\r\naccess-control-max-age: 600\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    )?;
    s.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_signer_of_both_standards() {
        let raw = json!({"standard": "raw_ed25519", "payload": "{\"signer_id\":\"abc\",\"verifying_contract\":\"intents.near\"}", "public_key": "ed25519:x", "signature": "ed25519:y"});
        assert_eq!(payload_signer(&raw).unwrap(), "abc");
        let nep = json!({"standard": "nep413", "payload": {"message": "{\"signer_id\":\"def\"}", "nonce": "", "recipient": "intents.near"}});
        assert_eq!(payload_signer(&nep).unwrap(), "def");
        assert!(payload_signer(&json!({"standard": "erc191"})).is_err());
    }

    fn inner(intents: Value) -> Value {
        json!({"signer_id": "t", "verifying_contract": "intents.test", "nonce": "n", "intents": intents})
    }

    #[test]
    fn only_one_ft_withdraw_of_the_token_is_an_exit() {
        let w = |extra: Value| {
            let mut i = json!({"intent": "ft_withdraw", "token": "usdc.test", "receiver_id": "alice.test", "amount": "5"});
            for (k, v) in extra.as_object().unwrap() {
                i[k] = v.clone();
            }
            i
        };
        assert_eq!(check_exit(&inner(json!([w(json!({}))])), "intents.test", "usdc.test").unwrap().amount, 5);
        assert!(check_exit(&inner(json!([w(json!({"memo": "x"}))])), "intents.test", "usdc.test").is_ok());
        let bad = [
            inner(json!([])),
            inner(json!([w(json!({})), w(json!({}))])),
            inner(json!([w(json!({"token": "other.test"}))])),
            inner(json!([w(json!({"msg": "call me"}))])),
            inner(json!([w(json!({"amount": "0"}))])),
            inner(json!([w(json!({"amount": "-1"}))])),
            inner(json!([w(json!({"amount": 5}))])),
            inner(json!([w(json!({"receiver_id": ""}))])),
            inner(json!([{"intent": "transfer", "receiver_id": "x", "tokens": {"nep141:usdc.test": "1"}}])),
            inner(json!([{"intent": "add_public_key", "public_key": "ed25519:x"}])),
        ];
        for b in bad {
            let e = check_exit(&b, "intents.test", "usdc.test").err().unwrap_or_else(|| panic!("accepted {b}"));
            assert_eq!(status_of(&e), 400, "{b}");
        }
        let mut other_verifier = inner(json!([w(json!({}))]));
        other_verifier["verifying_contract"] = json!("intents.other");
        assert!(check_exit(&other_verifier, "intents.test", "usdc.test").is_err());
    }

    #[test]
    fn a_payout_shows_in_the_stage_or_in_the_withdraw() {
        assert!(has_payout(&json!({"stage": "Sweeping"})));
        assert!(has_payout(&json!({"stage": "Swept"})));
        assert!(has_payout(&json!({"stage": "Ready", "taker_withdraw_at": 1})));
        assert!(has_payout(&json!({"stage": {"Aborted": "claimed via the punish path without funding"}, "taker_withdraw_at": 7})));
        assert!(!has_payout(&json!({"stage": "Ready", "taker_withdraw_at": null})));
        assert!(!has_payout(&json!({"stage": {"Aborted": "request expired"}})));
        assert!(!has_payout(&json!({"stage": "Quoting"})));
        assert!(!has_payout(&json!({"stage": "Refunded"})));
        assert_eq!(stage_name(&json!({"stage": {"Aborted": "x"}})), "Aborted");
    }

    #[test]
    fn at_most_four_connections_at_once_and_two_per_source() {
        let sources = Arc::new(Mutex::new(Sources::default()));
        let ip = |n: u8| Some(IpAddr::from([10, 0, 0, n]));
        let held: Vec<Slot> = (1..=WORKERS as u8).map(|n| Slot::take(&sources, ip(n)).unwrap()).collect();
        assert!(Slot::take(&sources, ip(99)).err().unwrap().contains("busy"));
        drop(held);
        let a = Slot::take(&sources, ip(1)).unwrap();
        let b = Slot::take(&sources, ip(1)).unwrap();
        assert!(Slot::take(&sources, ip(1)).err().unwrap().contains("per address"));
        assert!(Slot::take(&sources, ip(2)).is_ok());
        for _ in 0..MAX_RELAYS_PER_SOURCE_PER_MINUTE {
            assert!(a.relay_allowed());
        }
        assert!(!b.relay_allowed(), "the limit is per address, not per connection");
        drop((a, b));
        // The address keeps its relay count after its connections close (memory: one entry).
        assert_eq!(lock(&sources).by_ip.len(), 1);
        assert_eq!(lock(&sources).busy, 0);
    }

    #[test]
    fn lines_and_requests_are_bounded() {
        let mut r = io::Cursor::new(vec![b'a'; MAX_LINE_BYTES + 100]);
        let e = read_line(&mut r).unwrap_err();
        assert!(format!("{e}").contains("longer than"), "{e}");
        let mut r = io::Cursor::new(b"GET / HTTP/1.1\r\n".to_vec());
        assert_eq!(read_line(&mut r).unwrap(), "GET / HTTP/1.1\r\n");
        let mut r = io::Cursor::new(b"no newline".to_vec());
        assert!(read_line(&mut r).is_err());
    }
}
