#!/bin/sh
# Build the wasm package for the page and the native helper tools.
set -e
cd "$(dirname "$0")"
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg target/wasm32-unknown-unknown/release/browser_wallet.wasm
cargo build --release --bins
ls -l web/pkg/browser_wallet_bg.wasm
