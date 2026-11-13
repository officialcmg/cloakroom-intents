# Fixes for security review 3

Review: `docs/security/review-3.md`, `findings-3.json`, `poc3/`. Date: 2026-11-13. Not
committed. Line numbers refer to the working tree when this file was written.

## Summary

| Id | What changed | Test |
|---|---|---|
| F3-02 (Medium) | Relay: 5 s deadline for the whole request, 70 KiB per connection, 8 KiB per line, 64 headers, 4 worker threads, 503 when busy, 2 connections per source, panics isolated | `crates/zecret-node/tests/relay.rs` (the review's two F3-02 proofs of concept, adapted, and five new tests); unit tests in `relay.rs` |
| F3-01 (Low) | Relay pays only for one `ft_withdraw` of the maker's token, at most the swap's price, for a swap with a payout; at most 3 transactions per swap, counted in the store; the same payload is never sent twice; 6 relay requests a minute per source; `--relay` refused on mainnet | `tests/relay.rs` (the review's F3-01 proof of concept with real `ft_withdraw` payloads, and two new tests); unit tests in `relay.rs` |
| NV3-01 | Production builds take only `local` and `testnet` from `?config=`; development builds (or `VITE_ZECRET_ALLOW_CONFIG_URL=1`) take any file and show a red warning that names it; `?directory=` ignored when a configuration is loaded; mainnet: https endpoints and escrow, code hash, token and Verifier equal to constants in the bundle | `web/tests/unit/config_override.test.ts` (adapted from `poc3/`), `netconfig.test.ts`, `e2e/tabs.spec.ts` (production build refuses a link) |
| NV3-02 | Web Lock per engine before `init`/`resume`/`createSwap`; a second tab shows "This swap is open in another tab", starts no worker, and takes over when the first tab closes; `web/version` counter refuses an older tab's write | `e2e/tabs.spec.ts` (two pages, one context, desktop and phone); `tests/unit/realEngine.test.ts` (version guard, OTHER_TAB) |
| H1 | After a failed IndexedDB write the engine calls nothing in the core until the same records are written | `realEngine.test.ts` |
| H2 | `importRecovery` computes the deposit address and payment link from the wallet key and checks the file's addresses | `realEngine.test.ts` |
| H3 | Only `https:` links from 1Click, from explorer templates and from recovery files | `config_override.test.ts`, `realEngine.test.ts` |
| H4 | The 1Click deposit address is recorded before the submit; every signed address is polled | not run (see below) |
| H5 | Direct exit: done when the taker's Verifier balance stays 0 for 15 s | not run (see below) |
| H6 | A WebAssembly trap stops the worker's engine; the page ends the worker and starts a new one | `realEngine.test.ts` (the halt); page restart not run |
| H8 | `installCommand: npm ci` in `vercel.json` | – |
| H9 | "Guaranteed minimum" became "Your minimum", with what the engine enforces | screenshots |
| H10 | The CSP test is driven by the testnet configuration; no localhost in the CSP | `csp.test.ts` |
| Item 6 | `public/maker-directory.json`: premium 0.0045 ZEC flat, testnet maker and escrow ids | `money.test.ts` |

Each regression test fails on the code before the fix: the relay proofs of concept failed
in the review's run (review-3.md §2; not re-run against the old code here, because the
tests now use the new `Relay::token` field and `serve_listener`). The web tests were
checked by undoing each fix in turn: the four engine tests and the two-tab test failed,
and passed again with the fix.

## F3-02: relay denial of service

`crates/zecret-node/src/relay.rs`:

- `DeadlineReader` (line 539) sets the read timeout before every read to the time left
  until one deadline, 5 s after accept (`REQUEST_DEADLINE`). A client that sends one byte a
  second gets 400 after 5 s.
- `read_request` (576) reads through `Read::take(70 KiB)` (`MAX_REQUEST_BYTES`); `read_line`
  (556) stops each line at 8 KiB (`MAX_LINE_BYTES`); at most 64 headers; `Content-Length`
  at most 64 KiB; `Transfer-Encoding` refused.
- `serve_listener` (372) starts 4 worker threads (`WORKERS`, 70). `Slot::take` (440)
  answers 503 at once when all 4 are busy or the source already has 2 connections; the
  number of remembered sources is capped at 1024. The slot is freed by `Drop`, also on a
  panic, and before the connection closes.
- `worker` (493): each request runs under `catch_unwind` (504); a panic answers 500 and the
  worker goes on. Poisoned locks are recovered (`lock`). The signer is one `Signer` behind
  a `Mutex`, held for one `handle` call, so nonces stay ordered. Each worker makes its own
  `NearRpc` (its transport is not `Send`).
- Memory: at most 4 requests of 70 KiB each, plus a queue of at most 4 connections.
- The relay runs in its own threads of `zecret maker`, and shares no lock with the maker
  loop. A relay panic cannot reach the maker loop.

## F3-01: relay spends the maker's gas

`relay.rs`:

- `check_exit` (158): the payload's inner message must name this Verifier and hold exactly
  one intent, `ft_withdraw`, of `Relay::token` (new field; `main.rs` passes
  `chains.token`), with a positive whole amount, a receiver, and no field other than
  `token`, `receiver_id`, `amount`, `memo` (so no `msg`).
- `paid_swap` (241): the signer must be the `taker_account` of a swap record with a payout:
  stage `Sweeping` or `Swept`, or `taker_withdraw_at` set (the maker sent `withdraw` for
  the taker; this covers the time between the claim and the sweep, which waits for cover,
  and the punish path that ends in `Aborted` after paying). A record in `Opened` or `Ready`
  answers 503 "try again in a minute", because the maker may not have seen the claim yet;
  the browser treats 503 as transient and retries. Any other stage is refused (400).
- The amount must be at most the swap's `price` (the escrow amount).
- The count: `relay/<swap id>` in the maker's store (`PREFIX_RELAY`) holds the number of
  transactions and the payloads sent. It is written before the transaction is sent, under
  a lock (`RECORDS`, 228). The review asked for the count in the swap record itself; that
  record belongs to the maker loop, which rewrites it from memory and would drop the
  field, so the count is a separate record of the same store, keyed by the swap id. After
  3 (`MAX_RELAYS_PER_SWAP`) the swap is refused. A payload that was already sent is not
  sent again: the answer is `{"ok": false, "pending": true, "hash": …}` (the browser then
  sees the used nonce), or 400 with the earlier failure.
- Per source: 6 `POST /execute-intents` a minute (`relay_allowed`, 463), then 503.
- `main.rs` line 165: `zecret maker --relay` stops with an error when `chains.network` is
  `main` or `mainnet`.
- Errors carry a status class (`Refused`, `status_of`): 503 for "try again", 400 for a
  refusal. A transaction that could not be sent is 503.

## NV3-01: a link chose every trust anchor

- `web/src/lib/netconfig.ts`: `chooseConfig` (200). `?config=` must be one of
  `KNOWN_CONFIGS` (22, `local`, `testnet`) unless `CONFIG_OVERRIDE_ALLOWED` (28) is true:
  in `vite` development, or a build made with `VITE_ZECRET_ALLOW_CONFIG_URL=1`. Otherwise
  the page shows "The network settings did not load" with the reason. `VITE_ZECRET_CONFIG`
  (build time) is unchanged.
- `ConfigOverrideBanner` (`web/src/screens/Shell.tsx` 69): a red banner above the page that
  names the file, shown as soon as the override is chosen.
- `parseNetworkConfig` / `checkMainnet` (167): mainnet requires https for the NEAR RPC,
  lightwalletd, 1Click and a directory on another site, no `zebraRpc`, and `escrow`,
  `escrowCodeHash`, `token`, `verifier` equal to `MAINNET_ANCHORS` (43). Token and
  Verifier are Circle's USDC and `intents.near`. No mainnet escrow is deployed, so the
  escrow and its code hash come from `VITE_MAINNET_ESCROW` and
  `VITE_MAINNET_ESCROW_CODE_HASH` at build time; without them every mainnet configuration
  is refused. All networks: explorer templates must be https; endpoints must be http or
  https URLs.
- `web/src/app/useZecret.ts` 83: with a configuration, the directory is the
  configuration's; `?directory=` is read only for the demo.

### What the browser scenario runner must pass

`harness/browser/run.mjs` opens `?config=/run/<scenario>/config.json`, a path, which a
production build now refuses. It must build the site with the override switch. In
`main()`, in the build loop, the `vite build` step needs
`VITE_ZECRET_ALLOW_CONFIG_URL=1` in its environment, for example:

```js
const env = { ...process.env };
delete env.LD_PRELOAD;
if (cmd === 'npx') env.VITE_ZECRET_ALLOW_CONFIG_URL = '1';
```

Nothing else changes: the URLs, the files and the headers stay. The page then shows the
red "Test build with outside network settings" banner above the content; it does not
cover any control. With `SKIP_BUILD=1` the runner uses the existing `web/dist`, which must
come from such a build: `cd web && VITE_ZECRET_ALLOW_CONFIG_URL=1 npx vite build`. A plain
`npm run build` or `npm run e2e` (which builds) writes a `web/dist` without the switch.
`?config=local` (the `--serve` mode's second URL) works in every build.

## NV3-02: two tabs on one swap

- `useZecret.ts` 93-97, `takeLock` (161): before the engine starts (so before `init`,
  `resume` and any `createSwap`), the page takes the Web Lock `zecret-engine` (real) or
  `zecret-engine-demo` (simulated) with `ifAvailable`. If another tab holds it, the page
  shows "This swap is open in another tab" (`App.tsx` 107), starts no worker, and waits
  for the lock; when the other tab closes, it starts its engine and shows the swap. The
  lock is held for the life of the page and through an engine restart.
- `web/src/engine/real/idb.ts`: `web/version` counts writes. A store reads it when it opens
  and on `all()`; `putAll` and `clear` check it in the same IndexedDB transaction
  (`guarded`, 62) and add one. A store whose count is older gets `StaleWriteError` (16) and
  writes nothing. `memoryRecords` (112) has the same rule (and a shared map for tests).
- `realEngine.ts` `persist` (226): a `StaleWriteError` stops the engine (`OTHER_TAB`); the
  page ends the worker and shows the other-tab screen with "Reload this tab".
- New error codes in `web/src/engine/api.ts`: `OTHER_TAB`, `ENGINE_CRASHED`.

## Hardening notes (§6)

- **H1, applied.** `realEngine.ts`: `persist` (226) keeps the records of a failed write in
  `unsaved` (129) and emits one `STORAGE` error with `transient: true`. `run` (153) writes
  `unsaved` first before any work; while that fails, every page call is refused with
  `STORAGE` and the loop only retries the write every 5 s. No core call (and so no reveal
  and no broadcast) happens on unpersisted state. The page shows a red notice "Your swap
  could not be saved in this browser" (`Shell.tsx` `EngineWarnings`, 83), removed on the
  next engine event (every event is stored before it is sent).
- **H2, applied.** `importRecovery` (`realEngine.ts` 626) checks the file's page state with
  `checkRestoredState` (652): the meta's swap id, the stage, the amounts, the destination
  address (1Click networks) or NEAR account (direct exit), and the return address, as
  `createSwap` does; links are kept only when https. Then it sets `deposit.address` from
  `taker.ua()` and `deposit.uri` with `paymentUri`. The payment screen says "Restored from a
  recovery file" (`Pay.tsx` 198; `SwapState.restoredAt`).
- **H3, applied.** `safeLink` (`netconfig.ts` 241) keeps only absolute https URLs; used for
  1Click's `destinationTxs[].url` (`realEngine.ts` 1343), for explorer templates
  (`explorerUrl`, 235; the parser also refuses non-https templates) and for links in a
  recovery file.
- **H4, applied (it is TypeScript, not Rust).** The deposit address of the firm quote is
  added to `signedDeposits` and stored before `exitSubmit` (`realEngine.ts` 1300). While
  delivering, and before a new firm quote, the engine asks the status of every signed
  address; one that reports `SUCCESS` finishes the swap (`earlierExitDelivered`, 1274,
  1328). Only `SUCCESS` of an older address counts; an older address that never got a
  deposit does not block. Not run against 1Click or the recorded answers.
- **H5, applied (TypeScript).** In the direct exit's `delivering`, if the receiver's balance
  did not rise but the taker's Verifier balance reads 0 for 15 s at final blocks, the swap is
  done (`realEngine.ts` 1222). The engine reaches `delivering` only after the relay reported
  the intent executed or its nonce used, and a failed `ft_transfer` is refunded into the
  Verifier within a few blocks. Not run against the sandbox.
- **H6, applied.** `guardWasm` (`realEngine.ts` 188) wraps the module and every `WasmTaker`:
  a `WebAssembly.RuntimeError` stops the engine (`ENGINE_CRASHED`), and no later call
  reaches the trapped instance, also on paths that catch errors and retry. The page
  (`useZecret.ts` `restart`, 142) ends the worker, starts a new one that resumes from
  IndexedDB, and shows "The swap engine restarted"; after 3 restarts in 10 minutes it shows
  the failure screen. The halt is unit-tested; the restart in a browser was not run (it
  needs the real engine to trap).
- **H7, not applied.** Signing the whole directory needs a change in the maker
  (`zecret directory`, `crates/zecret-node`/`zecret-core`) and the core's check. The other
  agent added `maker.signedLimits` (limits covered by the signature) meanwhile. The review
  says the unsigned fields are fine while the directory is a same-origin file, which
  NV3-01 now enforces for production builds.
- **H8, applied.** `web/vercel.json`: `"installCommand": "npm ci"`.
- **H9, applied.** `SwapForm.tsx` 441: the row is "Your minimum", with: the engine accepts
  no maker price below the USDC floor (the swap stops and the ZEC comes back); it signs no
  delivery whose firm minimum is below this amount unless the user accepts a lower one;
  NEAR Intents then delivers at least the signed minimum or returns the USDC to the user's
  key; delivery itself is not guaranteed. `Finished.tsx` 110: "Your minimum was". The
  headline "You receive at least …" is unchanged.
- **H10, partly applied.** `tests/unit/csp.test.ts` now derives `connect-src` from the https
  endpoints of `public/config/testnet.json`, including `exit.relayUrl` when it is https, and
  refuses any `http:` host. So a public relay is added by writing its https URL into
  `testnet.json`; the test then fails until `vercel.json` lists its origin. Not done: no
  public testnet relay exists, so `testnet.json` still points at `http://127.0.0.1:3410`
  and the direct exit still cannot run from the deployed site. The owner must run a relay
  at a public https URL and put it in both files.

## Item 6: the sample maker directory

`web/public/maker-directory.json`: `premium` is now `{ "bps": 0, "minZec": "0.0045" }`
(450,000 zats flat, `premium_zats` of `deploy/maker/README.md`, the smallest premium its
premium check passes at the example price); `nearAccount` and `escrowContract` are
`maker.zecret-3f109eea1e.testnet` and `escrow2.zecret-3f109eea1e.testnet` from
`deploy/deployment.testnet.json`. Rate (940) and limits (0.01 to 2 ZEC) already matched.
The Zcash address and request key are still sample values (no maker runs on testnet). The
price-math unit tests now use their own premium terms; a new test ties the sample to the
two deploy files. `e2e/journey.spec.ts` expects the new first quote amount (935.488 USDC).

## Requirements for other owners

For the agent that owns `crates/zecret-wasm` and `harness/`:

1. **`crates/zecret-wasm/tests/sandbox.rs` does not compile now** (`cargo check -p
   zecret-wasm --tests`: "missing field `token` in initializer of `Relay`", line 151). In
   `relay(w, takers)`:
   - add `token: w.wrap.clone(),` to the `Relay { … }` literal;
   - write each record with a payout and a price, for example
     `json!({"rfq_id": format!("{i:02}"), "taker_account": t, "stage": "Swept", "price": amount.to_string()})`,
     where `amount` is at least what the test withdraws (30 NEAR in
     `direct_exit_through_the_makers_relay`; pass it in or use `u128::MAX.to_string()`).
   - The stranger check (line 173) still holds: a signer with no record gets
     "… is not the taker of a swap with this maker".
   - The resubmission check (line 187) holds: the browser side sees the used nonce first.
2. **`harness/browser/run.mjs`**: build the site with `VITE_ZECRET_ALLOW_CONFIG_URL=1`
   (see "What the browser scenario runner must pass").
3. **`harness/run.py`** and other users of `zecret maker --relay`: a relay request for a
   swap whose maker record is still `Opened`/`Ready` without `taker_withdraw_at` now gets
   503; the browser engine retries it (`direct_relay` maps 5xx to a transient `NETWORK`).
4. **`crates/zecret-wasm/src/exit.rs` `direct_relay`** (optional): a 400 from the relay is
   final (`EXIT_FAILED`). That is right for the new refusals (wrong token, count used up).
   No change is needed.

For the owner: a public https relay for testnet (H10); `VITE_MAINNET_ESCROW` and
`VITE_MAINNET_ESCROW_CODE_HASH` at build time once a mainnet escrow exists.

## What was run

- `cargo test -p zecret-node`: lib 11 passed; `tests/relay.rs` 9 passed (7 runs after the last
  change; one test failed now and then on a fixed 200 ms sleep, and now polls instead); `tests/sandbox.rs` with
  `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD` 1 passed.
- `cargo clippy -p zecret-node --all-targets -- -D warnings`: clean.
- `web`: `npm run typecheck`, `npm run lint` clean; `npm test` 78 passed (7 files);
  `npm run build`; `npm run e2e`: 50 passed (38 desktop and phone, 12 screenshot tests;
  the screenshot tests rewrote `web/screenshots/`, which now show the new sample premium).
- Screenshots (development server, both themes, desktop and phone) of the other-tab screen,
  the configuration warning, the storage notice with the restart notice and the restored
  payment screen, and the minimum note. One fix followed: the note under "Your minimum"
  was bold.

## Not verified

- The relay against a real NEAR RPC and the browser engine through it (the `zecret-wasm`
  sandbox test needs the change above; the scenario runners were not started).
- H4 and H5 against 1Click or the sandbox; the page's engine restart after a real trap.
- IndexedDB's version guard in a browser: the IndexedDB path of `idb.ts` has no test (no
  IndexedDB in the unit-test environment); its logic is the same as `memoryRecords`, which
  is tested, and the two-tab e2e test exercises the lock, not the guard.
- Firefox and Safari; Web Locks are in all current browsers, and without them the version
  guard is the only protection.
