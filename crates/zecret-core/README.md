# zecret-core

The code both parties of a Cloakroom Intents swap share. It has no I/O of its own and builds
natively and for `wasm32-unknown-unknown`.

| Module | Contents |
|---|---|
| `keys` | Pallas key shares, Schnorr proofs of knowledge, the joint Ironwood key, the escrow salt and the maker's directory digest. |
| `msg` | The three encrypted memo messages (request, quote, acceptance) and the funding marker. |
| `wallet`, `zcash` | The Ironwood light wallet (scan, trial decryption, tree, PCZT build, proof, signature) over Zebra JSON-RPC or lightwalletd gRPC-web. |
| `near` | NEAR transactions (borsh, sha256, ed25519), JSON-RPC (`block`, `query`, `send_tx`, `tx`) and the escrow, token and `intents.near` calls. |
| `config` | Timing bounds (D, F, R, ∆fund, k_Z) and each party's settings. |
| `engine` | `Tick`, `Next`, the events, and Zcash transactions waiting for broadcast. |
| `taker`, `maker` | The two state machines with the wallet rules of the Cloakroom paper (R1, R1q, R1b, R2c, R2r, R3e, R3d, R3l, R4, L_M). |
| `net`, `store` | The interfaces to the outside: `Transport` (one blocking POST) and `Store` (key-value records). |

The interface for a host (a Web Worker or the native programs) is in
[`docs/ENGINE.md`](../../docs/ENGINE.md).

## Origin

`keys`, `msg`, `wallet` and the two state machines come from the Cloakroom client
(`node/` at the baseline commit `2f8e2f7`, branch `pre-hackathon-baseline`). `wallet` and `zcash` include the browser changes
from `spike/browser-wallet` (gRPC-web client, `CoverWait`, the verifying key taken from the
proving key). Changes for NEAR: the taker's ed25519 key replaces the EVM payout address;
the maker signs quotes and its directory entry with its NEAR key; there is no relayer and
no relayer fee; the maker sends a NEAR stipend after `open`; key shares and points use the
escrow's encodings (32-byte little-endian scalars, compressed points). New: transactions
are built and broadcast in two steps so a record can be stored in between; the state
machines run in `tick(now)` steps and never sleep.

## Build and test

```sh
cd crates
cargo test -p zecret-core                                    # unit tests
cargo build -p zecret-core --target wasm32-unknown-unknown --release
cargo clippy -p zecret-core --all-targets
```

The wasm build needs the `getrandom` cfg in `crates/.cargo/config.toml` (it is there) and
uses `orchard` and `zcash_primitives` without `multicore`. The NEAR code is tested against
a NEAR sandbox in `crates/zecret-node/tests/sandbox.rs`; the full protocol in
`harness/run.py`.
