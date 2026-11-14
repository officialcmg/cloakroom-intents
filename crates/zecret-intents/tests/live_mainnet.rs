//! Read-only checks against NEAR mainnet and the live 1Click API. Nothing is spent and
//! nothing is submitted.
//!
//! Opt-in: the tests do nothing unless `ZECRET_LIVE_MAINNET_READONLY=1`.
//!
//! ```sh
//! cd crates/zecret-intents
//! ZECRET_LIVE_MAINNET_READONLY=1 cargo test --test live_mainnet -- --nocapture --test-threads 1
//! ```
//!
//! What they prove, with a fresh key that holds nothing:
//!
//! - **Format.** Every payload this crate can sign (`transfer` and `ft_withdraw`, with
//!   `raw_ed25519` and `nep413`) passes the mainnet Verifier's checks up to the balance:
//!   `simulate_intents` on `intents.near` fails with "insufficient balance or
//!   overflow". The Verifier checks the signature, the payload format, the verifying
//!   contract, the deadline, the public key and the nonce (salt, nonce deadline) before
//!   it touches a balance (`contracts/defuse/core/src/engine/mod.rs` in `near/intents`),
//!   so that error means every earlier check passed.
//! - **Controls.** The same payload with a wrong salt fails with "invalid salt", and with
//!   a changed signature with "invalid signature". So the simulation does tell these
//!   errors apart.
//! - **The 1Click exit payload.** One firm quote (USDC on NEAR to USDC on Base, refund to
//!   the fresh key): the quote check passes, `/v0/generate-intent` for both standards
//!   passes the pre-signing check, and the signed payload fails in `simulate_intents`
//!   only for the balance. `/v0/submit-intent` is not called. A firm quote creates a
//!   deposit address and nothing else.
//!
//! Optional environment: `ZECRET_MAINNET_RPC` (default `https://rpc.mainnet.fastnear.com`),
//! `ZECRET_ONECLICK_URL`, `ZECRET_LIVE_EVIDENCE` (a file to write the answers to),
//! `ZECRET_LIVE_SKIP_FIRM_QUOTE=1` (only the direct payloads and a dry quote).
#![cfg(not(target_arch = "wasm32"))]

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Duration;
use serde_json::{Value, json};
use zecret_intents::direct::{self, execute_intents_args};
use zecret_intents::oneclick::{self, USDC_NEAR_ASSET, VERIFIER_MAINNET};
use zecret_intents::time::{self, Time};
use zecret_intents::{
    ApprovedExit, Error, ExitClient, ExitQuoteParams, HttpRequest, HttpResponse, HttpTransport,
    IntentDraft, IntentSigner, Nonce, OneClick, Policy, Salt, SignedPayload, Standard, VerifierRpc,
    check_generated_intent, check_quote, check_status,
};

const USDC_TOKEN: &str = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const USDC_BASE: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
const RECIPIENT_EVM: &str = "0x1111111111111111111111111111111111111111";
/// The Verifier's error for a balance that does not cover the intent.
const INSUFFICIENT: &str = "insufficient balance or overflow";

fn enabled() -> bool {
    if std::env::var("ZECRET_LIVE_MAINNET_READONLY").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped: set ZECRET_LIVE_MAINNET_READONLY=1 to run the read-only mainnet checks");
    false
}

fn rpc_url() -> String {
    std::env::var("ZECRET_MAINNET_RPC")
        .unwrap_or_else(|_| "https://rpc.mainnet.fastnear.com".into())
}

/// A blocking transport over `curl` (the crate does no I/O itself).
struct Curl;

impl HttpTransport for Curl {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut c = Command::new("curl");
        c.args([
            "-sS",
            "--max-time",
            "60",
            "-X",
            r.method.as_str(),
            "-w",
            "\n%{http_code}",
        ]);
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
        let mut stdin = child.stdin.take().ok_or("stdin")?;
        if let Some(b) = &r.body {
            stdin.write_all(b).map_err(|e| e.to_string())?;
        }
        drop(stdin);
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').ok_or("no status")?;
        Ok(HttpResponse::new(
            code.trim().parse::<u16>().map_err(|e| e.to_string())?,
            body.as_bytes().to_vec(),
        ))
    }
}

