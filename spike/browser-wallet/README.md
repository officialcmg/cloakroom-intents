# Browser wallet spike

The Zcash light wallet from `node/src/wallet.rs`, compiled to WebAssembly and run in a
Web Worker in a browser. It creates a wallet, receives a shielded payment with a memo,
and sends a fully shielded transaction with a 512-byte memo. Results and the list of
changes are in `docs/experiments/browser-wallet.md`.

## Files

| Path | What it is |
|---|---|
| `src/wallet.rs` | Copy of `node/src/wallet.rs`, adapted for the browser. Builds natively and for `wasm32-unknown-unknown`. |
| `src/keys.rs` | Copy of `node/src/keys.rs`, unchanged. |
| `src/backend.rs` | Zebra JSON-RPC and lightwalletd gRPC-web clients over `net::Transport`. |
| `src/net.rs` | The network interface: one blocking POST. `reqwest` natively, synchronous `XMLHttpRequest` in a worker. |
| `src/web.rs` | The `wasm-bindgen` interface that the worker calls. |
| `src/lib.rs` | `Me` (wallet keys, from `node/src/common.rs` without files), memo text, unit tests. |
| `src/bin/sendmemo.rs` | Native build of the same wallet: a shielded send with a text memo. The check uses it to pay the browser wallet. |
| `src/bin/memocheck.rs` | Independent check of a mined transaction: shape, and trial decryption with orchard directly (not the wallet code). |
| `web/` | The page (`index.html`, `page.js`) and the worker (`worker.js`). `web/pkg/` is build output. |
| `proxy.mjs` | CORS proxy in front of Zebra's JSON-RPC, for local development. |
| `serve.mjs` | Static server for `web/`. |
| `run.mjs` | The check against local regtest. Exits 0 on success. |
| `testnet.mjs` | Read-only check of the gRPC-web backend against Zcash testnet. |
| `keybench.mjs` | Proving key cost across page loads. |

## Build

Requirements: Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` at the
version of the `wasm-bindgen` crate in `Cargo.lock`, Node 22, Playwright with Chromium,
Docker with the image `zfnd/zebra:7.0.0-rc.0`.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version =0.2.129 --locked
cd spike/browser-wallet
./build.sh                      # wasm package into web/pkg, native tools into target/release
cargo test --release --lib      # unit tests (native)
(cd ../../node && cargo build)  # the node/ client; run.mjs uses its `init` and `balance`
```

`.cargo/config.toml` sets `--cfg getrandom_backend="wasm_js"` for the wasm target.

## Run the check

```sh
node run.mjs
```

It starts Zebra regtest with `harness/zebra/docker-compose.yml` and `zebrad-nu7.toml`
(and stops it at the end), the CORS proxy on port 18300 and the page on port 18380, then
drives headless Chromium. Environment variables:

| Variable | Default | Effect |
|---|---|---|
| `CLOAKROOM_BIN` | `node/target/debug/cloakroom` | The `node/` client binary. |
| `ZEBRA_RPC` | `http://127.0.0.1:18232` | Zebra JSON-RPC. |
| `ZEBRA_EXTERNAL` | unset | Set to use a node that is already running; Docker is not touched. |
| `KEEP_ZEBRA` | unset | Set to leave the Docker node running at the end. |
| `VERBOSE` | unset | Print the page's console. |

## Other checks

```sh
node testnet.mjs https://zcash-testnet.chainsafe.dev 1000   # read-only; uses HTTPS_PROXY if set
node keybench.mjs 2                                         # key build on two page loads
```

## Use the page by hand

```sh
ZEBRA_CONF=zebrad-nu7.toml docker compose -f ../../harness/zebra/docker-compose.yml up -d
node proxy.mjs 18300 http://127.0.0.1:18232 &
node serve.mjs 8080 &
# open http://127.0.0.1:8080/?rpc=http://127.0.0.1:18300
```
