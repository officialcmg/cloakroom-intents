//! The wasm-bindgen interface the worker (`web/src/engine/real/`) calls. Every function
//! blocks the worker until it is done. Structured values cross as JSON text; amounts that
//! can exceed 2^53 are decimal strings; zatoshis and heights are numbers.

use std::rc::Rc;

use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use zecret_core::{
    config::TakerConfig,
    engine,
    net::{Response, SharedTransport, Transport},
    store::MemStore,
    taker::Taker,
    wallet,
};
use zecret_intents::{HttpRequest, HttpResponse, HttpTransport, OneClick};

use crate::{exit, session::Session};

fn js(e: anyhow::Error) -> JsError {
    JsError::new(&format!("{e:#}"))
}

#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("zecret engine panic: {info}").into());
    }));
}

// ---------------------------------------------------------------- transport

/// One synchronous `XMLHttpRequest` (allowed in workers only). `Err` only when no HTTP
/// answer arrived; header names are lower case (only CORS-exposed ones are visible).
/// Status, body and lower-case headers of an answer.
type XhrAnswer = (u16, Vec<u8>, Vec<(String, String)>);

fn xhr(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<XhrAnswer, String> {
    let e = |e: JsValue| format!("{method} {url}: {}", e.as_string().unwrap_or_else(|| format!("{e:?}")));
    let x = web_sys::XmlHttpRequest::new().map_err(e)?;
    x.open_with_async(method, url, false).map_err(e)?;
    x.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
    for (k, v) in headers {
        x.set_request_header(k, v).map_err(e)?;
    }
    match body {
        Some(b) => {
            x.send_with_opt_u8_array(Some(b)).map_err(e)?
        }
        None => x.send().map_err(e)?,
    }
    let status = x.status().map_err(e)?;
    if status == 0 {
        return Err(format!("{method} {url}: no answer (network error or CORS)"));
    }
    let buf = x.response().map_err(e)?;
    let body = if buf.is_null() || buf.is_undefined() { vec![] } else { js_sys::Uint8Array::new(buf.unchecked_ref::<js_sys::ArrayBuffer>()).to_vec() };
    let raw = x.get_all_response_headers().map_err(e)?;
    let headers = raw.split("\r\n").filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    Ok((status, body, headers))
}

pub struct Xhr;

impl Transport for Xhr {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> anyhow::Result<Response> {
        let (status, body, headers) = xhr("POST", url, headers, Some(body)).map_err(|m| anyhow::anyhow!(m))?;
        Ok(Response { status, body, headers })
    }
}

impl HttpTransport for Xhr {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let headers: Vec<(&str, &str)> = r.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let (status, body, _) = xhr(r.method.as_str(), &r.url, &headers, r.body.as_deref())?;
        Ok(HttpResponse::new(status, body))
    }
}

fn transport() -> SharedTransport {
    Rc::new(Xhr)
}

fn parse_cfg(cfg_json: &str) -> Result<TakerConfig, JsError> {
    serde_json::from_str(cfg_json).map_err(|e| JsError::new(&format!("engine configuration: {e}")))
}

fn seed(hex_seed: &str) -> Result<[u8; 32], JsError> {
    hex::decode(hex_seed.trim()).ok().and_then(|v| v.try_into().ok()).ok_or_else(|| JsError::new("the NEAR key seed must be 32 bytes of hex"))
}

fn result<T: serde::Serialize>(r: Result<T, exit::ExitError>) -> String {
    match r {
        Ok(v) => json!({"ok": true, "value": v}).to_string(),
        Err(e) => json!({"ok": false, "error": e}).to_string(),
    }
}

// ---------------------------------------------------------------- free functions

/// Builds the Orchard proving key (and its verifying key) now. Returns milliseconds.
#[wasm_bindgen(js_name = prepareProvingKey)]
pub fn prepare_proving_key() -> f64 {
    let t = js_sys::Date::now();
    wallet::prepare_proving_key();
    js_sys::Date::now() - t
}

/// Zatoshis the user must pay for a swap of `zats` with this largest premium.
#[wasm_bindgen(js_name = requiredDeposit)]
pub fn required_deposit(zats: f64, max_premium: f64) -> f64 {
    crate::session::required_deposit(zats as u64, max_premium as u64) as f64
}

