# The swap engine (`crates/zecret-core`)

This document is for an engineer who wraps the taker engine in a browser Web Worker (or
any other host). It describes the public interface of the shared core: the three
interfaces to the outside world, the taker's configuration, its stages and the events it
emits. The maker uses the same interfaces; it is summarised at the end.

The same Rust code runs natively (`crates/zecret-node`: the maker daemon and a reference
taker, used by `harness/run.py`) and as WebAssembly (`wasm32-unknown-unknown`, with
`orchard` and `zcash_primitives` built without `multicore`).

## 1. The three interfaces

### 1.1 Network: `net::Transport`

```rust
pub trait Transport {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> anyhow::Result<Response>;
}
pub struct Response { pub status: u16, pub body: Vec<u8>, pub headers: Vec<(String, String)> }
pub type SharedTransport = std::rc::Rc<dyn Transport>;
```

- One blocking HTTP POST. Return `Err` only when no HTTP response arrived (network error,
  timeout). An HTTP error status is an `Ok(Response)`; the clients read the body.
- Header names in `Response.headers` must be lower case. The lightwalletd client reads
  `grpc-status` from the headers if the server sends it there (trailers-only responses), so
  a gRPC-web server must expose it with CORS (`Access-Control-Expose-Headers: grpc-status,
  grpc-message`).
- In a Web Worker, implement it with a synchronous `XMLHttpRequest`
  (`open("POST", url, false)`, `responseType = "arraybuffer"`), as
  `spike/browser-wallet/src/net.rs` does. Synchronous XHR is allowed in workers only.
- Everything goes through it: Zebra JSON-RPC or lightwalletd gRPC-web (Zcash) and NEAR
  JSON-RPC. The engine never opens a socket.

While a request runs, the worker cannot answer the page. One shielded send also blocks
the worker for about 25 s (proof). The page must show progress from the events it has
already received, not by asking the worker.

### 1.2 Storage: `store::Store`

```rust
pub trait Store {
    fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>>;
    fn put(&mut self, key: &str, value: &[u8]) -> anyhow::Result<()>;
    fn keys(&self, prefix: &str) -> anyhow::Result<Vec<String>>;   // sorted
}
```

Keys the taker uses:

| Key | Value | Notes |
|---|---|---|
| `wallet` | hex of the 32-byte Orchard spending key (UTF-8 text) | Created by `Taker::open` on first use. The temporary wallet. |
| `swap/<rfq_id>` | JSON `TakerSwap` | One per swap. Holds the key share `a`, the seeds and the NEAR key seed while the swap runs. |

The browser cannot block on IndexedDB, so use `store::MemStore`:

1. At worker start, read every record from IndexedDB and `MemStore::load(key, bytes)` it.
2. Create the engine with `&mut mem_store`.
3. After every `tick` (and after `start`), call `mem_store.take_dirty()` and write each
   `(key, bytes)` to IndexedDB. Wait for the write to complete before the next `tick`.

The engine is built for this: it never uses a new secret, or sends a message that depends
on a new record, in the same call that created it. Examples: `start` creates `a` and the
NEAR key and returns; the request memo goes out in a later `tick`. A Zcash transaction is
built (and stored with the record) in one tick and broadcast in the next. `refund(b)` is
decided in one tick and sent in the next. `claim(a)` takes three: one tick decides to
reveal (stage `Revealed`), the next signs the claim and stores it (`claim_raw`), and the
next checks R2r again and broadcasts the stored bytes (security review 2, F2-02).

**Recovery file.** The records are the recovery file: `wallet` plus `swap/<rfq_id>`. A host
that restores them into a fresh store and opens the engine continues the swap.
`TakerSwap.near_sk` is the seed of the NEAR key that owns the payout inside
`intents.near`; it stays in the record after the swap ends.

