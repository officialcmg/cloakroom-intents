# Engine interface

The page and the swap engine talk through one small TypeScript interface. The engine
runs in a Web Worker. The page never touches keys, the Zcash wallet, NEAR or the signed
exit; it shows what the engine reports and passes on what the user decides.

- Types: `src/engine/api.ts` (the contract; this file explains it).
- Transport: `src/engine/rpc.ts` (postMessage, any port: `Worker`, the worker's own
  scope, or a `MessageChannel` in tests).
- Fold events into state: `src/engine/reducer.ts` (used by the page and by the simulator).
- Simulated engine: `src/engine/sim/`.
- Real engine: `src/engine/real/` (the worker side of `crates/zecret-wasm`).
- Typical durations: `src/engine/timeline.ts`.

## Which engine runs

`src/engine/index.ts` starts one of two workers. Both call `serveEngine(self, backend)`
from `rpc.ts` with an `EngineBackend` (the `Engine` methods plus `attach(emit)`).

| Engine | When | Worker |
|---|---|---|
| Real (`InitStatus.engine: 'wasm'`) | The page has a network configuration: `?config=local` or `?config=testnet` (any other name or URL only in a development build or one made with `VITE_ZECRET_ALLOW_CONFIG_URL=1`, with a warning on the page), or `VITE_ZECRET_CONFIG` at build time. `?demo=1` overrides both. | `src/engine/real/real.worker.ts` |
| Simulated (`'simulated'`) | No configuration: the demo. | `src/engine/sim/sim.worker.ts` |

The network configuration (`src/lib/netconfig.ts`, shipped as `public/config/local.json`
and `public/config/testnet.json`) names the Zcash endpoint (Zebra JSON-RPC through a CORS
proxy, or lightwalletd gRPC-web), the NEAR RPC, the escrow and its expected code hash,
the token and its decimals, the Verifier, the maker directory and the exit mode. The page
passes it to the engine as `EngineConfig.real`. The real engine does not answer `sim.*`
(`INVALID_PARAMS`).

