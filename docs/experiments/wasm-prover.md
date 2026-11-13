# WebAssembly prover: can the Zcash side run in a browser?

**Question.** The Cloakroom client builds shielded Zcash transactions, and each one needs
a Halo 2 proof. Can that code run as WebAssembly, and how fast?

**Result: yes.** One proof takes about 25 s in a real browser, on one thread.

| Check | Native | WebAssembly |
|---|---|---|
| Build the proving key (once per page load) | 4.0 s | 19.3 s |
| One 2-action proof (one shielded send) | 5.0 s | 25.3 s |
| Memory after the run | not measured | 107 MB |

The WebAssembly figures are from headless Chromium. Node with the same module gives
20.0 s and 25.8 s.

The Zcash crates build for `wasm32-unknown-unknown` and `wasm32-wasip1` once the
multithreaded prover is switched off: `orchard` and `zcash_primitives` without their
`multicore` feature. `getrandom` needs its `wasm_js` backend on the first target.

## The crash, and its cause

The first runs used Node's built-in WASI runner (`node:wasi`), and Node died with a
segmentation fault at the moment the proof finished. The cause is that runner, not the
prover:

- The same `.wasm` file runs to the end in Chromium.
- The same `.wasm` file runs to the end in the same Node, when a 30-line hand-written
  WASI shim (`browser/main.js`) replaces `node:wasi`.
- Stack size, compiler tier and memory limits made no difference to the crash.

So do not use `node:wasi` to test this code. Use a browser or the shim.

## What this means for the app

- Every shielded send from the browser costs about 25 s of computing: the request memo,
  the acceptance memo, the funding and any refund.
- The key build costs about 20 s once. Start it when the page loads.
- All of it must run in a Web Worker, or the page freezes.
- Threads could make it faster. They need cross-origin isolation headers and a special
  build. Not tested.

Still to check for a browser taker: reading Zcash blocks from a browser. Browsers cannot
speak plain gRPC to a lightwalletd server, so the page needs a gRPC-web endpoint or a
proxy. Cloakroom's client also shells out to `cast`, uses blocking HTTP and sleeps in
loops, which must change.

## Code

The experiment's code (`provebench`, its browser page and the Node shim) ran in a
separate scratch repository and is not in this repository. `spike/browser-wallet/`
supersedes it: it runs the wallet in a Web Worker in headless Chromium and builds,
proves and sends a shielded transaction on Zebra regtest (`node run.mjs`), and
`keybench.mjs` measures the proving key build across page loads. See
`spike/browser-wallet/README.md` and `docs/experiments/browser-wallet.md`. The native
figures in the table have no equivalent test in this repository.
