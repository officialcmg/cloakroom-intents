# Engine interface

The page and the swap engine talk through one small TypeScript interface. The engine
runs in a Web Worker. The page never touches keys, the Zcash wallet, NEAR or the signed
exit; it shows what the engine reports and passes on what the user decides.

- Types: `src/engine/api.ts` (the contract; this file explains it).
- Transport: `src/engine/rpc.ts` (postMessage, any port: `Worker`, the worker's own
  scope, or a `MessageChannel` in tests).
- Fold events into state: `src/engine/reducer.ts` (used by the page and by the simulator).
- Simulated engine: `src/engine/sim/`.
- Typical durations: `src/engine/timeline.ts`.

## Switching to the real engine

One line, in `src/engine/index.ts`:

```ts
const worker = new Worker(new URL('./sim/sim.worker.ts', import.meta.url), { type: 'module' }); // <- engine worker
```

Point the URL at the real engine's worker entry. That entry calls
`serveEngine(self, backend)` from `rpc.ts`, where `backend` implements `EngineBackend`
(the `Engine` methods plus `attach(emit)`). The worker reports which engine it is in
`InitStatus.engine` (`'wasm'`); the page then hides the demo notice and controls. The
real engine may leave out `sim` (calls to `sim.*` are answered with `INVALID_PARAMS`).

## Transport

```
page -> worker   { kind: 'call',   id, method, args }
worker -> page   { kind: 'result', id, ok: true,  value }
worker -> page   { kind: 'result', id, ok: false, error: { code, message, transient? } }
worker -> page   { kind: 'event',  event }
```