/// The ZIP 317 fee of one transaction with one spend and two outputs (every send of the
/// temporary wallet).
#[wasm_bindgen(js_name = feePerSend)]
pub fn fee_per_send() -> f64 {
    wallet::zip317_fee(1, 2) as f64
}

/// The Zcash chain tip, through the configured backend (for a new wallet's birthday).
#[wasm_bindgen(js_name = zcashTip)]
pub fn zcash_tip(cfg_json: &str) -> Result<f64, JsError> {
    let cfg = parse_cfg(cfg_json)?;
    let mut w = engine::wallet_for(&cfg.chains, transport()).map_err(js)?;
    Ok(w.tip().map_err(js)? as f64)
}

/// True if `ua` is a unified address with an Orchard receiver on `network`
/// ("regtest", "test" or "main"), as the engine's wallet decodes it.
#[wasm_bindgen(js_name = checkAddress)]
pub fn check_address(ua: &str, network: &str) -> Result<bool, JsError> {
    wallet::set_network(network).map_err(js)?;
    Ok(wallet::decode_ua(ua.trim()).is_ok())
}

/// Checks the maker's directory entry in `cfg_json`: its signature by the maker's NEAR
/// key, and that the key is a full-access key of the maker's account at a final block.
#[wasm_bindgen(js_name = verifyMaker)]
pub fn verify_maker(cfg_json: &str) -> Result<(), JsError> {
    let cfg = parse_cfg(cfg_json)?;
    let mut scratch = MemStore::new();
    let t = Taker::open(cfg, transport(), &mut scratch).map_err(js)?;
    t.verify_maker().map_err(js)
}

/// The account a NEAR key seed owns (hex of its public key).
#[wasm_bindgen(js_name = nearAccount)]
pub fn near_account(seed_hex: &str) -> Result<String, JsError> {
    Ok(zecret_core::near::NearKey::from_seed(seed(seed_hex)?).implicit_account())
}

// ---------------------------------------------------------------- the exit

/// Firm 1Click quote, checked against `approval_json`. JSON `{ok, value | error}`.
#[wasm_bindgen(js_name = exitQuote)]
pub fn exit_quote(base_url: &str, approval_json: &str, now_ms: f64) -> String {
    let r = serde_json::from_str::<exit::Approval>(approval_json)
        .map_err(|e| exit::ExitError { code: "INVALID_PARAMS", message: e.to_string(), transient: false, offered: None })
        .and_then(|a| exit::oneclick_quote(&Xhr, &OneClick { base_url: base_url.into(), api_key: None }, &a, now_ms as i64));
    result(r)
}

/// Generate, check, sign and submit the exit for the quote of `exitQuote`.
#[wasm_bindgen(js_name = exitSubmit)]
pub fn exit_submit(base_url: &str, seed_hex: &str, approval_json: &str, quote_raw_json: &str, now_ms: f64) -> Result<String, JsError> {
    let s = seed(seed_hex)?;
    let r = serde_json::from_str::<exit::Approval>(approval_json)
        .map_err(|e| exit::ExitError { code: "INVALID_PARAMS", message: e.to_string(), transient: false, offered: None })
        .and_then(|a| {
            let raw: Value = serde_json::from_str(quote_raw_json).map_err(|e| exit::ExitError { code: "INTERNAL", message: e.to_string(), transient: false, offered: None })?;
            exit::oneclick_submit(&Xhr, &OneClick { base_url: base_url.into(), api_key: None }, &s, &a, &raw, now_ms as i64)
        });
    Ok(result(r))
}

#[wasm_bindgen(js_name = exitStatus)]
pub fn exit_status(base_url: &str, deposit_address: &str) -> String {
    result(exit::oneclick_status(&Xhr, &OneClick { base_url: base_url.into(), api_key: None }, deposit_address))
}

/// Plans the direct exit (local and testnet). JSON `{ok, value: DirectPlan | error}`.
#[wasm_bindgen(js_name = directPrepare)]
pub fn direct_prepare(rpc_url: &str, verifier: &str, token: &str, seed_hex: &str, receiver: &str, now_ms: f64) -> Result<String, JsError> {
    let s = seed(seed_hex)?;
    Ok(result(exit::direct_prepare(&transport(), rpc_url, verifier, token, &s, receiver, now_ms as i64)))
}

