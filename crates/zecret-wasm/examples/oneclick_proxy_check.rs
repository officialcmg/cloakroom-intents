//! Read-only check of a 1Click base URL, such as the site's proxy (docs/ONECLICK_PROXY.md):
//! the token list, a dry quote whose 1Click signature the engine verifies, and with `--firm`
//! a firm quote for a throwaway key that goes through the engine's own quote check
//! (`zecret_wasm::exit::oneclick_quote`, the function the browser calls). Nothing is signed
//! or sent to NEAR; the throwaway key holds nothing, so the firm quote can never execute.
//!
//! ```sh
//! cargo run -p zecret-wasm --example oneclick_proxy_check -- https://<site>/api/oneclick --firm
//! ```
//!
//! It prints the `x-oneclick-auth` header of each answer when the base URL is the proxy.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use zecret_intents::{
    HttpRequest, HttpResponse, HttpTransport, IntentSigner, Method,
    client::ExitClient,
    oneclick::{ExitQuoteParams, OneClick, USDC_NEAR_ASSET},
    time,
};
use zecret_wasm::exit::{Approval, oneclick_quote};

/// USDC on Base: the site's default destination.
const DEST: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
const RECIPIENT: &str = "0x1111111111111111111111111111111111111111";
const AMOUNT: u128 = 5_000_000;

struct Native(reqwest::blocking::Client);

impl HttpTransport for Native {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut req = match r.method {
            Method::Get => self.0.get(&r.url),
            Method::Post => self.0.post(&r.url).body(r.body.clone().unwrap_or_default()),
        };
        for (k, v) in &r.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().map_err(|e| format!("{} {}: {e}", r.method.as_str(), r.url))?;
        let auth = resp.headers().get("x-oneclick-auth").and_then(|v| v.to_str().ok()).unwrap_or("(no header: not the proxy)").to_owned();
        let status = resp.status().as_u16();
        println!("  {} {} -> {status}, x-oneclick-auth: {auth}", r.method.as_str(), r.url);
        Ok(HttpResponse::new(status, resp.bytes().map_err(|e| e.to_string())?.to_vec()))
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(base) = args.iter().find(|a| !a.starts_with("--")) else {
        bail!("usage: oneclick_proxy_check <base URL> [--firm]");
    };
    let firm = args.iter().any(|a| a == "--firm");
    let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60)).build()?;
    let api = OneClick { base_url: base.clone(), api_key: None };
    let c = ExitClient::new(Native(client), api.clone());

    println!("1. tokens");
    let tokens = c.tokens().context("token list")?;
    let usdc = tokens.iter().any(|t| t.asset_id == USDC_NEAR_ASSET);
    println!("   {} tokens, USDC on NEAR listed: {usdc}", tokens.len());
    if !usdc {
        bail!("USDC on NEAR is missing from the token list");
    }

    let signer = IntentSigner::generate()?;
    let now = time::from_unix_millis(now_ms())?;
    println!("2. dry quote, 5 USDC on NEAR to USDC on Base (signature checked by the engine's verifier)");
    let q = c
        .quote(&ExitQuoteParams {
            signer_id: signer.account_id(),
            origin_asset: USDC_NEAR_ASSET.into(),
            amount: AMOUNT,
            destination_asset: DEST.into(),
            recipient: RECIPIENT.into(),
            slippage_bps: 100,
            deadline: now + chrono::Duration::minutes(30),
            dry: true,
            referral: None,
            quote_waiting_time_ms: None,
        })
        .context("dry quote")?;
    println!("   signed by 1Click: amountOut {} minAmountOut {} appFees {} bp", q.quote.amount_out, q.quote.min_amount_out, q.quote_request.app_fee_bps());

    if firm {
        println!("3. firm quote for a throwaway key through the engine's oneclick_quote (quote check against an approval)");
        let min: u128 = q.quote.min_amount_out.parse()?;
        let approval = Approval {
            signer_id: signer.account_id(),
            verifying_contract: "intents.near".into(),
            origin_asset: USDC_NEAR_ASSET.into(),
            amount_in: AMOUNT.to_string(),
            destination_asset: DEST.into(),
            recipient: RECIPIENT.into(),
            // The approval the page would ask for: 2 % under the dry minimum.
            min_amount_out: (min * 98 / 100).to_string(),
        };
        let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60)).build()?;
        match oneclick_quote(&Native(client), &api, &approval, now_ms()) {
            Ok(f) => println!("   accepted by the engine's check: deposit address {}, minAmountOut {}, fee {} bp", f.deposit_address, f.min_amount_out, f.fee_bps),
            Err(e) => bail!("the engine refused the firm quote: {} {}", e.code, e.message),
        }
    }
    println!("PASS");
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_millis() as i64
}
