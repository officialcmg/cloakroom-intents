// The maker's relay (src/relay.rs) against security review 3, F3-01 and F3-02.
//
// The three proofs of concept of docs/security/poc3/relay_pocs.rs are here, adapted to the
// fixed relay: `Relay` names the token it relays, the F3-01 payloads are real ft_withdraw
// intents, and the F3-02b client stops streaming when the relay closes the connection.
// They failed on the reviewed code (see review-3.md) and pass now. A scripted NEAR RPC
// stands in for the network; the TCP tests use loopback ports. No other network is used.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use serde_json::{json, Value};
use zecret_core::{
    near::{NearKey, NearRpc, Signer},
    net::{Response, SharedTransport, Transport},
    store::{get_json, put_json},
};
use zecret_node::{
    io::FileStore,
    relay::{status_of, Relay, MAX_RELAYS_PER_SOURCE_PER_MINUTE, MAX_RELAYS_PER_SWAP},
};

/// A NEAR JSON-RPC that accepts every simulate_intents and executes every transaction.
/// `panic_on_send` makes send_tx panic (a bug in the relay's call path).
struct Chain {
    sent: Arc<AtomicUsize>,
    panic_on_send: bool,
}

impl Transport for Chain {
    fn post(&self, _url: &str, _h: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        let req: Value = serde_json::from_slice(body)?;
        let p = &req["params"];
        let result = match req["method"].as_str().unwrap_or("") {
            "query" if p["request_type"] == "call_function" => {
                let bytes: Vec<u8> = serde_json::to_vec(&json!({"intents_executed": [], "logs": []}))?;
                json!({"result": bytes, "logs": [], "block_height": 1, "block_hash": "11111111111111111111111111111111"})
            }
            "query" if p["request_type"] == "view_access_key" => json!({"nonce": 100 + self.sent.load(Ordering::SeqCst) as u64, "permission": "FullAccess", "block_height": 1, "block_hash": "11111111111111111111111111111111"}),
            "block" => json!({"header": {"hash": "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi", "height": 1000, "timestamp": 1_800_000_000u64 * 1_000_000_000}}),
            "send_tx" => {
                if self.panic_on_send {
                    panic!("scripted panic in send_tx");
                }
                let n = self.sent.fetch_add(1, Ordering::SeqCst);
                json!({
                    "final_execution_status": "EXECUTED",
                    "status": {"SuccessValue": ""},
                    "transaction": {"hash": format!("tx{n}")},
                    "transaction_outcome": {"id": format!("tx{n}"), "outcome": {"logs": [], "status": {"SuccessReceiptId": "x"}, "executor_id": "maker.test"}},
                    "receipts_outcome": [{"outcome": {"logs": [], "status": {"SuccessValue": ""}, "executor_id": "intents.test"}}]
                })
            }
            m => anyhow::bail!("unexpected RPC {m} {p}"),
        };
        Ok(Response { status: 200, body: serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "result": result}))?, headers: vec![] })
    }
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zecret-relay-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn relay(dir: &std::path::Path) -> Relay {
    Relay {
        near_rpc: "http://near.invalid".into(),
        verifier: "intents.test".into(),
        token: "usdc.test".into(),
        maker_account: "maker.test".into(),
        key: NearKey::from_seed([9; 32]),
        store_dir: dir.to_path_buf(),
    }
}

/// A raw_ed25519 payload of `signer` with nonce `i` and the given intents. The relay does
/// not check the signature itself; simulate_intents does (accepted by the scripted RPC).
fn payload_with(signer: &str, i: u32, intents: Value) -> Vec<u8> {
    let inner = json!({"signer_id": signer, "verifying_contract": "intents.test", "deadline": "2030-01-01T00:00:00Z", "nonce": format!("nonce-{i}"), "intents": intents});
    json!({"signed": [{"standard": "raw_ed25519", "payload": inner.to_string(), "public_key": "ed25519:x", "signature": "ed25519:y"}]}).to_string().into_bytes()
}

fn withdraw(signer: &str, i: u32, amount: &str) -> Vec<u8> {
    payload_with(signer, i, json!([{"intent": "ft_withdraw", "token": "usdc.test", "receiver_id": "alice.test", "amount": amount}]))
}

fn put_swap(dir: &std::path::Path, id: &str, taker: &str, stage: Value, price: &str) {
    let mut st = FileStore::new(dir).unwrap();
    put_json(&mut st, &format!("swap/{id}"), &json!({"rfq_id": id, "stage": stage, "taker_account": taker, "price": price, "premium": 0, "taker_withdraw_at": null})).unwrap();
}

