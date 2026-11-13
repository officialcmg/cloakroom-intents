# Security review 3: the website, `zecret-wasm` and the maker's relay

Date: 2026-11-13. Reviewed code: the working tree of `/home/user/zecret` (no git metadata,
so no commit id). Another session edited `crates/zecret-core` (taker.rs, maker.rs),
`crates/zecret-node` and `crates/zecret-wasm` during this review for the taker-side fixes
of review 2. Line numbers refer to these files (sha256 prefix):

| File | sha256 |
|---|---|
| `crates/zecret-node/src/relay.rs` | `af31720d604a` (unchanged at the end of the review) |
| `crates/zecret-node/src/main.rs` | `0c909a7925f2` |
| `crates/zecret-core/src/maker.rs` | `e66958e4c4cf` |
| `crates/zecret-core/src/taker.rs` | `66193331d264` (changed later by the other session) |
| `crates/zecret-wasm/src/{web,exit,session,transport}.rs` | `57fc9eefb2ff`, `40827a038b7f`, `114c6c40ecf6`, `22dd99987be7` |
| `web/src/engine/real/realEngine.ts`, `idb.ts` | `47caad470e21`, `7cb08f324f31` |
| `web/src/lib/netconfig.ts`, `directory.ts`, `web/src/app/useZecret.ts` | `89bdbec075c5`, `a626fb62d612`, `2d89921e0cf8` |
| `web/vercel.json` | `c0976d25767a` |

Machine-readable records: `findings-3.json` (12 records; the method's
`validate-findings.cjs` says PASS). Proofs of concept: `poc3/`.

## 1. Result

| Id | Severity | Title |
|---|---|---|
| F3-02 | Medium | One unauthenticated TCP client can stop the relay, and grow the maker process without bound |
| F3-01 | Low | The relay pays `execute_intents` without limit for any account that ever sent the maker a request |

Needs validation: NV3-01 (a link chooses every trust anchor of the real engine; blocked
on Vercel only by the CSP), NV3-02 (two tabs run the same swap with no lock).

Both confirmed findings are in the maker's relay, which only the direct exit uses (local
network and testnet). Neither lets anyone take funds. No finding in the website or the
wasm engine lets a third party take the user's funds or make the user approve a worse
trade than the page showed, on the deployment that `web/vercel.json` describes.

## 2. Scope and method

**In scope.** `web/` (page, engine interface, real worker `web/src/engine/real/`, simulated
engine as far as it can be confused with the real one, `public/config/`, the maker
directory, `vercel.json`, `package-lock.json`); `crates/zecret-wasm` (all of `src/`);
`crates/zecret-node/src/relay.rs` and its wiring in `main.rs`; in `crates/zecret-core` the
parts the browser path added (`Wallet::sends`, `Taker::reconcile`, the "never fund twice"
check in `on_open`). Review 1 (escrow, `zecret-intents`) and review 2 (the engines) are not
repeated. F2-01 to F2-03 (taker) were being fixed by another session and were not
re-reviewed.

**Method.** Cloudflare's published security-audit method, guidance mode, one reviewer,
phases in order: recon, hunt, validate, record. Companion files used: HUNTING,
ATTACK-CLASSES, VALIDATION-AND-REPORTING, CLIENT-SIDE, WEB-PROTOCOL-AND-AUTH,
SUPPLY-CHAIN-AND-RELEASE, DATA-ISOLATION-AND-LIFECYCLE,
RESOURCE-EXHAUSTION-AND-AVAILABILITY. Each candidate was argued against before it was
kept. A finding is confirmed only with a full source trace and a local run. A missing
second layer is a hardening note. Severity is likelihood × impact and never above the
impact.

**Execution.** Only local code ran. No public endpoint was called; no testnet or mainnet
transaction was made.

- `cd web && npx vitest run`: 61 tests pass.
- `poc3/config_override.test.ts`, copied into `web/tests/unit/` for the run and removed
  after: 4 tests pass (they document behaviour; they do not fail).
- `poc3/relay_pocs.rs`, from a scratch workspace in the session scratchpad that depends on
  `zecret-core` and `zecret-node` by path, with `CARGO_TARGET_DIR=crates/target` (no new
  target directory): 3 tests, all fail on the reviewed code, as intended. The relay's own
  `handle` and `serve` run; a scripted NEAR RPC stands in for the network; the TCP tests
  use a free loopback port.
