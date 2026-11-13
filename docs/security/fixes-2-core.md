# Fixes 2 (core): the maker side of security review 2

Date: 2026-11-13. Review: [`review-2.md`](review-2.md), records in
[`findings-2.json`](findings-2.json). This file covers the maker side of `zecret-core` and
`zecret-node`. The taker-side findings (F2-01, F2-02, F2-03, H1) are not fixed here; see
section 6. (The other file named `fixes-2.md` is about the escrow's Intents withdraw, a
different change.)

Line numbers refer to the working tree on the date above.

## 1. F2-04 (Medium): unpaid requests no longer grow the maker's tick

| Change | Where | Why |
|---|---|---|
| New settings `max_pending_quotes` (default 32), `quote_ttl_secs` (3600), `max_quotes_per_tick` (1), checked by `check_limits` (each ≥ 1). | `crates/zecret-core/src/config.rs:273-285`, `:365` | The review's fix. Defaults are small because each quote is a proof (seconds of CPU) and an attacker chooses how many requests arrive. |
| `MakerSwap.created_at` (wall clock, `#[serde(default)]`). A record without it (older store) gets the time of its next tick. | `maker.rs:129-132`, set in `on_rfq` at `maker.rs:528` | The TTL needs the time the request was taken. |
| `step` is split into: sync; advance swaps from `Opening` on; read memos; expire and quote. | `maker.rs:269-366` (`step`, `advance_all`) | Deadline duties (R3d, R3l, L_M) run before any work that unpaid requests cause. The attacker no longer chooses the order (it chose the rfq_id). |
| `on_rfq` refuses a request while `Quoting` + `Quoted` ≥ `max_pending_quotes`. The count is taken once per tick (`maker.rs:293`) and raised for each new swap (`maker.rs:535`), so the check is O(1). | `maker.rs:430-432` | Bounds the number of unpaid swaps. A refused request is marked handled, like every other rejected request. |
| `Quoting`/`Quoted` with no transaction in flight become `Aborted("request expired")` after `quote_ttl_secs`; `save` erases b, s_m, e_m (R4). | `maker.rs:594-603` | Unpaid work ends. A quote that is in flight is not expired: it is mined, or it dies at its expiry height (`quote expired unmined`). |
| The quote budget for the tick is `max_quotes_per_tick` when the wallet's spendable ZEC (computed once per tick, no RPC) covers `zip317_fee(1, 2)`, else 0. `send_quote` returns at once when the budget is 0, and takes one unit before it does anything that costs work (reply address scan, `build_pay`). | `maker.rs:330-344`, `:664-674` | No RPC round trip and no O(n) scan per stuck request. A failed build also uses the budget, so failures are bounded too. A `warning` is emitted once when requests wait and the wallet cannot pay a quote fee. |
| An Accept for an unknown request is logged and marked handled. | `maker.rs:540-546` | It was retried, with a warning, on every tick. |

Measured with the review's mock (one tick, debug profile with `opt-level = 3`):

| Stuck requests | Before: RPC round trips / CPU | After: RPC round trips / CPU | Still `Quoting` 30 days later (before → after) |
|---|---|---|---|
| 100 | 101 / 108 ms | 1 / 0.6 ms | 100 → 0 |
| 400 | 401 / 473 ms | 1 / 2.8 ms | 400 → 0 |
| 1600 | 1601 / 4.39 s | 1 / 18 ms | 1600 → 0 |

With 3000 stuck requests and one Ready swap past t1: before, one tick took 13.3 s and 3003
RPC round trips; after, the four ticks took 22 ms, 9 ms, 7 ms, 7 ms with 3, 6, 3, 3 round
trips, the same counts as with no stuck requests. The refund was sent in the second tick.

What is left:

- An attacker can keep `max_pending_quotes` requests waiting and so delay quotes for honest
  takers (denial of quotes). Each request costs the attacker a Zcash fee and lasts at most
  `quote_ttl_secs`. The maker's funds and deadlines are not affected. The operator tunes
  the three settings.
- The memo scan still visits every received note once per tick (a set lookup and a copy).
  It costs no RPC. 3000 notes cost a few milliseconds.
- Each opened swap adds about three NEAR round trips per tick. Opened swaps need a paid
  premium, and NV2-02 (section 3) makes the premium cover their cost.
- `quote_ttl_secs` must exceed 2 × the takers' `quote_timeout_secs` plus the time to mine
  an acceptance; otherwise a taker could pay a premium for a quote the maker has dropped.
  This is documented, not enforced, because the maker does not know the takers' setting.

Tests (all through the public `tick` API, scripted RPC, no network):