fn chain(sent: &Arc<AtomicUsize>) -> Rc<Chain> {
    Rc::new(Chain { sent: sent.clone(), panic_on_send: false })
}

#[test]
fn poc_f3_01_any_request_sender_drains_relay_gas() {
    let dir = scratch_dir("gas");
    // What the maker stores when a stranger's request memo arrives, later expired: no
    // premium, no escrow, no payout.
    let attacker = NearKey::from_seed([66; 32]).implicit_account();
    put_swap(&dir, "00112233445566778899aabbccddeeff", &attacker, json!({"Aborted": "request expired"}), "1");
    let sent = Arc::new(AtomicUsize::new(0));
    let rpc = NearRpc::new(chain(&sent), "http://near.invalid");
    let r = relay(&dir);
    let signer = Signer::new("maker.test", r.key.clone());
    let n = 25;
    let mut ok = 0;
    for i in 0..n {
        // The review's payloads (no intents), and real exits of the attacker's own tokens.
        for body in [payload_with(&attacker, i, json!([])), withdraw(&attacker, 1000 + i, "1")] {
            if let Ok(v) = r.handle(&rpc, &signer, &body)
                && v["ok"] == true
            {
                ok += 1;
            }
        }
    }
    let sent = sent.load(Ordering::SeqCst);
    println!("F3-01: {ok} of {} payloads of a never-paid requester relayed; the maker sent {sent} execute_intents", 2 * n);
    let stranger = NearKey::from_seed([77; 32]).implicit_account();
    let e = r.handle(&rpc, &signer, &withdraw(&stranger, 0, "1")).unwrap_err();
    assert!(format!("{e:#}").contains("is not the taker"), "{e:#}");
    assert!(sent <= 1, "the relay sent {sent} execute_intents for a requester that never paid a premium");
    assert_eq!(sent, 0);
}

#[test]
fn a_paid_out_swap_is_relayed_once_per_payload_and_at_most_three_times() {
    let dir = scratch_dir("paid");
    let taker = NearKey::from_seed([5; 32]).implicit_account();
    let id = "aa112233445566778899aabbccddeeff";
    put_swap(&dir, id, &taker, json!("Swept"), "1000");
    let sent = Arc::new(AtomicUsize::new(0));
    let rpc = NearRpc::new(chain(&sent), "http://near.invalid");
    let r = relay(&dir);
    let signer = Signer::new("maker.test", r.key.clone());

    // More than the swap paid, or another token: refused, nothing sent.
    let e = r.handle(&rpc, &signer, &withdraw(&taker, 0, "1001")).unwrap_err();
    assert_eq!(status_of(&e), 400, "{e:#}");
    let other = payload_with(&taker, 0, json!([{"intent": "ft_withdraw", "token": "wrap.test", "receiver_id": "alice.test", "amount": "1"}]));
    assert!(r.handle(&rpc, &signer, &other).is_err());
    assert_eq!(sent.load(Ordering::SeqCst), 0);

    // The exit: relayed once.
    let v = r.handle(&rpc, &signer, &withdraw(&taker, 0, "1000")).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    // The same signed payload again: not sent again; the answer names the first tx.
    let again = r.handle(&rpc, &signer, &withdraw(&taker, 0, "1000")).unwrap();
    assert_eq!(again["pending"], true, "{again}");
    assert_eq!(again["hash"], v["hash"]);
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    // New nonces count until the limit; then the swap is refused.
    for i in 1..MAX_RELAYS_PER_SWAP {
        assert_eq!(r.handle(&rpc, &signer, &withdraw(&taker, i, "1")).unwrap()["ok"], true);
    }
    assert_eq!(sent.load(Ordering::SeqCst), MAX_RELAYS_PER_SWAP as usize);
    let e = r.handle(&rpc, &signer, &withdraw(&taker, 99, "1")).unwrap_err();
    assert!(format!("{e:#}").contains("at most 3"), "{e:#}");
    assert_eq!(status_of(&e), 400);
    assert_eq!(sent.load(Ordering::SeqCst), MAX_RELAYS_PER_SWAP as usize);
    // The count is in the maker's store, so a restart of the maker does not reset it.
    let st = FileStore::new(&dir).unwrap();
    let rec: Value = get_json(&st, &format!("relay/{id}")).unwrap().unwrap();
    assert_eq!(rec["count"], MAX_RELAYS_PER_SWAP);
}

