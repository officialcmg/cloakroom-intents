//! Self-test of zecret-intents compiled to WebAssembly (see run.sh).

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zecret_intents::check::Policy;
use zecret_intents::oneclick::{GeneratedIntent, USDC_NEAR_ASSET};
use zecret_intents::*;

#[wasm_bindgen]
pub fn selftest(fixture: &str) -> String {
    let f: Value = serde_json::from_str(fixture).unwrap();
    let ex = f["exchanges"].as_array().unwrap();
    let now = time::parse(f["recorded_at"].as_str().unwrap()).unwrap();
    let quote = QuoteResponse::from_value(ex[1]["response"]["body"].clone()).unwrap();
    let g = &ex[2]["response"]["body"]["intent"];
    let generated = GeneratedIntent {
        standard: g["standard"].as_str().unwrap().into(),
        payload: g["payload"].clone(),
        correlation_id: None,
    };
    let signer = IntentSigner::from_seed(&[0x5a; 32]);
    let approved = ApprovedExit::mainnet(
        signer.account_id(),
        USDC_NEAR_ASSET,
        5_000_000,
        "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near",
        "0x1111111111111111111111111111111111111111",
        1,
    );
    let checked = check_generated_intent(
        &approved,
        &quote,
        &generated,
        Standard::RawEd25519,
        &Policy::default(),
        now,
    )
    .unwrap();
    let signed = signer.sign(&checked.payload).unwrap();
    let recorded_sig = ex[3]["request"]["body"]["signedData"]["signature"].clone();
    let mut bad = approved.clone();
    bad.recipient = "0x2222222222222222222222222222222222222222".into();
    let refusal = check_generated_intent(
        &bad,
        &quote,
        &generated,
        Standard::RawEd25519,
        &Policy::default(),
        now,
    )
    .unwrap_err();
    let fresh = IntentSigner::generate().unwrap();
    let nonce = Nonce::versioned_random(Salt([1, 2, 3, 4]), now).unwrap();
    json!({
        "quote_signature_valid": quote.signature_is_valid(),
        "signature_matches_recording": signed.to_json()["signature"] == recorded_sig,
        "intent_hash": checked.intent_hash,
        "refusal": refusal.to_string(),
        "random_key_account": fresh.account_id(),
        "random_nonce": nonce.to_base64(),
    })
    .to_string()
}
