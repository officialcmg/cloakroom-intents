# Browser wallet: can the Zcash wallet code run in a browser worker?

**Question.** The taker engine must hold a temporary shielded wallet in the browser. Can
the wallet code in `node/src/wallet.rs` (key derivation, block scanning, trial decryption,
note commitment tree and witnesses, PCZT build, proof, signature, broadcast) run as
WebAssembly in a Web Worker, and what has to change?

**Result: yes.** Tested 2026-11-13 in headless Chromium against a local Zebra 7.0.0-rc.0
regtest node. One page, with the wallet in a Web Worker:

1. creates a wallet and shows its shielded address;
2. receives a shielded payment from a native wallet and shows the amount (250,000 zats)
   and the decrypted memo;
3. builds, proves and broadcasts a fully shielded transaction with a 512-byte memo to a
   second address;
4. the transaction is mined. Zebra reports no transparent, Sapling or Orchard parts and 2
   Ironwood actions. `memocheck` (orchard trial decryption, not the wallet code) decrypts
   the value and the exact 512 bytes of the memo with the recipient's key. The `node/`
   client (`cloakroom balance`) sees the 100,000 zats at the recipient;
5. after a page reload, the wallet comes back from IndexedDB with the same balance.

The check is `spike/browser-wallet/run.mjs`. It passed on both runs. The read-only
lightwalletd path (gRPC-web, Zcash testnet) also works with the same wallet code.

## Timings

Headless Chromium, one worker thread, this container. "Run 1" and "run 2" are the two
runs of `run.mjs`.

| Step | Run 1 | Run 2 | Notes |
|---|---|---|---|
| wasm download and instantiation | 0.1 s | 0.1 s | 7.6 MB `.wasm`, 1.56 MB gzipped, no `wasm-opt` |
| Proving key build | 22.7 s | 20.1 s | Runs while the native funder proves on the same machine. Alone: 19.4 s and 19.8 s (`keybench.mjs`). |
| Scan to the tip (regtest, 5 blocks) | 0.1 s | 0.1 s | Zebra JSON-RPC through the CORS proxy |
| Send: sync, tree and witness, PCZT build | 0.15 s | 0.15 s | |
| Send: proof (2 actions) | 28.5 s | 26.3 s | |
| Send: sign, extract and verify, broadcast | 0.4 s | 0.4 s | Extraction verifies the proof |
| Send total | 29.1 s | 26.9 s | |
| Page loaded to all checks done | 52.6 s | 47.6 s | Includes the native funding send and mining |
| Testnet scan, 5,000 blocks | 9.5 s | 10.5 s | 8,807 Ironwood actions trial-decrypted with 2 keys, over gRPC-web (`testnet.mjs`) |

## What had to change

`src/keys.rs` is a byte-for-byte copy. `src/wallet.rs` is a copy of `node/src/wallet.rs`
with the changes below. Scanning, trial decryption, the tree, witnesses, the PCZT roles,
proving, signing and extraction needed no change.

| # | Change | Why |
|---|---|---|
| 1 | `orchard` and `zcash_primitives` without `multicore` (`default-features = false`; `orchard` with `circuit, std, zeroize`; `zcash_primitives` with `std, circuits`). | The multithreaded prover cannot run on `wasm32-unknown-unknown`. |
| 2 | `zcash_client_backend` without `lightwalletd-tonic*` features (`default-features = false`, `orchard`). | Only its protobuf message types are used. tonic, tokio, hyper and rustls do not build for the browser and are not needed. |
| 3 | `getrandom` backends: 0.2 with `js`, 0.3 with `wasm_js` plus `--cfg getrandom_backend="wasm_js"` (in `.cargo/config.toml`), 0.4 with `wasm_js`. All three majors are in the tree. | No OS random source in the browser. `OsRng` (rand 0.10 `SysRng`) then works unchanged. |
| 4 | `Rpc` takes a `Transport` (one blocking `post`) instead of `reqwest::blocking`. Natively it is `reqwest`; in the worker it is a synchronous `XMLHttpRequest`. | No blocking sockets in wasm. Browsers allow synchronous XHR in workers (not on the main thread), so the blocking control flow of the wallet stays the same. |
| 5 | `Lwd` is a hand-framed gRPC-web client over the same `Transport` (`src/backend.rs`, 120 lines) instead of tonic on a tokio runtime. Same protobuf messages. | Browsers cannot speak plain gRPC. |
| 6 | `wait_for_cover` (a loop with `std::thread::sleep`) became `CoverWait::poll`, a small state machine. The caller syncs, polls with the current time and sleeps its own way. | `std::thread::sleep` does not work on wasm32. Unit-tested natively. |
| 7 | No `std::time` in the wallet: a `now_ms()` helper (`Date.now()` in wasm) for timings only. Wall-clock time stays an input (`now_unix`), as it already was. | `SystemTime::now()` panics on `wasm32-unknown-unknown`. |
| 8 | Wallet keys (`Me`) are made from bytes, not from `wallet.json`. The worker keeps the spending key in IndexedDB. | No file system. |
| 9 | The verifying key comes from the proving key (`pk.verifying_key()`), not from a second `VerifyingKey::build`. | Saves one `keygen_vk`. `node/` should do the same. |
| 10 | A panic hook that writes to the console; `Option<u64>` arguments are passed as `BigInt` from JavaScript. | wasm-bindgen details. |

The browser cannot call Zebra's JSON-RPC directly, because Zebra sends no CORS headers.
`proxy.mjs` (30 lines of Node) forwards POSTs and adds them. The page and the proxy run on
different ports, so these are real cross-origin requests.

## Testnet, read-only (gRPC-web)