#[test]
fn a_swap_before_the_claim_is_seen_gets_try_again_later() {
    let dir = scratch_dir("ready");
    let taker = NearKey::from_seed([6; 32]).implicit_account();
    put_swap(&dir, "bb112233445566778899aabbccddeeff", &taker, json!("Ready"), "1000");
    let sent = Arc::new(AtomicUsize::new(0));
    let rpc = NearRpc::new(chain(&sent), "http://near.invalid");
    let r = relay(&dir);
    let signer = Signer::new("maker.test", r.key.clone());
    let e = r.handle(&rpc, &signer, &withdraw(&taker, 0, "1000")).unwrap_err();
    assert_eq!(status_of(&e), 503, "{e:#}");
    // Once the maker recorded its withdraw for the taker, the payout exists.
    let mut st = FileStore::new(&dir).unwrap();
    put_json(&mut st, "swap/bb112233445566778899aabbccddeeff", &json!({"rfq_id": "bb112233445566778899aabbccddeeff", "stage": "Ready", "taker_account": taker, "price": "1000", "taker_withdraw_at": 1_800_000_000u64})).unwrap();
    assert_eq!(r.handle(&rpc, &signer, &withdraw(&taker, 0, "1000")).unwrap()["ok"], true);
    assert_eq!(sent.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------- over TCP

fn start(dir: &std::path::Path, panic_on_send: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let sent = Arc::new(AtomicUsize::new(0));
    let s = sent.clone();
    let r = relay(dir);
    std::thread::spawn(move || {
        let _ = r.serve_listener(listener, move || Ok(Rc::new(Chain { sent: s.clone(), panic_on_send }) as SharedTransport));
    });
    std::thread::sleep(Duration::from_millis(100));
    (addr, sent)
}

/// Sends one request and returns the status line and the body.
fn request(addr: &str, req: &[u8]) -> (String, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(req).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out.lines().next().unwrap_or("").to_string();
    let body = out.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

fn post(body: &[u8]) -> Vec<u8> {
    let mut r = format!("POST /execute-intents HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n", body.len()).into_bytes();
    r.extend_from_slice(body);
    r
}

#[test]
fn poc_f3_02_one_slow_client_blocks_the_relay() {
    let dir = scratch_dir("slow");
    let (addr, _) = start(&dir, false);

    // The slow client: a request line, then one header byte every 2 s for 20 s.
    let a = addr.clone();
    let slow = std::thread::spawn(move || {
        let mut s = TcpStream::connect(&a).unwrap();
        s.write_all(b"POST /execute-intents HTTP/1.1\r\nx-slow: ").unwrap();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_secs(2));
            if s.write_all(b"a").is_err() {
                return;
            }
        }
    });
    std::thread::sleep(Duration::from_millis(300));

    // An honest client: a CORS preflight, answered at once by an idle relay.
    let mut s = TcpStream::connect(&addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(12))).unwrap();
    s.write_all(b"OPTIONS /execute-intents HTTP/1.1\r\nhost: x\r\n\r\n").unwrap();
    let t = Instant::now();
    let mut buf = [0u8; 64];
    let got = s.read(&mut buf);
    let waited = t.elapsed();
    println!("F3-02: honest preflight answered after {waited:?}: {:?}", got.as_ref().map(|n| String::from_utf8_lossy(&buf[..*n]).lines().next().unwrap_or("").to_string()));
    slow.join().unwrap();
    assert!(got.is_ok() && waited < Duration::from_secs(2), "one client that sends a byte every 2 s kept the relay from answering anyone for {waited:?}");
}