- `crates/zecret-core/tests/poc2_maker.rs`: the review's PoC (`poc_f2_04_…`), copied from
  `docs/security/poc2/maker_pocs.rs` with the `common` import changed as review-2.md §7
  says; `tests/poc2_common.rs` is `poc2/common.rs` unchanged.
- `crates/zecret-core/tests/maker_bounds.rs`:
  - `refund_deadline_is_met_with_thousands_of_stuck_requests`: 0 and 3000 stuck requests
    plus a Ready swap past t1. Asserts the refund is sent before t2 − B_R, at most 8 RPC
    round trips per tick, the same counts per tick for 0 and 3000, and each tick < 5 s.
  - `quote_builds_per_tick_are_capped`: with ZEC for fees and 50 requests, the Zcash round
    trips in one tick are at most 1 + 2 × `max_quotes_per_tick` (1 and 3 tested).
  - `requests_beyond_the_pending_cap_are_refused`, `stuck_requests_expire`,
    `accept_for_unknown_request_is_handled_once`.

Each of these failed on the code before the change and passes after it (outputs in the
report of this change).

## 2. Hardening notes of review-2.md §6

| Note | Done? | Change / reason |
|---|---|---|
| H1 `reject_quote` fails open | No | It is in `taker.rs`, which another session is editing. Open for the next person (section 6). |
| H2 Zcash sync blocks the maker's NEAR duties | Yes | `maker.rs:280-286`: a sync error is a `warning`, and the tick goes on with the last synced view (`zcash_stale`). With an old view: no early refund R3e (`maker.rs:943-946`, the funding may be there unseen; R3d after t1 needs no Zcash data); no `Aborted` with b erased because no joint note is visible (`maker.rs:999-1002`); pending Zcash transactions are not driven, except that `Sweeping` still pays the taker's credit (`maker.rs:604-609`); no quotes. Allowed: R3d and R3l refunds, re-sent refunds, `open` and the stipend, `withdraw`, and Ready. Ready on an old view is safe: it needs a joint note already seen with k_Z confirmations, and an old tip only gives fewer confirmations. Tests: `zcash_outage_does_not_stop_refunds` (failed before), `zcash_outage_blocks_no_early_refund_decision` (guard for the new path; it also passed before, because the old code did nothing at all). |
| H3 gRPC-web frame length overflow | Yes | `zcash.rs:74-79`: `(i + 5).checked_add(len)`; an overflow or a short body is "truncated gRPC-web frame". Shared with the browser; the change is these lines only. Test `huge_frame_length_is_an_error_not_a_panic`. On a 64-bit host the old code did not overflow either, so this test passes before and after there; the overflow only exists on wasm32, where tests are not run. The wasm32 build of zecret-core with the change succeeds. |
| H4 FileStore durability and permissions | Yes | `crates/zecret-node/src/io.rs:38-125`: write to `<key>.tmp` created with mode 0600 (and set to 0600 if an old one exists), `write_all`, `fsync` the file, rename, `fsync` the directory; new directories are created with 0700 and their parent is synced; `FileStore::new` sets the store directory to 0700 (an error if it cannot). Test `file_store_is_private` (failed before: directory mode 0755). The `fsync` calls are not tested (no power-loss test). Existing files with a wider mode are not changed; the 0700 directory protects them. |
| H5 test-only `misbehave` in production configs | Yes, at the configuration boundary | `config.rs:183-193` `de_misbehave`: without the cargo feature `misbehave`, a configuration (maker or taker JSON) that sets `misbehave` is an error. `Maker::open` also refuses a set value (`maker.rs:186-188`). Feature `misbehave` in `crates/zecret-core/Cargo.toml` and `crates/zecret-node/Cargo.toml`; `harness/run.py:180-182` builds `zecret-node --features misbehave`. The release Dockerfile (`cargo build --release -p zecret-node`) does not enable it. The enum and the field still compile in every build, because `taker.rs` reads them and that file was not to be edited; compiling them out is a follow-up for whoever owns `taker.rs`. Test `misbehave_is_refused_without_the_feature`. |
| H6 rebuilt request reuses a nonce | No | The taker builds the request (`taker.rs`). |
| H7 reserved notes are not stored | No | After a restart a new build can spend the inputs of a pending transaction. The result is a conflicting transaction, not a loss (review §6). The clean fix (re-reserve the nullifiers of stored pending transactions at `open`) needs a parse of the stored raw transaction with its branch id; not small enough for this change. |
| H8 the light wallet trusts its server | No (documentation) | No code change. The statement for ENGINE.md: the light wallet has no reorg handling and does not check that block hashes link; it relies on k_Z and an honest lightwalletd. Left for the documentation writer, as ENGINE.md §1-2 is being edited by another session. |
| H9 native HTTP limits | No | A body size limit is not clearly correct without measurement: a `GetBlockRange` of 1000 blocks can be large, and a too small limit would stop the wallet's sync for good. The 120 s timeout is needed by `send_tx` with `wait_until: EXECUTED`. With F2-04 and H2 fixed, a slow endpoint delays one tick but no longer multiplies by the number of requests. |
| H10 "Funded" from any joint note | No | Taker side; it constrains the F2-01 fix (section 6). |