fn now() -> Time {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64;
    time::from_unix_millis(ms - ms % 1000).expect("time")
}

static EVIDENCE: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn record(what: &str, v: Value) {
    println!("{what}: {v}");
    EVIDENCE
        .lock()
        .unwrap()
        .push(json!({ "what": what, "at": time::format(now()), "value": v }));
    if let Ok(path) = std::env::var("ZECRET_LIVE_EVIDENCE") {
        let all = EVIDENCE.lock().unwrap().clone();
        let _ = std::fs::write(path, serde_json::to_string_pretty(&all).unwrap() + "\n");
    }
}

fn verifier() -> VerifierRpc<Curl> {
    VerifierRpc {
        transport: Curl,
        rpc_url: rpc_url(),
        verifier: VERIFIER_MAINNET.into(),
    }
}

/// `simulate_intents` for one signed payload; returns the Verifier's error text, or
/// panics if the simulation succeeded (an unfunded key must not be able to move tokens).
fn simulate_error(signed_json: Value) -> String {
    let r = Curl
        .send(&direct::view_call_request(
            &rpc_url(),
            VERIFIER_MAINNET,
            "simulate_intents",
            &json!({ "signed": [signed_json] }),
        ))
        .expect("NEAR RPC");
    match direct::parse_view_result::<Value>(&r) {
        Ok(v) => panic!("simulate_intents succeeded for an unfunded key: {v}"),
        Err(Error::Rpc(m)) => m,
        Err(e) => panic!("unexpected answer from NEAR RPC: {e}"),
    }
}

fn assert_only_balance_fails(what: &str, signed: &SignedPayload) {
    let err = simulate_error(signed.to_json());
    record(
        what,
        json!({ "payload": signed.to_json(), "simulate_intents": err }),
    );
    assert!(
        err.contains(INSUFFICIENT),
        "{what}: expected {INSUFFICIENT:?}, got {err}"
    );
}

#[test]
fn direct_payloads_pass_every_mainnet_verifier_check_but_the_balance() {
    if !enabled() {
        return;
    }
    let v = verifier();
    let salt = v.current_salt().expect("current_salt");
    record("current_salt", json!(salt.to_hex()));
    let signer = IntentSigner::generate().expect("key");
    let me = signer.account_id();
    assert_eq!(v.balance(&me, USDC_NEAR_ASSET).expect("mt_balance_of"), 0);
    let other = IntentSigner::generate().expect("key").account_id();
    let t = now();
    let deadline = t + Duration::minutes(10);
    let draft = |nonce| IntentDraft::new(&me, VERIFIER_MAINNET, deadline, nonce);
    for standard in [Standard::RawEd25519, Standard::Nep413] {
        let n = || Nonce::versioned_random(salt, deadline).expect("nonce");
        let transfer = draft(n())
            .transfer(&other, USDC_NEAR_ASSET, 1_000_000)
            .sign(&signer, standard, t)
            .unwrap();
        assert_only_balance_fails(&format!("transfer {}", standard.as_str()), &transfer);
        let withdraw = draft(n())
            .ft_withdraw(USDC_TOKEN, "zecret-readonly-check.near", 1_000_000)
            .sign(&signer, standard, t)
            .unwrap();
        assert_only_balance_fails(&format!("ft_withdraw {}", standard.as_str()), &withdraw);

        // Controls: the simulation tells a format error from a balance error.
        let bad_salt = draft(Nonce::versioned_random(Salt([0, 0, 0, 0]), deadline).unwrap())
            .transfer(&other, USDC_NEAR_ASSET, 1_000_000)
            .sign(&signer, standard, t)
            .unwrap();
        let err = simulate_error(bad_salt.to_json());
        record(
            &format!("control wrong salt {}", standard.as_str()),
            json!(err),
        );
        assert!(err.contains("invalid salt"), "{err}");

        let mut forged = transfer.to_json();
        forged["signature"] = json!(
            IntentSigner::generate()
                .unwrap()
                .sign(transfer.unsigned())
                .unwrap()
                .signature_str()
        );
        let err = simulate_error(forged);
        record(
            &format!("control other key's signature {}", standard.as_str()),
            json!(err),
        );
        assert!(err.contains("invalid signature"), "{err}");
    }
    // The execute_intents arguments are the simulate_intents arguments.
    let one = draft(Nonce::versioned_random(salt, deadline).unwrap())
        .transfer(&other, USDC_NEAR_ASSET, 1)
        .sign(&signer, Standard::RawEd25519, t)
        .unwrap();
    assert_eq!(
        execute_intents_args(std::slice::from_ref(&one))["signed"][0],
        one.to_json()
    );
}

