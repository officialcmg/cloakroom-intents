// Shared helpers for the security review 2 proofs of concept.
//
// A scripted JSON-RPC transport (Zebra and NEAR both speak JSON-RPC over one POST), a
// taker configuration, and a way to make an Orchard note for the wallet's note list.
// Nothing here touches a network.

#![allow(dead_code)]

use std::cell::RefCell;

use anyhow::Result;
use serde_json::{json, Value};
use zecret_core::{
    config::TakerConfig,
    net::{Response, Transport},
    wallet::{empty_memo, Received},
};

/// A JSON-RPC transport that answers from a closure and records every request.
pub struct Mock {
    pub handler: Box<dyn Fn(&str, &Value) -> Value>,
    pub calls: RefCell<Vec<(String, Value)>>,
}

impl Mock {
    pub fn new(handler: impl Fn(&str, &Value) -> Value + 'static) -> Mock {
        Mock { handler: Box::new(handler), calls: RefCell::new(vec![]) }
    }
    pub fn count(&self, method: &str) -> usize {
        self.calls.borrow().iter().filter(|(m, _)| m == method).count()
    }
}

impl Transport for Mock {
    fn post(&self, _url: &str, _headers: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        let req: Value = serde_json::from_slice(body)?;
        let method = req["method"].as_str().unwrap_or("").to_string();
        self.calls.borrow_mut().push((method.clone(), req["params"].clone()));
        let result = (self.handler)(&method, &req["params"]);
        let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "result": result}))?;
        Ok(Response { status: 200, body, headers: vec![] })
    }
}

/// A NEAR `query` result for a view call that returns `v` as JSON.
pub fn view_result(v: &Value) -> Value {
    let bytes: Vec<u8> = serde_json::to_vec(v).unwrap();
    json!({"result": bytes, "logs": [], "block_height": 1, "block_hash": "11111111111111111111111111111111"})
}

/// A NEAR `block` result at `secs` (block time in seconds).
pub fn block_result(secs: u64) -> Value {
    json!({"header": {"hash": "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi", "height": 1000, "timestamp": secs * 1_000_000_000}})
}

/// The method name of a NEAR `query` call_function request, if it is one.
pub fn view_method(params: &Value) -> Option<String> {
    if params["request_type"] == "call_function" { params["method_name"].as_str().map(str::to_string) } else { None }
}

/// A taker configuration for the local tests (Zebra JSON-RPC backend, regtest).
pub fn taker_cfg(maker_ua: &str) -> TakerConfig {
    serde_json::from_value(json!({
        "chains": {"zebra_rpc": "http://zebra.invalid", "nu7_height": 2, "near_rpc": "http://near.invalid",
                   "escrow": "escrow.near", "token": "usdc.near", "intents": "intents.near"},
        "timing": {"inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2, "margin_secs": 0, "fund_secs": 120, "k_z": 2},
        "maker": {"ua": maker_ua, "epoch": 1, "rfq_pub": "00".repeat(32), "near_account": "maker.near",
                  "near_pk": "ed25519:11111111111111111111111111111111", "sig": "00".repeat(64)},
        "escrow_code_hash": "CODEHASH",
        "min_price": "1000",
        "max_premium": 50000,
        "min_stipend": "30000000000000000000000",
    }))
    .unwrap()
}

/// A received note of `zats` at `to`, as the wallet's scan would record it.
pub fn note(label: &str, to: orchard::Address, zats: u64, pos: u64, height: u64, txid: &str, memo: [u8; 512]) -> Received {
    use orchard::note::{RandomSeed, Rho};
    let mut rho_b = [0u8; 32];
    rho_b[..8].copy_from_slice(&(pos + 1).to_le_bytes());
    let rho = Option::from(Rho::from_bytes(&rho_b)).expect("rho");
    let mut seed = [7u8; 32];
    seed[..8].copy_from_slice(&pos.to_le_bytes());
    let rseed = Option::from(RandomSeed::from_bytes(seed, &rho)).expect("rseed");
    let n = Option::from(orchard::Note::from_parts(to, orchard::value::NoteValue::from_raw(zats), rho, rseed, orchard::NoteVersion::V2)).expect("note");
    let mut nf = [0u8; 32];
    nf[..8].copy_from_slice(&pos.to_le_bytes());
    nf[31] = 0x5a;
    Received { label: label.to_string(), note: n, pos, height, txid: txid.to_string(), memo, nf, spent_in: None }
}

pub fn plain_memo() -> [u8; 512] {
    empty_memo()
}
