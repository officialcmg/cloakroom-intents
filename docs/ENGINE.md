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
built (and stored with the record) in one tick and broadcast in the next. `claim(a)` and
`refund(b)` are decided in one tick and sent in the next.

**Recovery file.** The records are the recovery file: `wallet` plus `swap/<rfq_id>`. A host
that restores them into a fresh store and opens the engine continues the swap.
`TakerSwap.near_sk` is the seed of the NEAR key that owns the payout inside
`intents.near`; it stays in the record after the swap ends.

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

## 2. The taker API (`taker::Taker`)

```rust
let mut taker = Taker::open(cfg: TakerConfig, transport: SharedTransport, store: &mut dyn Store)?;
taker.ua()                       // the temporary wallet's unified address (show as QR code)
taker.balance()                  // spendable zatoshis as of the last tick
taker::required_deposit(zats, cfg.max_premium)  // what the user must pay into the wallet
taker.start(zats, now, store)?   // checks the maker's directory entry; creates the swap
taker.take_events()              // events from `start`
taker.tick(now, store)           // run the state machine
taker.swap()                     // Option<&TakerSwap>: the current record
taker.return_leftover(ua)?       // after the swap: send what is left to the user's address
wallet::prepare_proving_key()    // build the Orchard proving key now (about 20 s in a browser)
```

- `Taker::open` resumes the swap in progress if the store holds one that is not terminal.
- `start` fails (and stores nothing) if a swap is in progress, if the maker's directory
  entry is not signed by its NEAR key, or if that key is not a full-access key of the
  maker's NEAR account at a final block. Nothing is sent and nothing is paid then.
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

The `maker` entry is what the maker daemon prints with `zecret directory`. `misbehave` is a
test-only field; leave it out.

### 2.2 Stages

`TakerSwap.stage` (serialized as a string, or `{"Aborted": "<reason>"}`). The `Stage` event
carries the name.

| Stage | Meaning | Leaves when | Next stage |
|---|---|---|---|
| `WaitingDeposit` | The swap exists; the wallet waits for `required_deposit` zatoshis. | The balance is enough: the encrypted request memo is built and broadcast. | `RfqSent` |
| `RfqSent` | Request sent to the maker. | A quote memo arrives and passes every check (proof of knowledge of B, escrow and maker ids, the maker's ed25519 signature, price ≥ `min_price`, premium ≤ `max_premium`, stipend ≥ `min_stipend`, R1q: W > F + B_R and t2 − open ≤ H, funding and claim windows long enough). The acceptance memo with the premium is broadcast. Rejection or no quote by `quote_timeout_secs`: `Aborted`, nothing funded. | `Accepted` |
| `Accepted` | Premium paid; waiting for the escrow. | `open` is at a final block and every stored parameter equals the quote (R1: maker, token, amount, taker key, A, B, windows, state Open); the escrow account runs `escrow_code_hash`, has no access keys and pays into `chains.intents`; the stipend is in the taker's NEAR account, and NEAR time + ∆fund < t0 (R1b). The joint address is funded. A mismatch, or R1b closing first: `Aborted` (the premium is lost; no ZEC moved). | `Funded` |
| `Funded` | ZEC at the joint address. | Ready is final and now + B_R < t1 (R2c): `commit_claim`, signed by the taker key, sent from the taker's own NEAR account. Or the punish path: Open/Ready at a final block from t2. If the maker refunds: recovery. If t1 − B_R passes with no Ready: the engine logs `R2c` and waits for the refund or for t2. | `Committed` (or `Recovering`) |
| `Committed` | Escrow locked by the taker's commitment. | The lock is final and now + B_R < locked_at + W (R2r): the engine decides to reveal (stored), then sends `claim(a)` on the next tick. | `Revealed` |
| `Revealed` | `a` is public (or about to be). | The claim is final. If the claim is not on chain after D, it is sent again while the window is open. | `Claimed` |
| `Claimed` | The claim is final; `a`, the seeds and the joint key are erased (R4). | The payout is in `intents.near` under the taker's account (`withdraw` by the taker if its stipend pays the gas, otherwise by the maker; anyone may call it). | `Completed` |
| `Completed` | Terminal. The token is in `intents.near`, owned by `taker_account` (the hex of the NEAR key). | | |
| `Recovering` | The maker refunded; `b` was read from the escrow; the sweep of the joint note with a + b is broadcast. | The sweep has k_Z confirmations (it is built again if it expires). If no note was ever funded, or the funding expired unmined, the engine goes straight to `Recovered`. While a broadcast funding may still be mined, it keeps `a` (R4). | `Recovered` |
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
| `warning` | `swap`, `message` | A transient error; the engine retries. |
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

## 3. The NEAR side

- The taker's key is ed25519. Its NEAR account and its Intents account are both the
  lower-case hex of the public key (an implicit account). The maker's stipend (0.03 NEAR)
  creates the account right after `open`.
- The taker signs `commit_claim`'s message with this key and sends `commit_claim` and
  `claim` itself; there is no relayer. A signer must hold the prepaid gas at a pessimistic
  price (about 10^9 yoctoNEAR per gas): the engine attaches 15 Tgas to `commit_claim` and 25
  Tgas to `claim` (it burns 16.8 Tgas). Both cost about 0.002 NEAR in total. A 100 Tgas
  `withdraw` does not fit in the stipend; the maker sends it.
- Transactions are built, borsh-serialized and signed in `near::tx`, and sent with
  `send_tx` (`wait_until: EXECUTED`). Views use `finality: final` or a final block's hash.

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