## 3. NV2-02: the premium must cover the stipend and the maker's costs

`config.rs:286-363`: new optional maker setting

```json
"premium_check": {
  "yocto_near_per_zec": "10000000000000000000000000",
  "costs_per_request_yocto": "15000000000000000000000"
}
```

- `yocto_near_per_zec`: yoctoNEAR for 1 ZEC (10^8 zatoshis), from the operator. Use a low
  estimate of the ZEC price in NEAR.
- `costs_per_request_yocto`: what one accepted request costs the maker besides the
  stipend: `open` gas (ft_transfer_call), refund gas, withdraw gas and the escrow storage of
  one swap. Default 0.015 NEAR. `deploy/maker/README.md` says a swap costs about 0.04 NEAR
  with the 0.03 stipend; measure it on the network in use.

`MakerConfig::check_premium`, called by `Maker::open` (`maker.rs:185`) before anything
else is set up, refuses to start when `premium_zats × yocto_near_per_zec / 10^8 <
stipend + costs_per_request_yocto`, and names the smallest premium that passes. It is
required when `chains.network` is `main` (or `mainnet`); on other networks it runs when it
is set. Test: `premium_below_cost_is_refused` (at 10 NEAR per ZEC, 449 999 zats is refused
and 450 000 accepted; mainnet without `premium_check` is refused).

Not done: the check uses a configured price, not a live one; a margin is the operator's
choice (put it in `costs_per_request_yocto` or use a low price). `testnet-maker.json` and
the example in `deploy/maker/README.md` are unchanged: on testnet the check is optional, and
`deploy/` was not to be edited. With the README's values (premium 20 000 zats) the check
would fail at any realistic price; the operator must raise the premium before mainnet.

## 4. NV2-01: one NEAR RPC endpoint is a trust anchor (no code change)

The precise statement:

- The taker reads every NEAR fact it acts on from one RPC endpoint, with no proof: the
  escrow's swap record at open (R1: maker, token, amount, keys, windows, state), the
  escrow account's code hash, access keys and Intents account, the time of the final
  block (R1b, R2c, R2r), and the Ready and Locked states. It hands its signed
  `commit_claim` and `claim(a)` transactions to the same endpoint.
- An endpoint that lies, alone, can only delay or stall a swap: it does not know b or the
  seeds. The taker's ZEC stays at the joint address, and she keeps `a`.
- An endpoint that lies together with the maker can take the taker's ZEC: it shows a
  matching Open escrow that does not exist, then Ready, then a final lock. The taker funds
  the joint address and publishes `a` to the endpoint. The maker has b and both seeds, so
  it sweeps the joint note. No tokens were ever escrowed.
- The maker relies on its own endpoint in the same way. A lying endpoint can make it miss
  a refund (its tokens) but cannot take its ZEC.
- So the taker's safety assumes: the NEAR RPC endpoint she uses does not collude with the
  maker. Who runs the endpoint the website gives to takers is a deployment fact that is not
  in the repository.

Options, from simplest:

1. State the assumption in DESIGN.md and ENGINE.md, and let the user choose the endpoint
   (`nearRpcUrl` is already an option of the web engine).
2. Read the facts the taker acts on (the escrow at open, Ready, the final lock, the final
   block time, the escrow code hash and keys) from two or more endpoints run by different
   operators, and act only when they agree; send `claim(a)` only after the agreement.
   This removes a single colluding operator but not a collusion of all of them.
3. Verify NEAR state with light-client proofs: follow block producer signatures
   (`next_light_client_block`) from a trusted checkpoint, and check the escrow's state with
   `view_state` proofs against the state root of a verified block. This removes the trust
   in the endpoint for reads; it is the largest change (and a browser cost).

## 5. Test runs for this change

See the report of this change for the commands and outputs (unit tests, integration tests,
clippy, the wasm32 build and the harness).

## 6. Still open for the next person (taker side, `crates/zecret-core/src/taker.rs`)

- **F2-01 (Medium)**: after a refund, `recover` sweeps the first joint note and
  `on_recovering` ends in `Recovered` (a erased) with the funding still unspent. Fix as in
  review-2.md §3 and `findings-2.json`: ignore notes at or below the sweep fee
  (`zip317_fee(1, 1)`), prefer the note from `fund_txid`, and before `Recovered` look again
  for any joint note above the fee (clear `sweep_height`/`sweep_txid`, recover again); keep
  `a` while `fund_txid` is unmined and the tip ≤ `fund_expiry + k_Z`. H10: `fund_txid` can
  name a maker transaction, so do not rely on it alone; the "any note above the fee is left"
  test is what protects the taker. Regression test: `poc2/taker_pocs.rs` `poc_f2_01_…`
  (use `crates/zecret-core/tests/poc2_common.rs`, which is already in place).