#[test]
fn the_oneclick_exit_payload_passes_every_mainnet_verifier_check_but_the_balance() {
    if !enabled() {
        return;
    }
    let api = OneClick {
        base_url: std::env::var("ZECRET_ONECLICK_URL")
            .unwrap_or_else(|_| oneclick::DEFAULT_BASE_URL.into()),
        api_key: None,
    };
    let client = ExitClient::new(Curl, api.clone());
    let signer = IntentSigner::generate().expect("key");
    let me = signer.account_id();
    let t = now();
    let params = |dry| ExitQuoteParams {
        signer_id: me.clone(),
        origin_asset: USDC_NEAR_ASSET.into(),
        amount: 2_000_000,
        destination_asset: USDC_BASE.into(),
        recipient: RECIPIENT_EVM.into(),
        slippage_bps: 100,
        deadline: t + Duration::minutes(30),
        dry,
        referral: None,
        quote_waiting_time_ms: None,
    };
    let dry = client
        .quote(&params(true))
        .expect("dry quote (signature checked)");
    record("dry quote", dry.raw.clone());
    if std::env::var("ZECRET_LIVE_SKIP_FIRM_QUOTE").as_deref() == Ok("1") {
        return;
    }
    let firm = client
        .quote(&params(false))
        .expect("firm quote (signature checked)");
    record("firm quote", firm.raw.clone());
    let min: u128 = dry.quote.min_amount_out.parse().unwrap();
    let approved = ApprovedExit::mainnet(
        &me,
        USDC_NEAR_ASSET,
        2_000_000,
        USDC_BASE,
        RECIPIENT_EVM,
        min - min / 100,
    );
    check_quote(&approved, &firm, &Policy::default(), now())
        .expect("the firm quote passes the check");
    let deposit = firm.quote.deposit_address.clone().unwrap();
    for standard in [Standard::RawEd25519, Standard::Nep413] {
        let r = Curl
            .send(&api.generate_intent_request(&deposit, &me, standard))
            .expect("generate-intent");
        let generated = oneclick::parse_generated_intent(&r).expect("generate-intent answer");
        record(
            &format!("generate-intent {}", standard.as_str()),
            generated.payload.clone(),
        );
        let checked = check_generated_intent(
            &approved,
            &firm,
            &generated,
            standard,
            &Policy::default(),
            now(),
        )
        .expect("the payload passes the pre-signing check");
        let signed = signer.sign(&checked.payload).unwrap();
        assert_only_balance_fails(&format!("1Click exit {}", standard.as_str()), &signed);
    }
    // The status of the unfunded deposit address carries 1Click's signed quote for it.
    let status = client.status(&deposit).expect("status");
    record(
        "status",
        json!({ "status": format!("{:?}", status.status), "updatedAt": status.updated_at }),
    );
    check_status(&status, &deposit, &Policy::default())
        .expect("the status answer carries the signed quote");
}
