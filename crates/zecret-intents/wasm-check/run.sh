#!/bin/sh
# Builds the crate for wasm32-unknown-unknown inside a small cdylib, binds it with
# wasm-bindgen (the CLI version must equal the pinned wasm-bindgen crate, 0.2.129) and
# runs a self-test in Node: quote signature, pre-signing check, signature, a refusal,
# and the browser random source (crypto.getRandomValues) for a key and a nonce.
set -e
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target nodejs --out-dir target/pkg target/wasm32-unknown-unknown/release/intents_wasm_harness.wasm
node -e "
const m = require('./target/pkg/intents_wasm_harness.js');
const fs = require('fs');
const f = fs.readFileSync('../tests/fixtures/exit_raw_ed25519.json', 'utf8');
const a = JSON.parse(m.selftest(f)), b = JSON.parse(m.selftest(f));
console.log(JSON.stringify(a, null, 1));
if (!a.quote_signature_valid || !a.signature_matches_recording) process.exit(1);
if (a.random_key_account === b.random_key_account || a.random_nonce === b.random_nonce) process.exit(1);
console.log('wasm self-test: PASS');
"