- **F2-02 (Medium)**: check R2r again at the claim hand-off in `on_revealed` (with
  `saturating_add`), go back to `Committed` if it fails; make the first claim two-phase
  (sign and store, broadcast next tick); and in `Taker::step` treat a Zcash sync error as a
  warning in `Funded`, `Committed`, `Revealed` and `Claimed` (the maker side of this is H2,
  done here; the pattern in `Maker::step` can be copied). Test `poc_f2_02_…`.
- **F2-03 (Low)**: R1 must require equal windows (`checked_sub` against the quote) and
  every deadline from escrow values must use `saturating_add`. Tests `poc_f2_03_r1_…` and
  `poc_f2_03_huge_…`. The maker writes exactly the quoted windows (`maker.rs`, `open_escrow`).
- **H1**: `reject_quote` returns "no objection" when the configured maker key does not
  parse (`parse_public_key(..).ok()?`); return a rejection.
- Related, also taker side: H6 (request nonce reuse), and compiling the `misbehave` branches
  of `taker.rs` out of non-test builds (H5; the configuration boundary is already closed).

## 7. The client side of the new escrow (`fixes-2.md`)

Requested with this change, after the escrow's payout fix landed:

| Change | Where | Why |
|---|---|---|
| `GAS_WITHDRAW` 100 → 200 Tgas. | `crates/zecret-core/src/near/escrow.rs:30-32` | The new `withdraw` needs at least 110 Tgas (Intents route). With 100 Tgas, `cargo test -p zecret-node --test sandbox` failed on the maker's withdraw for the taker ("Exceeded the prepaid gas"); with 200 it passes. |
| `GAS_SWEEP` = 150 Tgas and `Escrow::sweep()` (`sweep(token)`). | `escrow.rs:33-35`, `:211-216` | The escrow's new public method (at least 90 Tgas). |
| `Maker::sweep`; `pay_taker` calls it before it retries a taker withdraw whose credit is still there D + F after the last try (the credit was restored). | `maker.rs:1028-1053` | So that tokens left in the escrow's own Intents account (a failed `mt_transfer`) are back in the escrow's balance before the retry. `Sweeping` already waited for the credit to be paid. |
| `on_refunded` now stays in its stage until a final block shows the maker's own credit paid. It withdraws, and on each retry (at most every D + F, field `MakerSwap.withdraw_at`) sweeps first. Before, it set `Refunded` right after one withdraw, so a restored credit was never withdrawn again. | `maker.rs:958-985`, `:133-135` | Same failure mode as the taker's withdraw: the escrow's `ft_transfer` can fail while tokens wait in its Intents account, and the credit is restored. |
| Testnet configs name `escrow2.zecret-3f109eea1e.testnet` and code hash `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` (from `deploy/deployment.testnet.json`). | `crates/zecret-node/config/testnet-maker.json`, `testnet-taker.json`, `crates/zecret-node/README.md` | The old escrow is retired. |

Balance and gas price: NEAR checks at submission that the signer holds prepaid gas × a
pessimistic price (about 10^9 yoctoNEAR per gas in the 2.14.1 sandbox, `escrow.rs`
comment). A 200 Tgas `withdraw` therefore needs about 0.2 NEAR held, a 150 Tgas `sweep`
about 0.15 NEAR; unused gas is refunded a few blocks later. The maker's calls are
sequential, so the maker's account must keep at least about 0.2 NEAR free besides the
stipends (0.03 NEAR each) and storage funds. The local network's maker has 100 NEAR plus
its wNEAR. The testnet maker's NEAR balance is not in `deploy/deployment.testnet.json`;
the operator should check it. The taker's stipend never paid for a withdraw (100 Tgas
did not fit either), so nothing changes for the taker. A sweep and a withdraw are sent
again every D + F seconds while transfers keep failing (for example while the token is
paused); that costs gas each time, and the maker does not back off.

Tests: `restored_taker_credit_is_swept_and_retried` and
`restored_maker_credit_is_swept_and_retried` in `tests/maker_bounds.rs` (first try:
withdraw only; no retry before D + F; then sweep and withdraw again; the stage ends only
once the credit reads 0). These were written with the change and not run against the
code before it.

The harness (`harness/run.py`) builds the escrow wasm from `contracts/near-escrow` and
computes the expected code hash from that build (`zecret devnet`), so no hash had to be
changed there.