`testnet.mjs`, against `https://zcash-testnet.chainsafe.dev` (ECC LightWalletD v0.6.1),
from the wallet worker:

| lightwalletd call | Wallet uses it for | Verified |
|---|---|---|
| `GetLightdInfo` | not used by the wallet | yes, 1.1 s (first call, includes TLS setup) |
| `GetLatestBlock` | tip | yes, 0.3 s |
| `GetTreeState` | Ironwood tree frontier before the birthday | yes, 0.35 s; `ironwood_tree` parses (946,697 leaves) |
| `GetBlockRange` | compact blocks, Ironwood pool | yes; 10 blocks in 0.34 s; the wallet's own `sync_lwd` did 5,000 blocks (5 calls) in 9.5 to 10.5 s |
| `GetTransaction` | the full transaction of an own note, for the memo | the call is verified (39,678-byte transaction with 2 Ironwood actions, 0.3 s); not yet through the path that finds an own note |
| `SendTransaction` | broadcast | the call path is verified: 4 zero bytes are rejected with `-22 parse error: bad tx header`. No real transaction was sent on testnet. |

The scan found no notes, as expected for a fresh key. A testnet wallet address encodes as
`utest1...`.

## The proving key between page loads

- Within one worker the key is built once and kept (`OnceLock`): a second call takes 0 ms.
- Across a page reload nothing survives: the key build costs 19.4 s and 19.8 s again.
- About half of the build is the Halo 2 parameters (`Params::new(11)`: 9.9 s). They are
  deterministic and serialize to 131 KB, which reads back in 126 ms.
- `orchard` builds the parameters inside `ProvingKey::build` and has no way to pass them
  in. With a 7-line patch to a local copy of `orchard` (`build_with_params`), building the
  key from serialized parameters took **9.2 s against 19.4 s** for the normal build, in the
  same worker. The patched build was timed only; it was not used for a proof.
- The rest (`keygen_vk` and `keygen_pk`, about 9.5 s) cannot be cached without
  serializing the Halo 2 proving key, which `halo2_proofs` 0.4 does not offer.

So: start the key build at page load. The user scans a QR code and pays, which takes
longer than 20 s, so the build is hidden in practice. If it must be faster, ship the 131 KB
parameters with the page and carry the `orchard` patch.

## Not verified

- A spend on testnet, and memo recovery through `GetTransaction` for an own note.
- A real `SendTransaction` on testnet.
- Memory use of the worker (not measured here; the prover experiment measured 107 MB).
- Firefox and Safari. Only Chromium was used.
- Threads (`wasm-bindgen-rayon`, cross-origin isolation): not tried.
- The page's send button by hand. The check drives the same worker calls through
  `window.engine`; the "Create wallet and scan" button was clicked once against regtest.
- How long ChainSafe keeps its gRPC-web endpoint public.
- The native build of the gRPC-web client against a live server (only the browser build
  was run against testnet; the frame decoder has unit tests).

## Recommendation for the real engine

The maker (native, in our backend) and the taker (wasm, in a worker) should share one
crate, with all input and output behind three small interfaces. This spike shows that the
wallet needs nothing else.

**Crate layout.**

- `zecret-core` (builds for both targets, no I/O): `keys`, `msg`, the wallet (`wallet.rs`
  as in this spike), and the maker and taker state machines. It depends on the crates in
  this spike's `Cargo.toml`, with the feature choices of changes 1 to 3. Native-only
  features (`multicore`) are switched on by the maker binary, not by the core.
- `zecret-maker` (native binary): the core, plus `reqwest`, files and threads.
- `zecret-taker-web` (cdylib for the worker): the core, plus `wasm-bindgen` glue like
  `src/web.rs`.

**Network.** The core uses a `Transport` with one call, `post(url, headers, body) ->
response`. Zebra JSON-RPC, lightwalletd (gRPC-web) and NEAR JSON-RPC are thin clients on
top of it, the same on both sides. The maker can use gRPC-web too, so there is one
lightwalletd client, not two. The wallet needs these lightwalletd calls: `GetLatestBlock`,
`GetTreeState`, `GetBlockRange`, `GetTransaction` and `SendTransaction`.

For the hackathon, keep the interface blocking and use synchronous XHR in the worker.
That is what this spike proved, and it keeps the code of both sides identical. The cost:
while a request runs, the worker cannot answer the page. The prover blocks the worker for
25 s anyway, so the page must show progress from its own state, not by asking the worker.
After the hackathon, move to an async `Transport` (`fetch` with `wasm-bindgen-futures`;
`reqwest` async natively). That allows cancellation and streaming of `GetBlockRange`.

**Time and waiting.** No sleeps in the core. Each state machine exposes
`tick(now_unix) -> Next`, where `Next` says "call again at time T" or "done". The maker's
main loop sleeps a thread; the worker uses `setTimeout`. `CoverWait` in this spike is the
pattern. Wall-clock time is an argument, never read inside the core; protocol deadlines
use NEAR block time, as they do now.

**Storage.** The core uses a `Store` with `get(key)` and `put(key, bytes)` for the swap
record and the wallet key. The maker implements it with files (`common::save` as now).
In the browser, IndexedDB is asynchronous and blocking Rust code cannot wait for its
callbacks, so the worker does not call IndexedDB from inside Rust. Instead, `tick` returns the changed
records and the worker writes them to IndexedDB before it calls `tick` again. A record
that must be durable before an action (for example the share `a` before the request memo
goes out) is made in one tick and used in the next. The wallet scan state is not stored:
the taker's wallet is new, so a rescan from its birthday is short (5,000 testnet blocks in
about 10 s).

**Proving key.** One `OnceLock` in the core, as now. The worker starts the build at page
load. Take the verifying key from the proving key (change 9).
