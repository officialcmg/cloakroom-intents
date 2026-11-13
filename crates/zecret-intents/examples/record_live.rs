//! Runs the exit flow against the live 1Click API with an unfunded key and records
//! every request and response as a fixture for `tests/recorded.rs`.
//!
//! Nothing is spent: the signer `5a5a…5a` (seed of 32 bytes 0x5a) holds no tokens, so
//! the submitted intents cannot execute. Uses `curl` as the transport.
//!
//! ```sh
//! cargo run --example record_live -- tests/fixtures
//! ```

use std::cell::RefCell;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use zecret_intents::oneclick::USDC_NEAR_ASSET;
use zecret_intents::{
    ApprovedExit, ExitClient, ExitQuoteParams, HttpRequest, HttpResponse, HttpTransport,
    IntentSigner, OneClick, Standard, time,
};

const USDC_BASE: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
const RECIPIENT_EVM: &str = "0x1111111111111111111111111111111111111111";

struct Curl {
    log: RefCell<Vec<Value>>,
}

impl HttpTransport for Curl {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut c = Command::new("curl");
        c.args(["-sS", "-X", r.method.as_str(), "-w", "\n%{http_code}"]);
        for (k, v) in &r.headers {
            c.args(["-H", &format!("{k}: {v}")]);
        }
        if r.body.is_some() {
            c.args(["--data-binary", "@-"]);
        }
        c.arg(&r.url);
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().map_err(|e| e.to_string())?;
        if let Some(b) = &r.body {
            child
                .stdin
                .take()
                .ok_or("no stdin")?
                .write_all(b)
                .map_err(|e| e.to_string())?;
        } else {
            drop(child.stdin.take());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').ok_or("no status line")?;
        let status: u16 = code.trim().parse().map_err(|e| format!("{e}"))?;
        let resp = HttpResponse::new(status, body.as_bytes().to_vec());
        self.log.borrow_mut().push(json!({
            "request": {
                "method": r.method.as_str(),
                "url": r.url,
                "headers": r.headers.iter().filter(|(k, _)| k != "x-api-key").map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                "body": r.json_body(),
            },
            "response": {
                "status": status,
                "body": serde_json::from_str::<Value>(body).unwrap_or(Value::String(body.into())),
            }
        }));
        Ok(resp)
    }
}

fn now() -> zecret_intents::time::Time {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64;
    // Whole seconds, so that a recorded run can be replayed with the same value.
    time::from_unix_millis(ms - ms % 1000).expect("time")
}

fn save(dir: &str, name: &str, now: zecret_intents::time::Time, t: &Curl, extra: Value) {
    let v = json!({
        "recorded_at": time::format(now),
        "notes": extra,
        "exchanges": t.log.borrow().clone(),
    });
    std::fs::write(
        format!("{dir}/{name}.json"),
        serde_json::to_string_pretty(&v).expect("json") + "\n",
    )
    .expect("write fixture");
    t.log.borrow_mut().clear();
    println!("saved {dir}/{name}.json");
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/fixtures".into());
    let transport = Curl {
        log: RefCell::new(Vec::new()),
    };
    let client = ExitClient::new(&transport, OneClick::default());
    let signer = IntentSigner::from_seed(&[0x5a; 32]);
    let me = signer.account_id();

    // Tokens.
    let tokens = client.tokens().expect("tokens");
    println!("{} tokens", tokens.len());
    save(&dir, "tokens", now(), &transport, json!({}));

    // A dry quote to show the price, then the signed exit with raw_ed25519.
    for (name, standard) in [
        ("exit_raw_ed25519", Standard::RawEd25519),
        ("exit_nep413", Standard::Nep413),
    ] {
        let t0 = now();
        let deadline = t0 + chrono::Duration::minutes(30);
        let dry = client
            .quote(&ExitQuoteParams {
                signer_id: me.clone(),
                origin_asset: USDC_NEAR_ASSET.into(),
                amount: 5_000_000,
                destination_asset: USDC_BASE.into(),
                recipient: RECIPIENT_EVM.into(),
                slippage_bps: 100,
                deadline,
                dry: true,
                referral: None,
                quote_waiting_time_ms: None,
            })
            .expect("dry quote");
        println!(
            "dry: out {} min {}",
            dry.quote.amount_out, dry.quote.min_amount_out
        );
        // The user approves the dry quote's minimum less 0.5 % of headroom.
        let min: u128 = dry.quote.min_amount_out.parse().expect("min");
        let approved = ApprovedExit::mainnet(
            me.clone(),
            USDC_NEAR_ASSET,
            5_000_000,
            USDC_BASE,
            RECIPIENT_EVM,
            min - min / 200,
        );
        let submitted = client
            .execute_exit(&signer, &approved, 100, deadline, standard, t0)
            .expect("execute_exit");
        println!(
            "{name}: deposit {} hash {}",
            submitted.deposit_address(),
            submitted.intent_hash
        );
        // Poll status until it is final or 60 s have passed.
        let mut last = None;
        for _ in 0..12 {
            std::thread::sleep(Duration::from_secs(5));
            let s = client.status(submitted.deposit_address()).expect("status");
            let fin = s.status.is_final();
            println!("status {:?}", s.status);
            last = Some(s.status);
            if fin {
                break;
            }
        }
        save(
            &dir,
            name,
            t0,
            &transport,
            json!({
                "approved_min_amount_out": approved.min_amount_out.to_string(),
                "quote_deadline": time::format(deadline),
                "signer_seed": "32 bytes of 0x5a",
                "final_status": format!("{last:?}"),
            }),
        );
    }

    // Error answers.
    let t0 = now();
    let base = ExitQuoteParams {
        signer_id: me.clone(),
        origin_asset: USDC_NEAR_ASSET.into(),
        amount: 5_000_000,
        destination_asset: USDC_BASE.into(),
        recipient: RECIPIENT_EVM.into(),
        slippage_bps: 100,
        deadline: t0 + chrono::Duration::minutes(30),
        dry: true,
        referral: None,
        quote_waiting_time_ms: None,
    };
    let cases = [
        ExitQuoteParams {
            deadline: t0 - chrono::Duration::minutes(1),
            ..base.clone()
        },
        ExitQuoteParams {
            recipient: "0x000000000000000000000000000000000000dEaD".into(),
            ..base.clone()
        },
        ExitQuoteParams {
            destination_asset: "nep141:btc.omft.near".into(),
            recipient: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".into(),
            ..base.clone()
        },
    ];
    for c in &cases {
        let e = client.quote(c).expect_err("should fail");
        println!("error: {e}");
    }
    let e = client
        .status("0000000000000000000000000000000000000000000000000000000000000001")
        .expect_err("unknown");
    println!("error: {e}");
    let bad_key = ExitClient::new(&transport, OneClick::with_api_key("not-a-real-key"));
    let e = bad_key.quote(&base).expect_err("bad key");
    println!("error: {e}");
    save(&dir, "errors", t0, &transport, json!({}));
}