#[test]
fn a_request_must_arrive_within_five_seconds() {
    let dir = scratch_dir("deadline");
    let (addr, _) = start(&dir, false);
    let mut s = TcpStream::connect(&addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    s.write_all(b"POST /execute-intents HTTP/1.1\r\n").unwrap();
    let t = Instant::now();
    // One header byte a second, each within the old per-read timeout.
    let w = s.try_clone().unwrap();
    let writer = std::thread::spawn(move || {
        let mut w = w;
        for _ in 0..12 {
            std::thread::sleep(Duration::from_secs(1));
            if w.write_all(b"a").is_err() {
                return;
            }
        }
    });
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let waited = t.elapsed();
    writer.join().unwrap();
    println!("deadline: answer after {waited:?}: {}", out.lines().next().unwrap_or(""));
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    assert!(out.contains("did not arrive within 5 s"), "{out}");
    assert!(waited < Duration::from_millis(6500), "the relay waited {waited:?}");
}

fn rss_mib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let kb: u64 = s.lines().find(|l| l.starts_with("VmRSS:")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    kb / 1024
}

#[test]
fn poc_f3_02b_one_header_line_grows_the_maker_process() {
    // One connection streams a header line with no newline. The relay stops reading at
    // 8 KiB and closes; the client's writes then fail and it stops (adapted: the review's
    // client unwrapped every write).
    let dir = scratch_dir("mem");
    let (addr, _) = start(&dir, false);
    let before = rss_mib();
    let mut s = TcpStream::connect(&addr).unwrap();
    s.write_all(b"POST /execute-intents HTTP/1.1\r\nx-big: ").unwrap();
    let chunk = vec![b'a'; 1 << 20];
    let mut written = 0;
    for _ in 0..256 {
        if s.write_all(&chunk).is_err() {
            break;
        }
        written += 1;
    }
    std::thread::sleep(Duration::from_millis(500));
    let after = rss_mib();
    println!("F3-02b: process RSS {before} MiB -> {after} MiB; the client got {written} MiB out before the relay closed");
    drop(s);
    assert!(after < before + 16, "one header line grew the maker process by {} MiB", after.saturating_sub(before));
    assert!(written < 256, "the relay read the whole 256 MiB line");
}

#[test]
fn bodies_and_header_lines_are_capped() {
    let dir = scratch_dir("caps");
    let (addr, _) = start(&dir, false);
    let (st, body) = request(&addr, b"POST /execute-intents HTTP/1.1\r\ncontent-length: 70000\r\n\r\n");
    assert!(st.starts_with("HTTP/1.1 400") && body.contains("larger than"), "{st} {body}");
    let long = format!("POST /execute-intents HTTP/1.1\r\nx: {}\r\n\r\n", "a".repeat(9000));
    let (st, body) = request(&addr, long.as_bytes());
    assert!(st.starts_with("HTTP/1.1 400") && body.contains("longer than"), "{st} {body}");
    let (st, _) = request(&addr, b"POST /execute-intents HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n");
    assert!(st.starts_with("HTTP/1.1 400"), "{st}");
}

#[test]
fn one_source_gets_two_connections_and_a_few_relays_a_minute() {
    let dir = scratch_dir("source");
    let (addr, _) = start(&dir, false);
    // Two idle connections from this address hold their slots...
    let a = TcpStream::connect(&addr).unwrap();
    let b = TcpStream::connect(&addr).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // ...and a third is turned away at once with 503.
    let t = Instant::now();
    let (st, body) = request(&addr, b"OPTIONS / HTTP/1.1\r\n\r\n");
    assert!(st.starts_with("HTTP/1.1 503") && body.contains("open connections"), "{st} {body}");
    assert!(t.elapsed() < Duration::from_secs(2));
    drop((a, b));
    // The relay frees both slots once it sees the connections close.
    let t = Instant::now();
    while !request(&addr, b"OPTIONS / HTTP/1.1\r\n\r\n").0.starts_with("HTTP/1.1 204") {
        assert!(t.elapsed() < Duration::from_secs(5), "the slots of closed connections were not freed");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Relay requests per minute: the first ones are read and refused on their merits,
    // then the address is told to wait.
    let junk = post(b"{}");
    for i in 0..MAX_RELAYS_PER_SOURCE_PER_MINUTE {
        let (st, body) = request(&addr, &junk);
        assert!(st.starts_with("HTTP/1.1 400"), "request {i}: {st} {body}");
    }
    let (st, body) = request(&addr, &junk);
    assert!(st.starts_with("HTTP/1.1 503") && body.contains("a minute"), "{st} {body}");
    // Preflights are not relay requests.
    let (st, _) = request(&addr, b"OPTIONS /execute-intents HTTP/1.1\r\n\r\n");
    assert!(st.starts_with("HTTP/1.1 204"), "{st}");
}

#[test]
fn a_panic_in_one_request_does_not_stop_the_relay() {
    let dir = scratch_dir("panic");
    let taker = NearKey::from_seed([8; 32]).implicit_account();
    put_swap(&dir, "cc112233445566778899aabbccddeeff", &taker, json!("Swept"), "1000");
    let (addr, _) = start(&dir, true);
    let (st, body) = request(&addr, &post(&withdraw(&taker, 0, "10")));
    assert!(st.starts_with("HTTP/1.1 500"), "{st} {body}");
    // The relay still answers, on every worker (more requests than workers).
    for _ in 0..6 {
        let (st, _) = request(&addr, b"OPTIONS /execute-intents HTTP/1.1\r\n\r\n");
        assert!(st.starts_with("HTTP/1.1 204"), "{st}");
    }
    // The signer lock was poisoned by the panic; the next relay still runs (and panics in
    // the scripted send again, answered 500, not a hang).
    let (st, _) = request(&addr, &post(&withdraw(&taker, 1, "10")));
    assert!(st.starts_with("HTTP/1.1 500"), "{st}");
}
