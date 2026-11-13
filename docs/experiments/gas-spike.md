# Gas spike: key-share checks on NEAR

A feasibility test, not a product. It measures whether a NEAR contract can
do the three checks the escrow needs: that a revealed key share `s` matches a
stored public share `S = s·G` on Pallas (Zcash Orchard-protocol keys) or on
ed25519 (Monero keys), and that an ed25519 signature is valid.

Result on a local sandbox, whole transaction, against a 300 Tgas
per-transaction limit:

| Check | Method | Gas (nearcore 2.14.1) | Gas (nearcore 2.9.0, lowered wasm) |
|---|---|---|---|
| ed25519 share | `check_share` | 5.1 Tgas | 8.9 Tgas |
| Pallas share | `check_share_pallas` | 16.2 Tgas | 16.5 Tgas |
| ed25519 signature (native host function) | `check_signature` | 1.8 Tgas | 2.0 Tgas |

- `escrow/` — the contract with the three check methods.
- `gastest/` — sandbox tests that call them and print the gas burnt.

## Run

```sh
cd escrow && cargo build --target wasm32-unknown-unknown --release
```

Mainnet and testnet ran nearcore 2.14.1 on 2026-11-12. A sandbox of that version
accepts the wasm as built, so skip the next step and point `ESCROW_WASM` at the
build output. An older sandbox (2.9.0) rejects WebAssembly features that recent
Rust emits; for that case only, lower them with binaryen's `wasm-opt`:

```sh
F="--enable-mutable-globals --enable-nontrapping-float-to-int --enable-bulk-memory --enable-sign-ext --enable-reference-types --enable-multivalue --enable-bulk-memory-opt --enable-call-indirect-overlong"
wasm-opt $F --signext-lowering --llvm-memory-copy-fill-lowering --llvm-nontrapping-fptoint-lowering -Oz \
  target/wasm32-unknown-unknown/release/escrow.wasm -o /tmp/escrow.lowered.wasm
wasm-opt --mvp-features --enable-mutable-globals /tmp/escrow.lowered.wasm -o /tmp/escrow.mvp.wasm
```

```sh
cd ../gastest && ESCROW_WASM=/tmp/escrow.mvp.wasm cargo test --test gas -- --nocapture
```

The sandbox binary is downloaded by `near-workspaces`. Where that download is
blocked, fetch it by hand and set `NEAR_SANDBOX_BIN_PATH`. In a container that caps
open files below 65535 the sandbox refuses to start; a small `LD_PRELOAD` shim that
reports success for the `RLIMIT_NOFILE` request gets past that for local tests.
