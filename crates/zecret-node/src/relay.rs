//! The maker's relay for the direct exit (local network and testnet).
//!
//! On networks where 1Click cannot execute the exit, the taker's browser signs one
//! Verifier intent (an `ft_withdraw` to a NEAR account the user named) and someone must
//! send it with `execute_intents`. The taker's own account cannot: `ft_withdraw` needs more
//! than 35 Tgas attached (it works with 40), and what is left of the 0.03 NEAR stipend after
//! commit and claim (0.0281 NEAR) pays for at most about 25 Tgas at the pessimistic gas
//! price that NEAR checks before it accepts a transaction (measured in
//! `crates/zecret-wasm/tests/sandbox.rs`).
//! So the maker sends it, as it already sends `withdraw` for the taker.
//!
//! `POST /execute-intents` with `{"signed": [<one signed payload>]}`. The relay sends it only
//! if the payload's signer is the taker account of one of this maker's swaps (read from the
//! maker's store) and `simulate_intents` accepts it (a free view call), so a stranger cannot
//! make the maker pay gas. The signed payload moves the taker's funds as the taker signed;
//! the relay cannot change it. Answers carry `Access-Control-Allow-Origin: *`.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};
use zecret_core::{
    maker::PREFIX_SWAP,
    near::{tx::Action, At, NearKey, NearRpc, Signer, TGAS},
    store::{get_json, Store},
};

use crate::io::{FileStore, Http};

/// Gas attached to `execute_intents` with one `ft_withdraw`: the call fails with
/// "Exceeded the prepaid gas" at 35 Tgas and works at 40 Tgas and above, costing about
/// 0.001 NEAR (sandbox, live `intents.near` and `wrap.near` code; see
/// `crates/zecret-wasm/tests/sandbox.rs`). Unused gas is refunded.
pub const EXECUTE_GAS: u64 = 50 * TGAS;

pub struct Relay {
    pub near_rpc: String,
    pub verifier: String,
    pub maker_account: String,
    pub key: NearKey,
    pub store_dir: PathBuf,
}

/// The `signer_id` of a signed Verifier payload (`raw_ed25519` or `nep413`).
pub fn payload_signer(p: &Value) -> Result<String> {
    let standard = p["standard"].as_str().ok_or_else(|| anyhow!("payload without a standard"))?;
    let inner = match standard {
        "raw_ed25519" => serde_json::from_str::<Value>(p["payload"].as_str().ok_or_else(|| anyhow!("raw_ed25519 payload is not a string"))?)?,
        "nep413" => {
            let msg = p["payload"]["message"].as_str().ok_or_else(|| anyhow!("nep413 payload without a message"))?;
            serde_json::from_str::<Value>(msg)?
        }
        other => bail!("unsupported standard {other}"),
    };
    Ok(inner["signer_id"].as_str().ok_or_else(|| anyhow!("payload without signer_id"))?.to_string())
}

impl Relay {
    fn takers(&self) -> Result<Vec<String>> {
        let store = FileStore::new(&self.store_dir)?;
        let mut out = vec![];
        for k in store.keys(PREFIX_SWAP)? {
            if let Some(s) = get_json::<Value>(&store, &k)?
                && let Some(a) = s["taker_account"].as_str()
            {
                out.push(a.to_string());
            }
        }
        Ok(out)
    }

    /// Checks and sends one request body. Returns the answer body.
    pub fn handle(&self, rpc: &NearRpc, signer: &Signer, body: &[u8]) -> Result<Value> {
        let v: Value = serde_json::from_slice(body).context("the body is not JSON")?;
        let signed = v["signed"].as_array().ok_or_else(|| anyhow!("expected {{\"signed\": [...]}}"))?;
        if signed.len() != 1 {
            bail!("exactly one signed payload is relayed, got {}", signed.len());
        }
        let who = payload_signer(&signed[0])?;
        if !self.takers()?.contains(&who) {
            bail!("{who} is not the taker of a swap with this maker");
        }
        let args = json!({ "signed": signed });
        rpc.view(&self.verifier, "simulate_intents", &args, &At::Optimistic).context("simulate_intents refused the payload")?;
        let mut last = None;
        for _ in 0..3 {
            let out = rpc.transact(signer, &self.verifier, vec![Action::call("execute_intents", &args, EXECUTE_GAS, 0)]);
            match out {
                Ok(o) => {
                    return match o.failure() {
                        Some(f) => Ok(json!({"ok": false, "hash": o.hash, "error": f})),
                        None if o.result.is_none() => Ok(json!({"ok": false, "hash": o.hash, "error": "not executed yet", "pending": true})),
                        None => Ok(json!({"ok": true, "hash": o.hash})),
                    };
                }
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

    /// Serves on `addr` until the process ends (a thread of `zecret maker`).
    pub fn serve(self, addr: &str) -> Result<()> {
        let listener = TcpListener::bind(addr).with_context(|| format!("relay: bind {addr}"))?;
        let rpc = NearRpc::new(Rc::new(Http::new()?), &self.near_rpc);
        let signer = Signer::new(&self.maker_account, self.key.clone());
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
            let answer = match read_request(&mut s) {
                Ok((method, path, _)) if method == "OPTIONS" => {
                    let _ = path;
                    (204, Value::Null)
                }
                Ok((method, path, body)) if method == "POST" && path == "/execute-intents" => match self.handle(&rpc, &signer, &body) {
                    Ok(v) => (200, v),
                    Err(e) => (400, json!({"ok": false, "error": format!("{e:#}")})),
                },
                Ok((method, path, _)) => (404, json!({"ok": false, "error": format!("no route {method} {path}")})),
                Err(e) => (400, json!({"ok": false, "error": format!("{e:#}")})),
            };
            let _ = write_response(&mut s, answer.0, &answer.1);
        }
        Ok(())
    }
}

fn read_request(s: &mut TcpStream) -> Result<(String, String, Vec<u8>)> {
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    if len > 64 * 1024 {
        bail!("body too large");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok((method, path, body))
}

fn write_response(s: &mut TcpStream, status: u16, body: &Value) -> Result<()> {
    let text = if body.is_null() { String::new() } else { body.to_string() };
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
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
}