A recovery file is usually saved before the payment, so its record says `WaitingDeposit`
while the request, the acceptance or the funding may already be on chain. After a
restore, call `taker.reconcile(now, store)` (it syncs the wallet) until the chain has
advanced by a few blocks, and only then `tick`. It moves the record forward from the
chain: a mined send of the temporary wallet means the request was sent; a send at or after
the quote's height means the acceptance was sent; a note at the joint address means the
funding was sent. A quote that fails the checks is left for `tick`. As a second guard,
`Accepted` never funds a joint address that already holds a note. The browser engine
waits for two new blocks (`web/src/engine/real/realEngine.ts`).

### 1.3 Time: `tick(now)`

The engine reads no clock and never sleeps.

```rust
pub fn tick(&mut self, now: u64, store: &mut dyn Store) -> engine::Tick;

pub struct Tick { pub next: Next, pub events: Vec<Event> }
pub enum Next { At(u64), Done }      // JSON: {"kind":"at","at":1794540785} / {"kind":"done"}
```

- `now` is the wall-clock Unix time in seconds (`Math.floor(Date.now() / 1000)`). It is
  used for timeouts and cover delays only.
- Protocol deadlines (t0, t1, t2, the reveal window) use NEAR block time, which the engine
  reads from final blocks itself.
- `Next::At(t)`: call `tick` again at time `t` or later. `t <= now` means "at once" (the
  engine has more to do, for example to broadcast what it just stored). `Next::Done`: the
  swap is in a terminal stage (or there is no swap).
- `tick` never fails. A network or node error becomes an `Event::Warning` and a retry time
  a few seconds later.
- A Zcash sync error stops only the stages that need the Zcash wallet. In `Funded`,
  `Committed`, `Revealed`, `Claimed` and `PayoutPending` the tick emits a `warning` and
  goes on with the NEAR steps. A refund seen with an old Zcash view does not end the
  swap: the recovery waits for a good sync.
- The light wallet trusts its server (security review 2, H8) for the content of blocks.
  It checks that the chain it scans is one chain and follows reorganisations; see §1.4.
  It relies on k_Z confirmations and an honest light-wallet server.

### 1.4 The Zcash wallet: sync, reorganisations, repair

`wallet::Wallet` keeps the Ironwood note commitment tree from its birthday (the server's
tree state just before it), the notes of its viewing keys, the nullifiers of every scanned
action and the heights of tracked transactions. Each transaction it builds is anchored in
that tree. A node refuses a transaction whose anchor is not the tree of a block in its
chain ("unknown Ironwood anchor"). So the tree must be the chain's tree.

Sync (`Wallet::sync`), for both backends (lightwalletd and Zebra JSON-RPC):

1. **One block at a time, all or nothing.** The wallet reads a block and makes every
   network call it needs (the full transaction of an own note) before it changes
   anything. Then it applies the whole block. A failure leaves the wallet at the last
   block it applied: an older view, never a wrong one. A stream that ends early (fewer
   blocks than asked, or a cut frame) skips nothing; the next sync goes on from the last
   applied block.
2. **Each block links to the one before.** The wallet keeps the hash and the tree size of
   the last 100 blocks (`KEEP_BLOCKS`). A block whose `prev_hash` is not the hash of the
   last applied block means a chain reorganisation. The wallet asks the server for the
   hashes of the kept blocks, rolls back to the newest common block (leaves, received
   notes, nullifiers, look-alike counts and mined heights above it are removed), and then
   scans the new chain. A tip that the server replaced at the same height is found by
   comparing the server's tip hash with the kept one. With no common block in the kept
   range, the wallet rescans from its birthday.
3. **The tree size must match the chain.** A compact block states the Ironwood tree size
   after it (`chain_metadata`). A block that does not give that size is not applied. If
   the wallet's own size already differs from the server's tree at the wallet's height,
   the wallet's tree is wrong, and it rescans from the birthday.

Anchor: the tree after the block `ANCHOR_DEPTH` (2) below the tip, or after the newest
block that holds a spent note if that is higher. Consensus accepts the root of any block
of the chain, so a new note can still be spent at once. A block at the tip is the one
that a reorganisation replaces most often.

Repair (`Wallet::repair`), automatic in `Wallet::broadcast`:

- A refused transaction that no node took before frees the notes it spends at once (they
  were reserved by the build).
