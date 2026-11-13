#!/bin/sh
# Builds the browser engine into web/public/engine/: zecret_wasm.js (wasm-bindgen glue)
# and zecret_wasm_bg.wasm. Needs Rust with the wasm32-unknown-unknown target and
# wasm-bindgen-cli 0.2.129 (cargo install wasm-bindgen-cli --version =0.2.129 --locked).
set -e
cd "$(dirname "$0")/.."
cargo build -p zecret-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir ../web/public/engine target/wasm32-unknown-unknown/release/zecret_wasm.wasm
ls -l ../web/public/engine/