- Not run: `harness/run.py`, `harness/browser/run.mjs` (in use by another session), the
  NEAR sandbox, a browser. No source file was changed.

**Covered.**

- Page boot: where the configuration and the directory come from (`?config=`,
  `?directory=`, `?demo=1`, build-time env), what is checked before use, what the page
  shows from unsigned fields.
- The real worker: init checks, `createSwap` (amount, address and return-address checks,
  `minUsdc`, `maxPremium`), persistence order, `resume`, `importRecovery` and
  `exportRecovery`, the step loop, refunds and leftovers, the 1Click and direct exits and
  the values passed to `zecret-intents` (approval from user-approved state, `now` from
  `Date.now()` right before each call).
- Price math (`price.ts`, `usePrice.ts`, `SwapForm.tsx`): what is shown versus what the
  engine enforces; rounding direction.
- DOM sinks: no `dangerouslySetInnerHTML`, `innerHTML`, `eval`, `new Function` or
  `window.open` in `web/src`; every `href` (explorer links, the ZIP 321 link); clipboard;
  the recovery download.
- ZIP 321 URI and QR code; address checks for all 17 families.
- CSP and headers in `vercel.json`; workers and the wasm module under that CSP.
- Dependencies: 241 locked packages, all from registry.npmjs.org with integrity; one
  install script (`fsevents`, dev, macOS only); 5 direct runtime dependencies.
- `zecret-wasm`: the synchronous XHR transport, panics, the session over `MemStore`, the
  exit steps.
- The relay: authorization, request parsing, concurrency, CORS, logging, error echoes,
  what a lying relay can do.
- Core browser additions: `reconcile`, `Wallet::sends`, the "never fund twice" check.

**Not covered.** A real browser run (CSP enforcement, two tabs, IndexedDB quota); Firefox
and Safari; the deployed site's headers (no deployment URL in the repository); the
`zecret-intents` checks themselves (review 1); 1Click's own behaviour; the e2e and
screenshot suites; the gRPC-web parser beyond review 2's H3.

## 3. Confirmed findings

### F3-02 (Medium): one unauthenticated TCP client can stop the relay, and grow the maker process

**Trace.**

1. `relay.rs:117` `serve`: one thread, one connection at a time.
2. `relay.rs:119`: only a per-read timeout of 10 s. A client that sends a byte every few
   seconds never trips it, and no total deadline exists.
3. `relay.rs:141`, `:148` `read_request`: `read_line` into a `String` with no length
   limit, for the request line and every header.
4. `relay.rs:157`: the 64 KiB limit is checked only after all headers, and only against
   `Content-Length`.
5. `main.rs:161-175`: the relay is a thread of `zecret maker`, in the same process as the
   maker loop (refunds, Ready, stipends, withdraws).

**Reproduction.** `poc3/relay_pocs.rs`:

- `poc_f3_02_one_slow_client_blocks_the_relay`: one client sends
  `POST /execute-intents HTTP/1.1\r\nx-slow: ` and then one byte every 2 s. A second client
  sends an `OPTIONS` preflight. Observed: no answer within 12.2 s (an idle relay answers at
  once).
- `poc_f3_02b_one_header_line_grows_the_maker_process`: one client streams 256 MiB with no
  newline. Observed: process RSS 270 MiB → 527 MiB. It grows for as long as the client
  streams.

**Impact.** Anyone who can reach the relay stops every direct exit while connected. The
takers' tokens stay in the Verifier under their own keys, but they cannot exit (the
stipend cannot pay for `execute_intents`). With the memory variant the whole maker
process can be killed, which also stops refunds and Ready for every open swap until the
operator restarts it. On the networks where the relay runs (local, testnet) the money has
no value; the maker is a shared service for all takers of the deployment.

**Severity.** Likelihood high (no account, one socket). Impact medium (availability of a
shared service on test networks; no funds move). Overall **Medium**.

**Fix.** In `read_request`:

```rust
let deadline = Instant::now() + Duration::from_secs(5);
let mut r = BufReader::new((&*s).take(70 * 1024));   // headers + body together
// before each read_line / read_exact:
s.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1))))?;
if Instant::now() > deadline { bail!("request too slow"); }
if line.len() > 8 * 1024 { bail!("header line too long"); }
```

In `serve`, hand each accepted connection to one of at most 4 worker threads (each with
its own `NearRpc` and a shared `Signer` behind a `Mutex`, so that nonces stay ordered),
and close new connections while all are busy. Regression: the three tests in
`poc3/relay_pocs.rs` pass.

### F3-01 (Low): the relay pays `execute_intents` without limit for any account that ever sent a request

**Trace.**

1. `relay.rs:125` → `handle`: the body of any `POST /execute-intents`.
2. `relay.rs:84`: `signer_id` from the attacker's payload.
3. `relay.rs:85`, `:64-75`: the only check is that the signer equals `taker_account` of
   **any** record under `swap/`, whatever its stage.
4. `maker.rs:488-497`: the maker writes such a record (stage `Quoting`,
   `taker_account = hex(rfq.taker_pk)`) for every request memo that passes the format
   checks, before any premium. `maker.rs:596-601`: an expired request becomes `Aborted`;
   no record is ever deleted.
5. `relay.rs:89`: `simulate_intents` accepts any correctly signed payload with an unused
   nonce.
6. `relay.rs:92`: the maker signs and pays `execute_intents` with 50 Tgas. No count per
   swap, no stage check, no check of the intent.

**Reproduction.** `poc3/relay_pocs.rs`, `poc_f3_01_any_request_sender_drains_relay_gas`:
the store holds one `Aborted` record whose `taker_account` is the attacker's implicit
account; 25 payloads with fresh nonces go through `Relay::handle`. Observed: all 25 were
relayed; the maker signed 25 transactions. A signer with no record is refused.

**Why a valid payload is cheap.** The attacker deposits any token into its own Verifier
account once and signs a `transfer` of 1 base unit per payload, or an
`add_public_key`/`remove_public_key` pair; every fresh nonce is a new payload. (Not run
against the Verifier here; this is standard Verifier behaviour.)

**Impact.** One request memo (one shielded fee, about 0.0001 ZEC; free on testnet) buys an
unlimited stream of maker-paid transactions, one every few seconds (the relay is serial).
That drains the maker's NEAR, after which the maker cannot send stipends, refunds or
withdraws. No taker funds move. The shipped configurations use the relay only on local
and testnet; `main.rs` does not refuse `--relay` on mainnet.

**Severity.** Likelihood medium; impact low (test money, liveness). Overall **Low**.

**Fix.** In `relay.rs`:

1. `takers()` returns only records in a stage that shows the taker claimed
   (`Sweeping`, `Swept`), with their swap key.