- A refusal for an unknown anchor starts a repair: sync, compare the kept block hashes
  with the server's and roll back to the newest common block, then compare the tree root
  with the server's tree at the same height (`GetTreeState`). If the roots differ, the
  wallet rescans from its birthday. It rescans at most once per tip height; the old state
  comes back if the rescan fails. On Zebra (no tree state to compare) it rescans.
- `Wallet::pay` builds the payment once more after a repair. The maker and the taker
  rebuild a refused transaction on their next tick, as before; the rebuild uses the
  repaired tree.
- The repair and the rollbacks write lines that `Wallet::take_notes` returns. The maker
  and the taker put them in their events as `Log` lines that start with `Zcash wallet:`.

A rescan from the maker's testnet birthday (4,614,922; 1,453 blocks on 2026-11-13) takes
about 1.3 s natively; 30,000 blocks (about one week of testnet blocks) took 41 to 44 s
(`zecret balance` with a new key, 2026-11-13).

## 2. The taker API (`taker::Taker`)

```rust
let mut taker = Taker::open(cfg: TakerConfig, transport: SharedTransport, store: &mut dyn Store)?;
taker.ua()                       // the temporary wallet's unified address (show as QR code)
taker.balance()                  // spendable zatoshis as of the last tick
taker::required_deposit(zats, cfg.max_premium)  // what the user must pay into the wallet
taker.start(zats, now, store)?   // checks the maker's directory entry; creates the swap
taker.start_with_min_price(zats, min_price, now, store)?  // the same, with this swap's minimum price
taker.take_events()              // events from `start`
taker.tick(now, store)           // run the state machine
taker.swap()                     // Option<&TakerSwap>: the current record
taker.return_leftover(ua)?       // after the swap: send what is left to the user's address
taker.reconcile(now, store)?     // after a restore from an early recovery file (§1.2)
wallet::prepare_proving_key()    // build the Orchard proving key now (about 20 s in a browser)
```

- `Taker::open` resumes the swap in progress if the store holds one that is not terminal.
- `start` fails (and stores nothing) if a swap is in progress, if the maker's directory
  entry is not signed by its NEAR key, or if that key is not a full-access key of the
  maker's NEAR account at a final block. Nothing is sent and nothing is paid then.
- If the entry publishes `limits`, `start` also fails when the amount is outside
  `min_zats..=max_zats` or `premium_zats` is above `max_premium`.
- Call `prepare_proving_key` at worker start, before the user pays. Otherwise the first
  shielded send (the request) builds it.
- A worker loop:

```js
// worker.js (sketch)
let next = 0;
async function loop() {
  const t = engine.tick(Math.floor(Date.now() / 1000));   // wasm-bindgen wrapper; blocks
  await persist(store.take_dirty());                       // IndexedDB, before the next tick
  for (const e of t.events) postMessage(e);
  if (t.next.kind === "done") return;
  setTimeout(loop, Math.max(0, t.next.at * 1000 - Date.now()));
}
```

### 2.1 Configuration (`config::TakerConfig`, JSON)

Amounts above 2^53 are decimal strings. The `//` comments explain fields; they are not valid JSON.

```json
{
  "chains": {
    "lightwalletd": "https://zcash-testnet.example/grpc-web",  // or "zebra_rpc": "http://127.0.0.1:18232"
    "birthday": 3000000,                 // first Zcash block to scan (lightwalletd only)
    "network": "test",                   // "regtest" (default) | "test"
    "near_rpc": "https://rpc.testnet.near.org",
    "escrow": "escrow.zecret.testnet",
    "token": "usdc.fakes.testnet",
    "intents": "intents.near"
  },
  "timing": {"inclusion_secs": 30, "finality_secs": 10, "reorg_budget": 2, "margin_secs": 0,
             "fund_secs": 600, "k_z": 3},
  "maker": {"ua": "utest1...", "epoch": 1, "rfq_pub": "<hex>", "near_account": "maker.testnet",
            "near_pk": "ed25519:...", "sig": "<hex, 64 bytes>"},
  "escrow_code_hash": "<base58 sha256 of the reviewed escrow wasm>",
  "min_price": "4990000",                // smallest price (token base units) the user accepts
  "max_premium": 100000,                 // zatoshis
  "min_stipend": "30000000000000000000000",   // 0.03 NEAR
  "max_horizon_secs": 172800,            // R1q horizon H (default 48 h)
  "quote_timeout_secs": 600,             // wait for the quote, and for the escrow after accepting
  "payout_wait_secs": 300,               // after the claim is final: when to report PayoutPending
  "cover": {"min_secs": 0, "max_secs": 0, "k": 0}   // cover delays (off by default)
}
```