// ---------------------------------------------------------------- the taker

/// The taker of one browser. Records cross as `[[key, value], ...]` JSON (UTF-8 text).
#[wasm_bindgen]
pub struct WasmTaker {
    s: Session,
}

#[wasm_bindgen]
impl WasmTaker {
    #[wasm_bindgen(constructor)]
    pub fn new(cfg_json: &str, records_json: &str) -> Result<WasmTaker, JsError> {
        let cfg = parse_cfg(cfg_json)?;
        let records: Vec<(String, String)> = serde_json::from_str(records_json).map_err(|e| JsError::new(&format!("records: {e}")))?;
        Ok(WasmTaker { s: Session::open(cfg, transport(), records).map_err(js)? })
    }

    /// The temporary wallet's unified address.
    pub fn ua(&self) -> String {
        self.s.taker.ua()
    }

    /// Changes the largest premium this taker accepts (the premium the page showed).
    #[wasm_bindgen(js_name = setMaxPremium)]
    pub fn set_max_premium(&mut self, zats: f64) {
        self.s.taker.cfg.max_premium = zats as u64;
    }

    /// Records changed since the last call: `[[key, value], ...]`.
    pub fn dirty(&mut self) -> String {
        serde_json::to_string(&self.s.dirty()).unwrap_or_else(|_| "[]".into())
    }

    pub fn record(&self, key: &str) -> Option<String> {
        self.s.record(key)
    }

    /// Starts the swap; returns the record as JSON.
    pub fn start(&mut self, zats: f64, min_price: &str, now: f64) -> Result<String, JsError> {
        let min: u128 = min_price.parse().map_err(|_| JsError::new("min_price must be a whole number"))?;
        Ok(self.s.start(zats as u64, min, now as u64).map_err(js)?.to_string())
    }

    /// One tick: `{next, events}`.
    pub fn tick(&mut self, now: f64) -> String {
        self.s.tick(now as u64).to_string()
    }

    pub fn reconcile(&mut self, now: f64) -> Result<bool, JsError> {
        self.s.reconcile(now as u64).map_err(js)
    }

    /// The current record (JSON) or undefined.
    pub fn swap(&self) -> Option<String> {
        self.s.swap().map(|v| v.to_string())
    }

    pub fn facts(&self) -> String {
        self.s.facts().to_string()
    }

    pub fn sync(&mut self) -> Result<f64, JsError> {
        Ok(self.s.sync().map_err(js)? as f64)
    }

    #[wasm_bindgen(js_name = txHeight)]
    pub fn tx_height(&mut self, txid: &str) -> Option<f64> {
        self.s.tx_height(txid).map(|h| h as f64)
    }

    #[wasm_bindgen(js_name = escrowState)]
    pub fn escrow_state(&self, escrow_id: &str) -> Result<Option<String>, JsError> {
        self.s.escrow_state(escrow_id).map_err(js)
    }

    #[wasm_bindgen(js_name = intentsBalance)]
    pub fn intents_balance(&self, account: &str) -> Result<String, JsError> {
        Ok(self.s.intents_balance(account).map_err(js)?.to_string())
    }

    #[wasm_bindgen(js_name = returnLeftover)]
    pub fn return_leftover(&mut self, ua: &str) -> Result<String, JsError> {
        self.s.return_leftover(ua).map_err(js)
    }
}

/// Signs the planned direct exit and hands it to the maker's relay.
#[wasm_bindgen(js_name = directRelay)]
pub fn direct_relay(rpc_url: &str, relay_url: &str, seed_hex: &str, plan_json: &str, now_ms: f64) -> Result<String, JsError> {
    let s = seed(seed_hex)?;
    let plan: exit::DirectPlan = serde_json::from_str(plan_json).map_err(|e| JsError::new(&format!("direct exit plan: {e}")))?;
    Ok(result(exit::direct_relay(&transport(), rpc_url, relay_url, &s, &plan, now_ms as i64)))
}