The real engine's WebAssembly is built by `crates/zecret-wasm/build.sh` into
`public/engine/` (not in git) and loaded by the worker at run time, so the page
type-checks, tests and builds without it.

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
| `init(config)` | once, first | Checks the maker directory, starts the proving-key build, resolves at once with `InitStatus { engine, phase: 'building_keys', startedAt, estimatedMs, timeScale }`. A later `init` event says `phase: 'ready'`. Throws `INIT_FAILED`. The real engine also loads its WebAssembly and checks the directory entry's signature and that the maker's key is a full-access key of its NEAR account. Its key build (about 20 s, blocking) starts one second after `init`, after `resume` has answered, and not while a resumed swap is between the funding and the final claim (it would delay a NEAR deadline); a `createSwap` sent during the build answers after it. |
| `subscribe(listener)` | any time | All events. Returns an unsubscribe function. (Page side only; not a wire call.) |
| `resume()` | after `init` | Loads the stored swap, if any, emits `swap`, and continues it (the real engine rescans from the wallet's birthday first). Returns the state or `null`. |
| `getState()` | any time | The current `SwapState` or `null`. |
| `createSwap(params)` | no unfinished swap | Makes the temporary wallet, key half `a` and the NEAR key; **stores them before it returns**; returns the state in `awaiting_deposit` with `deposit.address` and the ZIP 321 `deposit.uri`. Throws `SWAP_EXISTS`, `LEFTOVER_PENDING` (a finished swap still owes leftover ZEC), `INVALID_PARAMS`, `INVALID_ADDRESS`. |
| `setReturnAddress(addr)` | until the refund or leftover is sent | Unified address with an Orchard receiver on the swap's network. Clears `needs: 'return_address'`. |
| `exportRecovery()` | a swap exists | Returns `{ fileName, mimeType, text }`. Sets `recoveryExportedAt`. The page saves the text as is. |
| `importRecovery(text)` | no other unfinished swap | Restores and continues the swap. The real engine checks the file's addresses as `createSwap` does and computes `deposit.address` and `deposit.uri` from the file's wallet key; it sets `restoredAt`. Throws `RECOVERY_INVALID`, `RECOVERY_MISMATCH`, `SWAP_EXISTS`, `LEFTOVER_PENDING`, `INVALID_ADDRESS`. |
| `exit(request?)` | `exit_retry` | Runs the exit again from a fresh firm quote. `request.acceptMinimum` lowers the approved minimum to the `minAmountOut` of `exit.offered`, which the user saw and accepted; it can never raise it. In `paid_out` or `exiting` it is a no-op (the engine runs the exit by itself). |
| `forget(options?)` | before any payment arrives, or after `done` / `refunded` | Deletes the swap and its keys from this browser. Otherwise `WRONG_STAGE`. While `leftoverPending(swap)` (leftover ZEC with no return address, or its transaction not yet mined; `leftover.returned` marks it mined) it throws `LEFTOVER_PENDING` unless `options.discardLeftover` is true. The real engine also throws it (transient) at `done` until its leftover step has looked at the wallet once. |

`CreateSwapParams`:

| Field | Meaning |
|---|---|
| `amountZats` | What the user pays into the temporary wallet. |
| `destination` | `{ assetId, symbol, chain, decimals, address }`: 1Click asset id and blockchain id, and the recipient. The engine checks the address again. |
| `returnAddress?` | For refunds and leftovers. Optional; the engine asks (`needs`) when it needs one. |
| `minUsdc` | The engine accepts the maker's firm quote only at or above this. Below it, no premium is paid and the ZEC goes back (`QUOTE_BELOW_MINIMUM`). |
| `minDestinationAmount` | The approved minimum for the exit. The engine signs no exit whose firm `minAmountOut` is below it. |
| `directExit?` | `{ nearAccount }`. Required on networks whose exit mode is `direct` (local, testnet): the NEAR account that receives the stand-in token. `destination` then only names the mainnet preview, and its `address` may be empty. |
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
that storage does not have. The real engine writes the core's changed records to
IndexedDB after every call into the WebAssembly and waits for the transaction to commit
before the next call; the core never broadcasts or reveals in the call that created the
record it depends on.

When a write fails, the real engine keeps the records it could not write and emits one
`error` without `swapId`, code `STORAGE`, `transient: true`. From then on it calls nothing
in the WebAssembly: every call and every loop step first retries the same write, and a call
is refused with `STORAGE` while the write fails. The next event after that error means the
write succeeded.

One tab runs the engine. The page takes the Web Lock `zecret-engine` (demo:
`zecret-engine-demo`) before it starts the worker. The real engine's store also counts
writes in `web/version` and refuses a write from a tab whose copy is older; that engine
then stops and emits `OTHER_TAB`. A WebAssembly trap (`RuntimeError`) stops the engine at
once, no later call reaches the trapped instance, and the engine emits `ENGINE_CRASHED`;
the page ends the worker and starts a new one, which resumes from storage.

### How the real engine makes the website's stages

| Stage | Source |
|---|---|
| `awaiting_deposit` | `createSwap`; `deposit.receivedZats` from the core's `balance` events. |
| `deposit_seen`, `deposit_confirmed` | Both when the balance reaches the deposit: the wallet sees mined notes only, and a mined note is spendable. |
| `request_sent`, `accepted` | Core stages `RfqSent`, `Accepted` (with the `zcash_tx` events). |
| `quote_received` | Core `quote` event. |
| `escrow_open` | Core `escrow_verified` event; `deadlines.readyBy` = t0, `punishFrom` = t2 (NEAR block time). |
| `funding_sent` | Core `Funded`. |
| `funding_confirming` | The wallet's scan height minus the funding's height, up to k_Z. |
| `ready` | The escrow reads `Ready` at a final block once the funding has k_Z confirmations, or the core commits (not on the punish path). |
| `committing`, `claiming`, `claimed`, `paid_out` | Core `Committed`, `Revealed`, `Claimed`, `Completed` (with `payout`). `claim.punishPath` from the core's log. Core `PayoutPending` (the claim is final, but the maker has not sent `withdraw` within 5 minutes) stays on `claimed` with a `waiting` activity that names the call any NEAR account can send: the escrow's `withdraw` for the taker key, 200 Tgas. |
| `recovering`, `recovered` | Core `Recovering`, `Recovered`. |
| `refunding`, `refunded` | After `Recovered` or `Aborted`: `returnLeftover` to the return address; `refunded` once that transaction is mined. When the node refuses a refund or leftover send, the engine opens the swap again from storage before it retries: the wallet keeps the notes of a built transaction reserved until it expires, so without that the balance would read 0 and the step would end with nothing sent. A core `Aborted` reason that starts with `MAKER REFUSED` (the maker's signed refusal memo: over its limit, under its minimum, busy, unsupported) becomes `refund.reason` and `lastError` `MAKER_CHECK_FAILED`, with the maker's reason in the message. |
| `exiting`, `exit_retry`, `done` | The exit (1Click or direct) after `Completed`. |
| `paid_out`, `done` with `exit.doneElsewhere` | Core `Claimed` or `PayoutPending`, while the escrow shows the payout made (swap state `Claimed`, the taker's Intents credit 0) and the taker key's Verifier balance reads 0, at one final block, for 30 s: the exit happened before this browser followed the swap (a restore after the exit). `payout.usdc` is the escrow's amount; `deliveredAmount` and the delivery transaction are not set. The leftover step follows. |

Activities: `proving` before each proof (from the core's `activity` event, and before a
refund), `waiting` while the swap waits on a chain or the maker, `syncing` after a reload or
a restore, `signing` and `broadcasting` during the exit.

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
| `claimed` | The claim is final. The maker sends `withdraw`; if it does not within 5 minutes, a `waiting` activity names the call that any NEAR account can send. | network (maker) | may close |
| `paid_out` | USDC is in `intents.near` under the taker key (`payout`). The engine starts the exit by itself. | taker | may close |
| `exiting` | The exit runs; `exit.phase` says where (below). | taker, then 1Click | may close |
| `done` | Delivered (`exit.deliveredAmount`), or `exit.doneElsewhere`: the exit happened before a restore, and the engine did not see the delivery. A `leftover` may follow to the return address; a reload at `done` sends it if it was not sent. | – | – |

Failure paths:

| Stage | Meaning |
|---|---|
| `refunding` | ZEC goes back to the return address. `refund.reason` is `QUOTE_BELOW_MINIMUM`, `MAKER_TIMEOUT`, `MAKER_CHECK_FAILED` or `MAKER_REFUNDED`. Without a return address: `needs: 'return_address'`. |
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
| `done` | Delivered. | `deliveredAmount`, a `destination` `TxRef`; or `doneElsewhere: true` without them (see "How the real engine makes the website's stages") |

`exit.approvedMinimum` starts as `params.minDestinationAmount`. `exit.attempts` counts
firm quotes. A signed exit stays valid for about 72 hours; the real engine keeps polling
every deposit address it signed for (docs/INTENTS.md §6). It records the address before it
submits, so a lost `submit-intent` answer still leaves it to poll; an earlier address that
reports `SUCCESS` finishes the swap, also before a new firm quote. Explorer links from
1Click are kept only when they are `https:`.

## Errors

| Code | Meaning |
|---|---|
| `INIT_FAILED` | No WebAssembly, no worker storage, a bad directory, or the key build failed. |
| `NOT_READY` | A call before `init`. |
| `INVALID_PARAMS` | Bad arguments. |
| `INVALID_ADDRESS` | An address failed the engine's own check. |
| `SWAP_EXISTS` | This browser has an unfinished swap. |
| `LEFTOVER_PENDING` | The finished swap's leftover ZEC has not reached the return address; deleting the swap would leave it only in the recovery file. |
| `NO_SWAP` | The call needs a swap. |
| `WRONG_STAGE` | Not allowed in this stage. |
| `RECOVERY_INVALID` | The file cannot be read, or belongs to another engine. |
| `RECOVERY_MISMATCH` | The file is for another network. |
| `QUOTE_BELOW_MINIMUM` | The maker's firm quote was below `minUsdc`. Refund path. |
| `MAKER_CHECK_FAILED` | The maker's quote or escrow failed another of the engine's checks (signature, proof of knowledge, windows, escrow parameters, code hash, stipend), or the maker refused the request with a signed refusal memo (the message gives its reason). Nothing was funded. Refund path. |
| `MAKER_TIMEOUT` | No quote in time. Refund path. |
| `MAKER_REFUNDED` | The maker refunded the escrow instead of setting it ready. Recovery path. |
| `EXIT_FAILED` | 1Click reported `FAILED` or `REFUNDED`. |
| `EXIT_BELOW_MINIMUM` | The firm exit quote is below the approved minimum. |
| `EXIT_NO_QUOTE` | 1Click had no quote for a moment. Transient; the engine retries. |
| `EXIT_AMOUNT_TOO_LOW` | 1Click: "Amount is too low for bridge". Another destination is needed. |
| `EXIT_REFUSED` | The safety check refused the quote or payload. Nothing was signed. |
| `NETWORK` | A server did not answer. Transient; the engine retries. |
| `STORAGE` | The browser refuses storage. With `transient: true`: a write failed, and the engine does nothing until it succeeds. |
| `OTHER_TAB` | Another tab of this browser changed the swap; this tab's engine stopped. |
| `ENGINE_CRASHED` | The WebAssembly engine trapped; the page restarts it from storage. |
| `INTERNAL` | A bug. |

## The direct exit (networks without 1Click)

NEAR Intents has no testnet, and 1Click does not see a copy of the Verifier. On networks
whose configuration says `exit.mode: "direct"` (local and testnet), the exit is:

| `exit.phase` | Engine work | State fields |
|---|---|---|
| `signing` | Reads the taker key's Verifier balance, checks that the NEAR account exists and is registered with the token, fixes a deadline (1 hour) and a versioned nonce, and stores this plan. | `direct`, `amountIn` = `amountOut` = `minAmountOut` (the token's base units), `depositAddress` (the NEAR account), `intentHash` |
| `delivering` | Signs one `ft_withdraw` with the taker key and hands it to the maker's relay (`zecret maker --relay`), which sends `execute_intents` and pays the gas. Resending after a reload sends the same signed bytes; a used nonce means it executed. | a `TxRef` for `execute_intents` |
| `done` | The receiver's token balance rose by the amount, or (a receiver that moves the tokens on at once) the taker's Verifier balance reads zero for 15 s. | `deliveredAmount` |

The taker's own NEAR account cannot send `execute_intents`: an `ft_withdraw` needs 40 Tgas
attached (35 fails), and what is left of the 0.03 NEAR stipend pays for at most about
25 Tgas at the pessimistic price NEAR checks first (measured in
`crates/zecret-wasm/tests/sandbox.rs`). The relay sends only a payload with exactly one
`ft_withdraw` of the maker's token, for at most the swap's price, signed by the taker of
one of its maker's swaps that paid out (maker stage `Sweeping` or `Swept`, or a recorded
`withdraw` for the taker), that `simulate_intents` accepts; at most 3 transactions per swap
and 6 requests a minute per address; never the same payload twice. It answers 503 (the
engine retries) while the maker has not seen the claim yet, when it is busy, or when an
address asks too often, and 400 for a refusal.

The page still asks 1Click (mainnet) for a price for the chosen token and chain and shows
it as a labelled preview.

## Units on networks with a stand-in token

The page counts the maker's side in micro-USDC (6 decimals). Where the token has more
decimals (wNEAR on the local network: 24), `minUsdc`, `quote.usdc` and `payout.usdc` are
that token's amounts scaled to 6 decimals; the engine multiplies by 10^(decimals − 6)
before it compares with the maker's quote. The page shows the token's symbol.

## The recovery file

The engine defines the content; the page treats it as opaque text. It is plain JSON
(`format: "zecret-recovery"`, `version: 1`, `engine`). The page offers it right after
`createSwap`, before any money moves, and again on every progress screen. Anyone with the
file can move the swap's funds; the page says so. It is not encrypted with a passphrase:
a passphrase the user forgets loses the funds, and the file is meant to be kept like a
wallet backup.

The simulated engine's file also carries a snapshot of its simulated chain, because a
simulated swap has no real chain to rescan.

The real engine's file (`engine: "wasm"`) holds:

| Field | Content |
|---|---|
| `records.wallet` | The temporary Zcash wallet's spending key. |
| `records["swap/<id>"]` | The core's swap record: key half `a`, the seeds `s_T` and `e_T`, the NEAR key seed `near_sk` (it owns the payout in the Verifier), the amount and the minimum price. |
| `web` | The page's swap state and the worker's own record (destination, return address, exit progress). |
| `config` | The escrow, its code hash, the token, the Verifier and the maker. A file for another deployment is refused (`RECOVERY_MISMATCH`). |

Everything else is read back from the chains. A file saved before the payment is enough:
after `importRecovery` the engine rescans the temporary wallet and waits for two new Zcash
blocks while it reconciles the record with the chains (`Taker::reconcile` in the core):
a mined send of the wallet means the request went out; a send at or after the quote's
height means the acceptance went out; a note at the joint address means the funding went
out. Only then does it act, so nothing is sent twice. A transaction that is still in the
mempool after those two blocks is not seen. A swap restored after its exit finished goes
to `done` with `exit.doneElsewhere` (the escrow paid the taker key and the tokens left it)
and then sends the leftover ZEC, if any, to the return address.

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