`timing` holds the bounds of the paper: D (`inclusion_secs`), F (`finality_secs`), R
(`reorg_budget`), ∆fund (`fund_secs`: k_Z Zcash confirmations at a high percentile of
block intervals, plus D) and k_Z. B_R = (R+1)·D + R·F + `margin_secs`. The taker uses its
own values to check the maker's quote (R1q).

`escrow_code_hash` is the code hash (as `view_account` reports it) of the reviewed escrow
build; `zecret devnet` prints it for the local network, and `near::tx::code_hash(wasm)`
computes it. `chains.intents` is also the Intents account the escrow must pay into.

The `maker` entry is what the maker daemon prints with `zecret directory`. It has an
optional `limits` object (`{"min_zats", "max_zats", "premium_zats"}`). When it is present,
`sig` covers it (`keys::directory_digest_v2`, domain `maker-directory-near-v2`); an entry
without it is signed with the first digest. Removing or changing the limits breaks the
signature. `misbehave` is a test-only field; leave it out. A build without the cargo
feature `misbehave` refuses it, and the taker's test-only code is not compiled into such
a build.

### 2.2 Stages

`TakerSwap.stage` (serialized as a string, or `{"Aborted": "<reason>"}`). The `Stage` event
carries the name.

| Stage | Meaning | Leaves when | Next stage |
|---|---|---|---|
| `WaitingDeposit` | The swap exists; the wallet waits for `required_deposit` zatoshis. | The balance is enough: the encrypted request memo is built and broadcast. | `RfqSent` |
| `RfqSent` | Request sent to the maker. | A signed refusal memo arrives: `Aborted("MAKER REFUSED (<code>): <reason>; nothing was funded")` with a `refused` event (`code`: `over_limit`, `under_minimum`, `busy`, `unsupported`). A refusal not signed by the maker's key is ignored. Or a quote memo arrives and passes every check (proof of knowledge of B, escrow and maker ids, the maker's ed25519 signature, price ≥ `min_price`, premium ≤ `max_premium`, stipend ≥ `min_stipend`, R1q: W > F + B_R and t2 − open ≤ H, funding and claim windows long enough). The acceptance memo with the premium is broadcast. Rejection or no quote by `quote_timeout_secs`: `Aborted`, nothing funded. | `Accepted` |
| `Accepted` | Premium paid; waiting for the escrow. | `open` is at a final block and every stored parameter equals the quote (R1: maker, token, amount, taker key, A, B, state Open; t1 − t0, t2 − t1 and W exactly as quoted, t0 no later than now + the quoted funding window); the escrow account runs `escrow_code_hash`, has no access keys and pays into `chains.intents`; the stipend is in the taker's NEAR account, and NEAR time + ∆fund < t0 (R1b). The joint address is funded. A mismatch, or R1b closing first: `Aborted` (the premium is lost; no ZEC moved). | `Funded` |
| `Funded` | ZEC at the joint address. | Ready is final and now + B_R < t1 (R2c): `commit_claim`, signed by the taker key, sent from the taker's own NEAR account. Or the punish path: Open/Ready at a final block from t2. If the maker refunds: recovery. If t1 − B_R passes with no Ready: the engine logs `R2c` and waits for the refund or for t2. | `Committed` (or `Recovering`) |
| `Committed` | Escrow locked by the taker's commitment. | The lock is final and now + B_R < locked_at + W (R2r): the engine decides to reveal (stored). | `Revealed` |
| `Revealed` | `a` is public (or about to be). | First tick: R2r again, then `claim(a)` is signed and stored (`claim_raw`, `claim_tx`); nothing is sent. Next tick: R2r again, then the stored bytes are broadcast (`claim_at`). If R2r fails while `claim_at` is None, nothing is sent: the stored claim is dropped and the stage goes back to `Committed` (the maker's R3l refund then gives b). After the broadcast: the claim is final; if it is not on chain after D, a new claim is sent while the window is open. | `Claimed` (or back to `Committed`) |
| `Claimed` | The claim is final; `a`, the seeds and the joint key are erased (R4). | The payout is in `intents.near` under the taker's account. The maker sends `withdraw`; the taker sends it only if its own account holds the prepaid gas at the pessimistic price (about 0.2 NEAR; the stipend does not). If the payout is not there `payout_wait_secs` after the stage began: `PayoutPending`. | `Completed` or `PayoutPending` |
| `PayoutPending` | The claim is final, the payout is not in `intents.near`: the maker has not sent `withdraw`. The tokens wait in the escrow as the taker's credit; they can only go to the taker. The `payout_pending` event and the log give the call that any NEAR account can send: `<escrow>.withdraw({"token": <token>, "owner": <taker_account>})`, 200 Tgas, no deposit; the sender needs about 0.2 NEAR free for NEAR's gas check. Polled every 10 s. | The payout arrives. | `Completed` |
| `Completed` | Terminal. The token is in `intents.near`, owned by `taker_account` (the hex of the NEAR key). | | |
| `Recovering` | The maker refunded; `b` was read from the escrow; a sweep of a joint note with a + b is broadcast. | Every joint note worth more than the sweep fee is swept, the taker's own funding first, one sweep at a time; each needs k_Z confirmations (a sweep is built again if it expires). The stage ends only when no joint note above the fee is left and no funding of this engine can still be mined (it is mined, or its expiry height + k_Z has passed); `a` is kept until then (R4, security review 2, F2-01). Notes at or below the fee are ignored. If no note was ever funded, the engine goes straight to `Recovered`. | `Recovered` |
| `Recovered` | Terminal. The ZEC is back in the temporary wallet (minus fees and the premium). | | |
| `Aborted` | Terminal, with a reason. Nothing more happens. | | |

Typical durations on the local test network: request to `Completed` about 50 s. In a
browser add about 25 s per shielded send (request, acceptance, funding; one more for a
recovery) and real Zcash block times.

### 2.3 Events (`engine::Event`)

Serialized as JSON with a `type` field (snake case). `swap` is the request id (32 hex
characters). Amounts in token base units and yoctoNEAR are strings.

| `type` | Fields | When |
|---|---|---|
| `stage` | `swap`, `stage` | The stage changed (names as in 2.2). |
| `log` | `swap`, `line` | A progress line for humans (the same lines are kept in `TakerSwap.log`). |
| `balance` | `zats` | The temporary wallet's spendable balance changed. |
| `quote` | `swap`, `price`, `premium`, `stipend`, `joint_address` | The maker's quote passed every check. |
| `zcash_tx` | `swap`, `what` (`rfq`, `accept`, `fund`, `sweep`), `txid` | A node accepted a Zcash transaction. |
| `escrow_verified` | `swap`, `escrow_id`, `t0`, `t1`, `t2` | The escrow is open at a final block, matches the quote, and the stipend arrived. |
| `near_tx` | `swap`, `what` (`commit_claim`, `claim`), `hash` | A NEAR transaction was executed. |
| `payout` | `swap`, `account`, `token`, `amount` | The token is in `intents.near` under `account`. |
| `activity` | `swap`, `kind` (`proving`), `label`, `estimated_secs` | Emitted in its own tick, right before a shielded send's proof (request, acceptance, funding, recovery sweep). The next tick blocks for about 25 s in a browser. |
| `warning` | `swap`, `message` | A transient error; the engine retries. |
| `alert` | `swap`, `message` | Maker only: a swap near t1 or t2 that is not settled. |
| `refused` | `swap`, `code`, `min_zats`, `max_zats` | The maker refused the request with a signed memo (codes as in 2.2). The swap ends `Aborted`. |
| `payout_pending` | `swap`, `receiver`, `method` (`withdraw`), `args` (JSON text), `gas` (gas units, string), `deposit` (`"0"`), `credit` | The claim is final and the payout did not arrive within `payout_wait_secs`. Any NEAR account can send this call. |
| `done` | `swap`, `outcome` (`Completed`, `Recovered`, `Aborted`) | Terminal stage reached. |

Example:

```json
{"type":"escrow_verified","swap":"cf431149d0864b9ae3eabd81c916ccd1","escrow_id":"5ea36c8a...","t0":1794540920,"t1":1794540980,"t2":1794541040}
```

### 2.4 The record (`taker::TakerSwap`)

Fields a page may show: `rfq_id`, `stage`, `zats`, `taker_account`, `quote` (price,
premium, stipend, windows), `joint_address`, `escrow_id`, the transaction ids
(`rfq_txid`, `accept_txid`, `fund_txid`, `sweep_txid`, `commit_tx`, `claim_tx`) and `log`.
Fields that are secret: `a`, `s_t`, `e_t` (erased in a terminal stage), `near_sk` (kept),
and the `wallet` record. Do not send them anywhere.

### 2.5 Mapping to the website's interface (`web/ENGINE_API.md`)

The website talks to an `Engine` in a worker (`web/src/engine/api.ts`). That worker is a
layer above this core: `crates/zecret-wasm` (the core's taker and `zecret-intents` behind
wasm-bindgen, over synchronous XHR) and `web/src/engine/real/realEngine.ts` (stages,
units, IndexedDB, the recovery file, the return address and the exit). This table is the
mapping; the rows marked "not in the core" are done by that layer, as described in
`web/ENGINE_API.md`, "How the real engine makes the website's stages".

| Web stage | Core |
|---|---|
| `awaiting_deposit` | `WaitingDeposit` while `balance` < `required_deposit` |
| `deposit_seen` | Not in the core: the wallet sees mined notes only, not the mempool. The layer emits it together with `deposit_confirmed` when the balance reaches the deposit. |
| `deposit_confirmed` | `WaitingDeposit` with enough balance; the `activity` event for the request follows |
| `request_sent` | `RfqSent` |
| `quote_received` | `quote` event (the stage stays `RfqSent` until the acceptance is broadcast) |
| `accepted` | `Accepted` |
| `escrow_open` | `escrow_verified` event |
| `funding_sent` | `Funded` (the `zcash_tx` event with `what: fund`) |
| `funding_confirming` | Not in the core: no confirmation count is emitted. The layer reads the funding's mined height from the wallet (`Session::facts`). |
| `ready` | Not a core stage: the layer reads the escrow state once the funding has k_Z confirmations, and emits `ready` before `committing` otherwise (not on the punish path). |
| `committing` | `Committed` |
| `claiming` | `Revealed` |
| `claimed` | `Claimed`, and `PayoutPending` (the worker stays on `claimed` and shows a `waiting` activity that names the `withdraw` call) |
| `paid_out` | `Completed` (with the `payout` event) |
| `exiting`, `done`, `exit_retry` | Not in the core: the exit (the `zecret-intents` crate) runs after `Completed`, with the NEAR key from `TakerSwap.near_sk`: through 1Click on mainnet, and as a direct `ft_withdraw` sent by the maker's relay on networks without 1Click (`crates/zecret-wasm/src/exit.rs`, `crates/zecret-node/src/relay.rs`). |
| `recovering` | `Recovering` |
| `recovered` | `Recovered` |
| `refunding`, `refunded` | Not in the core as stages: `Taker::return_leftover(ua)` sends the wallet's ZEC to the return address. Use it after `Recovered`, and after `Aborted` (quote rejected, no quote, no escrow). |

Other differences:

- Names: the core's stages are `CamelCase` and its events `snake_case` with a `type` field;
  the web uses `snake_case` stages and its own event shapes (`stage` with `patch`,
  `activity`, `error`). The layer translates.
- Units: the core uses seconds (Unix time) and zatoshis/base units as strings for u128; the
  web uses milliseconds and decimal strings for every amount.
- `createSwap.amountZats` is what the user pays into the wallet. The core's `zats` is what
  the swap sells (the joint note). The layer computes
  `zats = amountZats − required_deposit(0, max_premium)`.
- `createSwap.minUsdc` maps to `start_with_min_price`. It is stored in the swap record.
- `Aborted(reason)`: the web distinguishes `QUOTE_BELOW_MINIMUM`, `MAKER_TIMEOUT` and
  `MAKER_CHECK_FAILED`. The core's reasons are text: a reason that starts with `QUOTE
  REJECTED: price` is below the minimum; `no quote before the timeout` and `the maker never
  opened the escrow` are timeouts; a reason that starts with `MAKER REFUSED` is the maker's
  signed refusal, shown as `MAKER_CHECK_FAILED` with the maker's reason; other `QUOTE
  REJECTED` and escrow-check reasons are safety refusals (`MAKER_CHECK_FAILED`).
- The core emits only the `proving` activity; the layer adds `syncing`, `waiting`,
  `signing` and `broadcasting`.
- `createSwap.shown.premiumZats` becomes the taker's `max_premium` for that swap.

## 3. The NEAR side

- The taker's key is ed25519. Its NEAR account and its Intents account are both the
  lower-case hex of the public key (an implicit account). The maker's stipend (0.03 NEAR)
  creates the account right after `open`.
- The taker signs `commit_claim`'s message with this key and sends `commit_claim` and
  `claim` itself; there is no relayer. A signer must hold the prepaid gas at a pessimistic
  price (about 10^9 yoctoNEAR per gas): the engine attaches 15 Tgas to `commit_claim` and 25
  Tgas to `claim` (it burns 16.8 Tgas). Both cost about 0.002 NEAR in total. A 200 Tgas
  `withdraw` does not fit in the stipend (NEAR wants about 0.2 NEAR held); the maker sends
  it. The taker does not try unless its account holds that much. If the payout does not
  arrive, the engine reports `PayoutPending` with the call (section 2.2).
- Transactions are built, borsh-serialized and signed in `near::tx`, and sent with
  `send_tx` (`wait_until: EXECUTED`). Views use `finality: final` or a final block's hash.
- The direct exit (`ft_withdraw` out of the Verifier) needs 40 Tgas attached; NEAR checks
  that the signer holds it at the pessimistic price (30 Tgas cost 0.031 NEAR in the
  sandbox), so what is left of the stipend (0.0281 NEAR) cannot send it. `zecret maker
  --relay HOST:PORT` serves `POST /execute-intents` for its own takers.

## 4. The maker (`maker::Maker`)

Native only in practice, same interfaces: `Maker::open(cfg: MakerConfig, transport,
store)`, `tick(now, store)`, `directory_entry()`. Its store also holds `near_key`
(`ed25519:...`), `rfq_key` and `seen`. It handles any number of swaps; each tick it scans
for request and acceptance memos, then advances every swap:

`Quoting → Quoted → Opening → Opened → Ready → Sweeping → Swept`, or
`→ RefundSent → Refunded`, or `Aborted`. Rules: Ready only with a sufficient joint note at
k_Z confirmations before t0; refund early only in Open with no sufficient note after the
taker's funding window (R3e); refund after t1 only from a final block past t1 with
now + B_R < t2 (R3d); refund a lock only from a final block past locked_at + W (R3l); never
Ready once b is public; pays the taker's credit into `intents.near` once the claim is
final; before it sends the stipend it checks that the swap at its id is its own (maker,
token, amount, taker key, A, B, reveal window), so a look-alike swap opened by someone
else gets no stipend. Its `add_storage_funds` is always for its own account.
`zecret maker` (crates/zecret-node) runs it in a loop.

Order of work in one tick (security review 2, F2-04 and H2; details in
`docs/security/fixes-2-core.md`):

1. Sync the Zcash wallet. If the sync fails, the tick goes on with the last synced view
   and emits a `warning`. With an old view the maker takes no early refund (R3e) and
   never erases b for lack of a joint note.
2. Advance every swap from `Opening` on (the swaps with deadlines).
3. Read new memos.
4. Expire requests (`quote_send_secs` for a quote not sent, `quote_ttl_secs` for the
   rest), then build quotes.
5. Build or send refusal replies, from the quote budget that no quote used.

A request that the maker does not quote gets a signed refusal memo (envelope type 5,
`msg::Refusal`: code, the maker's `min_zats` and `max_zats`, and an ed25519 signature by
the maker's NEAR key over `refuse-sig-near`, the request id and the fields). It is sealed
to the taker's ephemeral key from the request with the maker's epoch key (label
`refuse`). Codes: `over_limit`, `under_minimum`, `busy` (at `max_pending_quotes`),
`unsupported` (invalid proof of knowledge or NEAR key). A reply costs the maker one Zcash
fee and one proof, so it is limited: at most `max_refusals_per_hour` replies in any hour
(counted from the stored records `refusal/<rfq_id>`, also across restarts), one per
request id, built only when the wallet is in sync, no request waits for its quote and
the tick's quote budget is unused (one proof per tick for quotes and replies together),
dropped if not built within `quote_ttl_secs`. Requests without a reply are refused
silently, as before. The directory entry (`zecret directory`) publishes `limits`
(`min_zats`, `max_zats`, `premium_zats`) under the signature.

Maker settings for requests that nobody has paid for yet (all optional):

| Field | Default | Meaning |
|---|---|---|
| `max_pending_quotes` | 32 | A request is refused while this many swaps are `Quoting` or `Quoted`. |
| `quote_send_secs` | 300 | A `Quoting` swap whose quote no node took within this many seconds after the maker read the request becomes `Aborted("quote not sent in time")`; a built quote that was not sent is dropped. The taker stops waiting after its `quote_timeout_secs` (600 on testnet), counted from its own broadcast of the request. Keep it below that timeout minus the time to mine the quote. Must not be above `quote_ttl_secs`. |
| `quote_ttl_secs` | 3600 | A `Quoting` or `Quoted` swap with no transaction in flight becomes `Aborted("request expired")` after this many seconds. Keep it above 2 × the takers' `quote_timeout_secs` plus the time to mine an acceptance. |
| `max_quotes_per_tick` | 1 | Quotes built at most in one tick. No quote is built (and no RPC is made for one) while the wallet's spendable ZEC is below one quote fee. |
| `max_refusals_per_hour` | 6 | Refusal replies at most in any hour; 0 turns them off. |
| `premium_check` | none; required when `chains.network` is `main` | `{"yocto_near_per_zec": "<yoctoNEAR for 1 ZEC>", "costs_per_request_yocto": "<gas + storage per accepted request, default 0.015 NEAR>"}`. `Maker::open` fails if `premium_zats` is worth less than `stipend` + `costs_per_request_yocto`. |

`misbehave` (test-only) is refused unless the binary is built with the cargo feature
`misbehave`; `harness/run.py` builds it that way, release builds do not.

Withdraws with the escrow of `docs/security/fixes-2.md`: the maker attaches 200 Tgas to
`withdraw` and 150 Tgas to `sweep`. NEAR checks that the signer holds the prepaid gas at a
pessimistic price (about 10^9 yoctoNEAR per gas in the sandbox), so the maker's NEAR
account must keep at least about 0.2 NEAR free besides the stipends it sends. The escrow
restores a credit when the transfer of a withdraw fails. A swap therefore ends
(`Swept` or `Refunded`) only when a final block shows the taker's credit, or the maker's
own credit after a refund, paid. Before it retries a withdraw (at most every D + F
seconds) the maker calls the public `sweep(token)`, which brings tokens left in the
escrow's own Intents account back to the escrow, so the retry can pay.