2. `handle()` requires that the inner payload has exactly one intent, an `ft_withdraw` of
   the swap token (from the maker's configuration), and refuses anything else.
3. `handle()` keeps a relay count in the swap record (`relayed`) and refuses after 3.
4. `run_maker` refuses `--relay` when the maker's chain is mainnet.

Regression: `poc_f3_01_…` passes (it expects at most 1 transaction); add a test that a
`Swept` record with an `ft_withdraw` payload is relayed once, and a second identical
request is refused by the count.

## 4. Needs validation

### NV3-01: a link chooses every trust anchor of the real engine

`configUrl` (`netconfig.ts:108-114`) takes any `?config=<URL>`, ahead of
`VITE_ZECRET_CONFIG`. `parseNetworkConfig` (`netconfig.ts:82-105`) accepts any escrow,
escrow code hash, NEAR RPC, light-wallet server, Verifier, `minStipend` (0 too) and relay,
and `http://` endpoints on mainnet. `?directory=` overrides the configuration's directory
(`useZecret.ts:71-72`, `directory.ts:141-142`). The banner then shows the attacker's
`label` and `honesty` lines. The engine uses these values as its trust anchors
(`realEngine.ts:246-281`). An attacker's escrow and code hash, or an attacker's NEAR RPC
working with a maker (NV2-01), can take the ZEC of a victim who follows the link and pays.

The decisive control is outside the code: on Vercel, `vercel.json`'s `connect-src` blocks
the fetch of a cross-origin configuration or directory (and of `data:` URLs). The
documented testnet run (`web/README.md:78`, `vite preview`) and any other host send no
CSP. `poc3/config_override.test.ts` shows the parsing side.

- Local plan: serve `web/dist` once with `vite preview` and once with the `vercel.json`
  headers; open `/?config=http://127.0.0.1:<other port>/hostile.json`. Expect the first to
  load the hostile escrow into the banner, and the second to fail with a CSP violation.
- Owner plan: check that the production page and the worker script both carry the CSP.
- Fix regardless of the result (defence in depth, cheap): accept only bare names
  (`/^[a-z0-9-]+$/`) for `?config=` in production builds (a URL only when
  `import.meta.env.DEV`); ignore `?directory=` when a configuration is loaded; for
  `network: "mainnet"` require `https:` endpoints and compare `escrow`, `escrowCodeHash`,
  `token` and `verifier` with constants compiled into the bundle.

### NV3-02: two tabs run the same swap with no lock

Each tab has its own worker, loads the same IndexedDB records, ticks its own copy and
writes its dirty records and `web/state`/`web/meta` over the other's (`idb.ts:50-56`,
`realEngine.ts:169-186`, `:374-398`, `:651`). Source review found no double payment: both
copies spend the same notes (the network refuses the second), each exit moves the whole
Verifier balance (one executes), and one NEAR access-key nonce takes one of two equal
transactions. It found regressions: an older record or `web/meta` written over a newer
one, an exit that polls the wrong deposit address, and a finished tab writing its old
`web/state` over a new swap made in the other tab. Whether the last one can orphan a paid
new swap (whose key "Start a new swap" would then delete) needs a run of the real engine
in two tabs. Plan: `harness/browser/run.mjs --serve`, two tabs, record IndexedDB and the
chains after each stage, and repeat with "Start a new swap" in one tab while the other
polls a leftover. Fix regardless: take `navigator.locks.request('zecret-engine', {ifAvailable: true})`
in the worker before `resume`, and if it is held show "This swap is open in another tab";
store a counter in `web/meta` and refuse to write an older one.

## 5. Rejected candidates

Recorded in `findings-3.json` with their reasons.

- **A hostile maker directory makes the user pay an attacker.** The maker is an untrusted
  counterparty by design. The escrow must equal the configuration's
  (`realEngine.ts:319`), the directory entry's signature and the maker key are checked
  before any swap (`:334`), the firm price must reach the `minUsdc` and the premium stay
  under the `maxPremium` that the page showed. Unsigned fields only change what the user
  sees before agreeing.
- **XSS through token names, error strings, explorer URLs.** No raw HTML sinks; React
  escapes text; React 19.3.1 and the CSP (no `'unsafe-inline'`) both block `javascript:`
  hrefs. See H3 for the plain-link case.
- **A forged "below minimum" offer.** `check_quote` checks 1Click's signature first
  (`check.rs:275`); `acceptMinimum` can only lower the minimum after an offer exists, and
  the next quote is checked again.
- **A hostile light-wallet server makes a restored browser send twice.** Every send spends
  the temporary wallet's notes; a second request, acceptance or funding would reuse spent
  notes. A server can only stall (review 2, H8).
- **The maker uses "never fund twice" to block the taker.** A maker note at the joint
  address stops the taker's funding, so the ZEC stays in the temporary wallet and comes
  back through `refundStep`/`leftoverStep`. The taker loses the premium, which a maker can
  always cause by refunding.
- **A lying or withholding relay.** The payload is signed and names receiver and amount;
  one nonce. Only delay.
- **CORS `*` on the relay.** No ambient credentials; a page can do only what a direct
  client can.
- **A hostile recovery file.** Its author holds its keys, so its swap is theirs; it cannot
  replace an unfinished swap (`SWAP_EXISTS`) or reach the user's keys; nothing is
  uploaded. See H2.
- **Network-supplied time in the exit checks.** The worker passes `Date.now()` read right
  before `exitQuote`, `exitSubmit`, `directPrepare` and `directRelay`; the approval is
  built from the user's stored choices and the payout (`realEngine.ts:1096-1107`).

## 6. Hardening notes (not vulnerabilities)