Values are structured-cloned. All amounts are decimal strings of integer base units
(zats, micro-USDC, the destination token's base unit); all times are Unix milliseconds.
A rejected call carries an `EngineError` with a stable `code` (below).

**The worker may be blocked.** The real engine proves synchronously (about 25 s per
shielded send, about 20 s for the proving keys) and uses synchronous XHR, so it cannot
answer calls during that time. Calls queue and answer later. The page therefore never
polls; it draws progress from the last `activity` event and its own clock. The engine
must emit an `activity` with `estimatedMs` **before** it starts blocking work.

## Methods

| Method | Allowed | What it does |
|---|---|---|
| `init(config)` | once, first | Checks the maker directory, starts the proving-key build, resolves at once with `InitStatus { engine, phase: 'building_keys', startedAt, estimatedMs, timeScale }`. A later `init` event says `phase: 'ready'`. Throws `INIT_FAILED`. |
| `subscribe(listener)` | any time | All events. Returns an unsubscribe function. (Page side only; not a wire call.) |
| `resume()` | after `init` | Loads the stored swap, if any, emits `swap`, and continues it (the real engine rescans from the wallet's birthday first). Returns the state or `null`. |
| `getState()` | any time | The current `SwapState` or `null`. |
| `createSwap(params)` | no unfinished swap | Makes the temporary wallet, key half `a` and the NEAR key; **stores them before it returns**; returns the state in `awaiting_deposit` with `deposit.address` and the ZIP 321 `deposit.uri`. Throws `SWAP_EXISTS`, `INVALID_PARAMS`, `INVALID_ADDRESS`. |
| `setReturnAddress(addr)` | until the refund or leftover is sent | Unified address with an Orchard receiver on the swap's network. Clears `needs: 'return_address'`. |
| `exportRecovery()` | a swap exists | Returns `{ fileName, mimeType, text }`. Sets `recoveryExportedAt`. The page saves the text as is. |
| `importRecovery(text)` | no other unfinished swap | Restores and continues the swap. Throws `RECOVERY_INVALID`, `RECOVERY_MISMATCH`, `SWAP_EXISTS`. |
| `exit(request?)` | `exit_retry` | Runs the exit again from a fresh firm quote. `request.acceptMinimum` lowers the approved minimum to the `minAmountOut` of `exit.offered`, which the user saw and accepted; it can never raise it. In `paid_out` or `exiting` it is a no-op (the engine runs the exit by itself). |
| `forget()` | before any payment arrives, or after `done` / `refunded` | Deletes the swap and its keys from this browser. Otherwise `WRONG_STAGE`. |

`CreateSwapParams`:

| Field | Meaning |
|---|---|
| `amountZats` | What the user pays into the temporary wallet. |
| `destination` | `{ assetId, symbol, chain, decimals, address }`: 1Click asset id and blockchain id, and the recipient. The engine checks the address again. |
| `returnAddress?` | For refunds and leftovers. Optional; the engine asks (`needs`) when it needs one. |
| `minUsdc` | The engine accepts the maker's firm quote only at or above this. Below it, no premium is paid and the ZEC goes back (`QUOTE_BELOW_MINIMUM`). |
| `minDestinationAmount` | The approved minimum for the exit. The engine signs no exit whose firm `minAmountOut` is below it. |
| `shown` | What the page showed (`usdcExpected`, `destinationExpected`, `premiumZats`, `networkFeeZats`), for display and the record. |

## Events

| Event | When |
|---|---|
| `{ type: 'init', init }` | Key build started, finished (`ready`) or failed. |
| `{ type: 'swap', swap }` | After `createSwap`, `importRecovery`, `resume`, `forget` (`null`): a full snapshot. |
| `{ type: 'stage', swapId, stage, at, patch? }` | Every change. A repeated stage is an update (for example `funding_confirming` with one more confirmation, or a new `exit.phase`): it does not add a history entry. `patch` merges one level deep into the state, `null` clears a field, `addTxs` adds or replaces transactions by `id`. |
| `{ type: 'activity', swapId, activity }` | What the engine is doing now: `{ kind: 'proving' \| 'syncing' \| 'broadcasting' \| 'signing' \| 'waiting', label, startedAt, estimatedMs? }`, or `null`. A new stage clears it. |
| `{ type: 'error', swapId?, error }` | A problem not tied to a call. Without `swapId` it is about the browser (for example `STORAGE`). |

The engine persists every change **before** it emits it, so a reload never shows a state
that storage does not have.

## Stages

Normal path, after docs/DESIGN.md §3. "Driver" says who moves the swap forward from that
stage: `taker` steps need the page open; `network` steps happen anyway.

| Stage | Meaning | Driver of the next step | Tab |
|---|---|---|---|
| `awaiting_deposit` | Waiting for the user's payment. `deposit.receivedZats` shows partial payments; `deposit.uri` then asks for the rest. | user | may close |
| `deposit_seen` | The payment is seen, not yet spendable. | network | may close |
| `deposit_confirmed` | Spendable. The request proof starts. | taker | may close |
| `request_sent` | Request memo sent (send 1 of 3). | network (maker) | may close |
| `quote_received` | Firm quote `quote.usdc` ≥ `minUsdc`. The acceptance proof starts. | taker | may close |
| `accepted` | Acceptance and premium sent (send 2 of 3). | network (maker) | may close |
| `escrow_open` | Escrow opened, 0.03 NEAR stipend received (`escrow`). The funding proof starts. | taker | may close |
| `funding_sent` | ZEC sent to the shared address (send 3 of 3). `deadlines` set. | network | come back before `deadlines.readyBy` |
| `funding_confirming` | `funding.confirmations` of `funding.required`. | network | come back before `deadlines.readyBy` |
| `ready` | The maker set the escrow to Ready. | taker | **keep open** |
| `committing` | `commit_claim` sent. `claim.punishPath` is true when the maker vanished and the deadline passed. | taker | **keep open** |
| `claiming` | `claim(a)` sent. | network | **keep open** |
| `claimed` | The claim is final. | taker (withdraw) | may close |
| `paid_out` | USDC is in `intents.near` under the taker key (`payout`). The engine starts the exit by itself. | taker | may close |
| `exiting` | The exit runs; `exit.phase` says where (below). | taker, then 1Click | may close |
| `done` | Delivered (`exit.deliveredAmount`). A `leftover` may follow to the return address. | – | – |

Failure paths:

| Stage | Meaning |
|---|---|
| `refunding` | ZEC goes back to the return address. `refund.reason` is `QUOTE_BELOW_MINIMUM`, `MAKER_TIMEOUT` or `MAKER_REFUNDED`. Without a return address: `needs: 'return_address'`. |
| `refunded` | Sent: `refund.amountZats` to `refund.toAddress`. Terminal. |
| `recovering` | The maker refunded the escrow, which published key half `b`; the engine spends the shared address with `a + b`. |
| `recovered` | The ZEC is back in the temporary wallet; `refunding` follows. |
| `exit_retry` | The exit stopped. The USDC is under the taker key. `lastError.code` is `EXIT_BELOW_MINIMUM` (with `exit.offered`), `EXIT_FAILED` (1Click `FAILED` or `REFUNDED`), `EXIT_AMOUNT_TOO_LOW` or `EXIT_REFUSED`. `needs: 'exit_decision'`. The page calls `exit()` or `exit({ acceptMinimum })`. |

`needs` is what the engine waits for from the user: `'return_address'`, `'exit_decision'`
or `null`.

## The exit

The exit follows docs/INTENTS.md §2 and the `zecret-intents` crate. The engine owns every
step; the page only shows `exit.phase` and decides in `exit_retry`.

| `exit.phase` | Engine work (docs/INTENTS.md) | State fields |
|---|---|---|
| `quoting` | Firm `POST /v0/quote` (`dry: false`, 100 bp slippage, refund to the taker's Intents account). On `Quoting for this pair is not available` it sets `lastError` `EXIT_NO_QUOTE` with `transient: true` and asks again after 20 s. If `minAmountOut` < `exit.approvedMinimum`: stage `exit_retry`, `EXIT_BELOW_MINIMUM`, `exit.offered`. Nothing is signed. | `amountIn`, `amountOut`, `minAmountOut`, `depositAddress`, `timeEstimateS`, `feeBps` |
| `signing` | Quote signature and field check, `generate-intent`, payload check, one ed25519 signature. A failed check: `exit_retry` with `EXIT_REFUSED`. | – |
| `submitted` | `submit-intent` returned `intentHash` (checked against the signed payload). | `intentHash`, a `TxRef` |
| `delivering` | Polling `GET /v0/status` until final. `SUCCESS`: `done`. `REFUNDED` / `FAILED`: `exit_retry`, `EXIT_FAILED`, `exit.phase: 'refunded' \| 'failed'`. | – |
| `done` | Delivered. | `deliveredAmount`, a `destination` `TxRef` |

`exit.approvedMinimum` starts as `params.minDestinationAmount`. `exit.attempts` counts
firm quotes. A signed exit stays valid for about 72 hours; the real engine keeps polling
every deposit address it signed for (docs/INTENTS.md §6).

## Errors

| Code | Meaning |
|---|---|
| `INIT_FAILED` | No WebAssembly, no worker storage, a bad directory, or the key build failed. |
| `NOT_READY` | A call before `init`. |
| `INVALID_PARAMS` | Bad arguments. |
| `INVALID_ADDRESS` | An address failed the engine's own check. |
| `SWAP_EXISTS` | This browser has an unfinished swap. |
| `NO_SWAP` | The call needs a swap. |
| `WRONG_STAGE` | Not allowed in this stage. |
| `RECOVERY_INVALID` | The file cannot be read, or belongs to another engine. |
| `RECOVERY_MISMATCH` | The file is for another network. |
| `QUOTE_BELOW_MINIMUM` | The maker's firm quote was below `minUsdc`. Refund path. |
| `MAKER_TIMEOUT` | No quote in time. Refund path. |
| `MAKER_REFUNDED` | The maker refunded the escrow instead of setting it ready. Recovery path. |
| `EXIT_FAILED` | 1Click reported `FAILED` or `REFUNDED`. |
| `EXIT_BELOW_MINIMUM` | The firm exit quote is below the approved minimum. |
| `EXIT_NO_QUOTE` | 1Click had no quote for a moment. Transient; the engine retries. |
| `EXIT_AMOUNT_TOO_LOW` | 1Click: "Amount is too low for bridge". Another destination is needed. |
| `EXIT_REFUSED` | The safety check refused the quote or payload. Nothing was signed. |
| `NETWORK` | A server did not answer. Transient; the engine retries. |
| `STORAGE` | The browser refuses storage. |
| `INTERNAL` | A bug. |

## The recovery file

The engine defines the content; the page treats it as opaque text. It holds the
temporary wallet's spending key, key half `a`, the NEAR key, the swap parameters and the
network, and it is plain JSON (`format: "zecret-recovery"`, `version: 1`, `engine`). The
page offers it right after `createSwap`, before any money moves, and again on every
progress screen. Anyone with the file can move the swap's funds; the page says so.

The simulated engine's file also carries a snapshot of its simulated chain, because a
simulated swap has no real chain to rescan. The real engine rescans the chain instead.

## The simulated engine

`src/engine/sim/` implements the same backend with no network and no money. It keeps
two IndexedDB stores: `local` (this browser's keys) and `chain` (what the networks and
the maker know), so "a cleared browser" can be simulated. It plays the timeline of
`timeline.ts` divided by `speed`, pauses taker steps while the page is closed, and
catches up network steps when the page comes back.

Page URL parameters (read by `src/app/settings.ts`):

| Parameter | Effect |
|---|---|
| `speed=N` | Time multiplier, 1 to 10000. Default 1 (a swap takes about 10 minutes). |
| `fail=quote-low` | The maker quotes 3 % below `minUsdc`: refund. |
| `fail=no-quote` | The maker never answers: refund after the timeout. |
| `fail=maker-quiet` | No Ready; the maker refunds after `readyBy`: recovery, then refund. |
| `fail=maker-gone` | No Ready and no refund: claim through the punish path after `punishFrom`. |
| `fail=exit-busy` | The first firm exit quote answers "no quote"; the engine asks again. |
| `fail=exit-price` | The first firm exit quote is 15 % lower: `EXIT_BELOW_MINIMUM`. |
| `fail=exit-fail` | The first delivery is refunded: `EXIT_FAILED`. |
| `cpu=1` | The worker busy-loops during simulated proofs, like the real prover. |
| `pause=<stage>` or `pause=exiting.<phase>` | Stop there until `sim.skip()` ("Skip this wait" in the demo controls). |

`SimControls` (only the simulator answers): `pay(amountZats?)`, `skip()`, `clearLocal()`.
The simulator refuses to run on mainnet, so its deposit address is never a mainnet
address.