- **H1. A failed IndexedDB write loses the dirty set, and the loop goes on.**
  `persistCore` takes the dirty records (`session.rs:38`, `store.rs:40` empties the set)
  before `putAll`; when the write fails (quota, a closed database) `persist` throws, `step`
  catches it and calls `tick` again after 5 s (`realEngine.ts:182-186`, `:637-641`). The
  core then acts on records that storage does not hold, against the durability rule of
  `ENGINE_API.md`. No attacker causes this; the effect is that a reload after the failure
  starts from an older record (at worst a refused rebroadcast and a refund, losing the
  premium). Fix: on a failed write, keep the entries and retry the same write before any
  other engine call; stop the loop and show `STORAGE` until it succeeds.
- **H2. `importRecovery` trusts the file's page state.** `deposit.address`, `deposit.uri`,
  the destination and the direct-exit account come from `web.state`
  (`realEngine.ts:551-552`) and are not checked against the wallet key or re-validated;
  the QR (`Pay.tsx:172`) and the address text (`:185`) are read separately. A planted file
  can show a payment address in the site's own UI. Recompute `deposit.address` with
  `taker.ua()` and `deposit.uri` with `paymentUri`, re-run the address checks of
  `createSwap`, and show "restored from a file" on the payment screen.
- **H3. Links from the network.** `destinationTxs[].url` from 1Click's status becomes an
  `href` (`realEngine.ts:1169`). Allow only `https:` URLs (or build the link from
  `chains.ts`' `txUrl`); same for the configuration's explorer templates.
- **H4. An exit whose submit answer is lost.** `signedDeposits` is appended only after a
  successful `submit-intent` (`realEngine.ts:1151-1152`). If the answer is lost and the
  quote then expires, the engine signs for a new deposit address and polls only that one;
  the first intent may already have delivered. The page then waits forever. Record the
  deposit address before submitting and poll all of them (docs/INTENTS.md §6).
- **H5. The direct exit waits forever when the receiver spends.** `delivering` needs the
  receiver's balance to rise by the amount (`realEngine.ts:1076-1088`); a receiver that
  moves the tokens at once (an exchange) is never seen as done. Also accept "nonce used and
  the taker's Verifier balance is 0".
- **H6. A wasm trap poisons the engine.** A panic aborts inside the wasm instance; the
  worker keeps calling the same instance (`realEngine.ts:637-641`). Terminate and recreate
  the worker after a `RuntimeError`.
- **H7. The directory signature covers only the maker's identity.** Rate, limits, premium
  and fee are unsigned. This is fine while the directory is a same-origin file. Sign the
  whole document before directories are served from elsewhere.
- **H8. Build reproducibility.** Vercel runs `npm install` by default; set
  `"installCommand": "npm ci"` in `vercel.json`.
- **H9. Price wording.** The page calls `destMin` the "guaranteed minimum". The engine
  enforces it as "nothing is signed below this without the user's consent"; delivery below
  it is not possible, but delivery at all is not guaranteed. The words should say that.
- **H10. Testnet relay URL.** `public/config/testnet.json` points the relay at
  `http://127.0.0.1:3410`, which the deployed CSP does not allow; the direct exit cannot run
  from the deployed site. Functional, not a security issue.

## 7. Proofs of concept (`poc3/`)

| File | What |
|---|---|
| `relay_pocs.rs` | `poc_f3_01_…`, `poc_f3_02_…`, `poc_f3_02b_…`; all fail on the reviewed code and pass once fixed |
| `config_override.test.ts` | NV3-01: what the URL parameters can select (documents behaviour; passes) |

How to run `relay_pocs.rs` without touching `crates/`: make a crate with dependencies
`zecret-core` and `zecret-node` by path, `anyhow`, `serde_json`, `hex` and
`[profile.dev] opt-level = 3`; copy `crates/Cargo.lock` next to it; put the file in its
`tests/`; run
`CARGO_TARGET_DIR=<repo>/crates/target cargo test --offline --test relay_pocs -- --nocapture --test-threads=1`.
The tests need no network; two of them bind a free loopback port. To run
`config_override.test.ts`, copy it into `web/tests/unit/` and run
`npx vitest run tests/unit/config_override.test.ts`.
