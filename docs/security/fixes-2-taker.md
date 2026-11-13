# Fixes 2 (taker): the taker side of security review 2, and the testnet findings

Date: 2026-11-13. Review: [`review-2.md`](review-2.md). The maker side is in
[`fixes-2-core.md`](fixes-2-core.md); its section 6 listed the work done here. The testnet
findings are in [`../TESTNET.md`](../TESTNET.md), "Problems met".

Line numbers refer to the working tree on the date above.

## 1. F2-01 (Medium): `a` is kept while a funded joint note can still need it

| Change | Where | Why |
|---|---|---|
| `joint_notes_to_sweep`: the unspent joint notes worth more than the sweep fee (`zip317_fee(1, 1)`), the note of `fund_txid` first, then by value. | `crates/zecret-core/src/taker.rs:1264` | A note at or below the fee cannot pay its own sweep, so it must not hold the swap. The taker's own funding goes first. |
| `funding_may_still_come`: `fund_txid` not mined and the tip at most `fund_expiry + k_Z`. | `taker.rs:1273` | Rule R4: a broadcast funding may still be mined. `fund_expiry` is set only when this engine built the funding, so a `fund_txid` adopted from a note the maker sent (H10) never makes the taker wait for ever. |
| `recover` sweeps the first note of that list. With none left, it keeps `a` while the funding may still come, else ends `Recovered`. With an old Zcash view (sync failed) it does not decide at all. | `taker.rs:1283` | The old code swept `wallet.unspent(..).first()`, whatever it was. |
| `on_recovering`: once a sweep has k_Z confirmations, the stage ends only when no note above the fee is left and no funding can still come. Otherwise it clears `sweep_height` and `sweep_txid` and calls `recover` again (it reads `b` from the escrow again). | `taker.rs:1325` | The old code set `Recovered` (and erased `a`) after the first sweep. H10: the test is "any note above the fee is left", not `fund_txid`. |

Tests: `poc_f2_01_…` (`crates/zecret-core/tests/poc2_taker.rs`, the review's PoC), and in
`crates/zecret-core/tests/taker_fixes.rs`: `f2_01_second_note_is_swept_funding_first`
(a 2 ZEC note from the maker and the taker's 1.0001 ZEC funding: after the first sweep the
taker keeps `a` and chooses its funding next), `f2_01_dust_note_does_not_hold_the_recovery`
(a 10 000-zat note does not hold the swap; `fund_txid` is None, as with H10),
`f2_01_refund_before_the_funding_is_mined_keeps_a`. On the chains: scenario `s23` in
`harness/run.py` (a third wallet pays 200 000 zats to the joint address after the funding;
the maker refunds after t1; the taker sweeps both notes, its funding first).

## 2. F2-02 (Medium): R2r is checked at the hand-off of the claim

| Change | Where | Why |
|---|---|---|
| New record field `claim_raw` (hex of the signed first claim; `#[serde(default)]`, so older records load). | `taker.rs:162` | Two-phase claim. |
| `on_revealed`, state Locked, `claim_at` None: check R2r (`in_time(tnow, locked_at.saturating_add(W))`) first. If it fails: drop `claim_raw` and `claim_tx`, log, stage back to `Committed`, send nothing. Else, without `claim_raw`: sign the claim and store it (`sign_claim`, `taker.rs:1053`), return at once; nothing is sent. With `claim_raw`: broadcast exactly the stored bytes (`broadcast_claim`, `taker.rs:1065`) and set `claim_at`. | `taker.rs:1135-1180` | The review's fix 1 and 2. `claim_at == None` means nothing left the engine. In `Committed` the engine waits; the maker's R3l refund of the lock then gives b and the taker recovers. |
| A re-send after the first broadcast (claim not on chain after D, still before the deadline) signs a new transaction, as before (`resend_claim`). | `taker.rs:1075` | `a` is already public then. |
| `step`: a Zcash sync error is a `warning` in `Funded`, `Committed`, `Revealed`, `Claimed` and `PayoutPending` (`near_only`); the tick goes on with the last view. A pending Zcash transaction is not driven on an old view. In other stages the error stops the tick as before. | `taker.rs:218`, `:392-420`, `:522` | The review's fix 3, the pattern of `Maker::step` (H2). |
| `recover` refuses to decide on an old Zcash view (returns an error, so `a` is kept and the tick retries). | `taker.rs:1284-1286` | A refund seen while the light-wallet server fails must not end in `Recovered` because no note is visible. |

The browser worker: `web/src/engine/real/realEngine.ts` `stepInner` calls `taker.tick`,
then `await this.persistCore()` (it writes the dirty records to IndexedDB and waits), and
only then applies the events and schedules the next call; `restoreStep` does the same after
`reconcile`. So the record with `claim_raw` is durable before the tick that sends it. No
worker change was needed for this.

Tests: `poc_f2_02_…`; in `taker_fixes.rs`: `f2_02_first_claim_is_stored_then_sent` (first
tick: no `send_tx`, `claim_raw` in the store; second tick: the stored bytes are sent),
`f2_02_r2r_is_checked_again_before_the_broadcast` (signed, then NEAR time moves to
lockedAt + W − B_R: nothing sent, back to `Committed`, `a` kept),
`f2_02_zcash_outage_does_not_stall_the_claim`, `f2_02_zcash_outage_never_ends_a_recovery`.

## 3. F2-03 (Low): windows must equal the quote; deadlines saturate

| Change | Where | Why |
|---|---|---|
| R1 checks `v.t1.checked_sub(v.t0) == Some(t1_secs)`, `v.t2.checked_sub(v.t1) == Some(t2_secs)`, `v.reveal_window == reveal_secs`, and `v.t0 <= tnow + t0_secs` (the escrow's t0 is no later than the quoted funding window from now). The NEAR final block is now read before the checks. | `taker.rs:870-890` | The maker writes exactly these values (`maker.rs` `open_escrow`). `>=` accepted W = u64::MAX. |
| Every deadline built from escrow or quote values uses saturating arithmetic: `Timing::budget`, `Timing::in_time`, `Timing::check_windows` (`config.rs:54-90`); the reveal deadline in `on_committed` and `on_revealed` (`taker.rs:1099`, `:1140`); the horizon, R1b and the cover deadline in `on_open`; the quote and escrow timeouts; the withdraw retry time; R1q's bounds in `reject_quote`; the maker's R3l deadline (`maker.rs:1041`). | as listed | A debug build panicked, a release build wrapped. |

Tests: `poc_f2_03_r1_…`, `poc_f2_03_huge_…`; `f2_03_escrow_windows_must_equal_the_quote`
(claim window one second longer, reveal window one second shorter, punish window one
second longer: each aborts with that name; the exact values verify).

## 4. Hardening notes

| Note | Done? | Change / reason |
|---|---|---|
| H1 `reject_quote` fails open | Yes | `taker.rs:813`: a maker key that does not parse is a rejection ("the configured maker key does not parse"). Test `quote_check_fails_closed_on_a_bad_maker_key` (unit test in `taker.rs`). |
| H5 taker side: compile the `misbehave` branches out | Yes | Every taker branch that reads `misbehave` (free-claim, underfund, late-commit, no-reveal, the punish-path tag) is under `#[cfg(feature = "misbehave")]` (`taker.rs:35`, `:913`, `:938`, `:972-1030`, `:1093`). Without the feature the `Misbehave` import and the code are not compiled. The config field and enum stay (the configuration boundary refuses a value without the feature, done in fixes-2-core). `cargo clippy` is clean with and without `--features zecret-node/misbehave`; `harness/run.py` builds with it. |
| H6 rebuilt request reuses a nonce | Yes | `keys::pok_prove` derives its nonce k = H("pok-nonce", x, swap id, role) (`keys.rs:100-112`), as EdDSA does. A request built again (after a node refused the first) has the same plaintext, so the AEAD key never encrypts two different messages. The maker's proofs use the same function. Test `rebuilt_request_has_the_same_plaintext`. |
| H8 the light wallet trusts its server | Documented | `docs/ENGINE.md` §1.3. |
| H10 "Funded" from any joint note | Yes, as a constraint | The F2-01 fix does not depend on `fund_txid` (section 1). |
| H7, H9 | No | Maker and native transport; unchanged from fixes-2-core §2. |

## 5. Clippy

- `clippy::type_complexity` in the tests: `crates/zecret-core/tests/poc2_common.rs` now
  names the handler type `Handler` (a type alias). `find_quote` in `taker.rs` already used
  the alias `FoundQuote` in this tree; clippy reported nothing there.
- `cargo clippy -p zecret-core -p zecret-node -p zecret-wasm --all-targets -- -D warnings`
  is clean, also with `--features zecret-node/misbehave`, and for
  `--target wasm32-unknown-unknown` (core and wasm). No `allow` was added.

## 6. From the public testnet run

### 6a. The refusal reply

| Change | Where |
|---|---|
| Message `Refusal` (code u8, `min_zats`, `max_zats`, ed25519 signature by the maker's NEAR key over `h32("refuse-sig-near", rfq_id ‖ fields)`), envelope type 5 `Refuse { rfq_id, ct }`. Sealed with label `refuse` between the maker's epoch key and the taker's ephemeral key from the request (the same key pair as the request, a different label, so a different key). 81 bytes of payload; it fits the 512-byte memo with room. | `crates/zecret-core/src/msg.rs:28`, `:70-135` |
| Codes: `over_limit`, `under_minimum`, `busy` (at `max_pending_quotes`), `unsupported` (invalid proof of knowledge or NEAR key). An epoch the maker does not hold gets no reply (the maker cannot read the reply address). | `msg.rs:72` |
| Maker: `on_rfq` decrypts first, then decides (busy, then limits, then the request's keys) and calls `queue_refusal`. | `crates/zecret-core/src/maker.rs:487-510` |
| `queue_refusal` stores a `MakerRefusal` (`refusal/<rfq_id>`) if replies are on, none was queued for this id, fewer than `max_refusals_per_hour` were queued in the last hour, and the reply address parses. | `maker.rs:53`, `:603` |
| `advance_refusals`, last in the tick: builds a reply only when the wallet is in sync, no `Quoting` swap waits for its build, and the tick's quote budget (`max_quotes_per_tick`, 0 when the wallet cannot pay a fee) is unused; it uses one unit of it. The built transaction is stored and broadcast on the next tick, like a quote. A reply not built within `quote_ttl_secs` is dropped; one that dies before any node takes it is dropped, not built again. | `maker.rs:644` |
| New setting `max_refusals_per_hour` (default 6, 0 = off). | `crates/zecret-core/src/config.rs:333` |
| Taker: in `RfqSent`, when no quote is found, `find_refusal` looks for a `Refuse` memo for this request that opens and is signed by the maker's key; the swap ends `Aborted("MAKER REFUSED (<code>): <reason>; nothing was funded")` with event `refused`. Anything else is ignored. | `taker.rs:655`, `:762-775` |
| Event `Refused { swap, code, min_zats, max_zats }`. | `crates/zecret-core/src/engine.rs:65` |
| Browser worker: an `Aborted` reason that starts with `MAKER REFUSED` maps to `refund.reason` and `lastError` **`MAKER_CHECK_FAILED`** (the existing screen "The maker’s offer failed a safety check"), with the message "The maker refused the request: <reason>. Nothing was funded and no premium was paid". | `web/src/engine/real/realEngine.ts`, `case 'Aborted'` in `onCoreStage` |

**What the reply costs the maker, and why it cannot drain it.** A reply is one Zcash
transaction (10 000 zats) and one proof. Anyone can send requests, and senders are
anonymous, so a per-sender cap is not possible. The choices:

- A global cap: at most `max_refusals_per_hour` (6) replies in any hour, counted from the
  stored records, so a restart does not reset it. The most an attacker can make the maker
  spend is 6 × 10 000 zats per hour (0.0144 ZEC per day). Each request costs the attacker
  a fee of its own (10 000 zats), so the attacker pays at least as much as the maker.
- One reply per request id.
- No reply takes work from quotes: a reply is built only from the quote budget that no
  quote used in that tick, never while a request waits for its quote build, never when
  the wallet cannot pay a fee, and never on an old Zcash view. A reply's note is set
  aside until it is mined; that can delay the next quote by one block, at most 6 times an
  hour.
- Requests over the cap are refused silently, as before; their taker waits for its quote
  timeout. The cap bounds the cost, the F2-04 limits still bound the work per tick.
- Not chosen: asking the taker to pay for the reply in the request note. It changes the
  request (every request would carry the fee) and still pays nothing for silent refusals.

Tests: `crates/zecret-core/tests/maker_refusal.rs` (`over_limit_request_gets_a_signed_refusal`:
the memo opens with the taker's key and the signature verifies; `refusals_are_capped_per_hour`;
`refusals_can_be_turned_off`; `busy_maker_says_so`; `refusal_is_built_only_from_the_leftover_quote_budget`;
`unsent_refusal_expires`), `msg::tests::refusal_roundtrip_binding_and_size`,
`taker_fixes.rs` `signed_refusal_stops_the_taker` and `refusal_not_signed_by_the_maker_is_ignored`.
On the chains: scenario `s22` in `harness/run.py`.

### 6b. The limits in the signed directory entry

| Change | Where |
|---|---|
| `MakerEntry.limits: Option<MakerLimits { min_zats, max_zats, premium_zats }>` (omitted from JSON when absent) and `MakerEntry::digest()`: with limits, `keys::directory_digest_v2` (domain `maker-directory-near-v2`, the limits included); without, the first digest. | `config.rs:131-172`, `crates/zecret-core/src/keys.rs:150-160` |
| The maker's `directory_entry` always publishes and signs its limits. `check_limits` also refuses `min_zats` 0 or above `max_zats`. | `maker.rs:270-285`, `config.rs:422` |
| `Taker::verify_maker` checks the signature over `digest()`. `start_with_min_price` fails before anything is stored or sent when the amount is outside the limits or the published premium is above `max_premium`. | `taker.rs:290-330` |
| Browser: `web/src/lib/directory.ts` gets an optional `maker.signedLimits` (the core's `limits` object as `zecret directory` prints it); the worker's `coreConfig` passes it to the core as `maker.limits`. `harness/browser/run.mjs` and `make-directory.mjs` copy it from `zecret directory`. | `realEngine.ts` `coreConfig`, `directory.ts` |

A directory entry made by an older maker (no limits) still verifies. Removing or changing
the limits of a new entry breaks its signature (test
`published_limits_are_checked_before_anything_is_sent`, `directory_entry_signs_the_limits`).

### 6c. The false "0 spendable zats" warning

`maker.rs:376-390`: a `Quoting` swap whose quote is built but not yet broadcast no longer
counts as waiting, and the warning needs the wallet's whole balance (notes set aside for
transactions in flight included) to be below one fee. Test
`no_false_low_funds_warning_while_a_quote_is_in_flight` (it reproduces the testnet line
on the old code).

## 7. The taker cannot pay for `withdraw`

The stipend stays 0.03 NEAR; the maker keeps sending `withdraw`.

| Change | Where |
|---|---|
| The taker sends `withdraw` only if its account holds `GAS_WITHDRAW × 10^9` yoctoNEAR (NEAR's pessimistic gas check, 0.2 NEAR); otherwise it logs why once. The old code always tried, and the RPC refused it (testnet item 2). | `taker.rs:224`, `:1206-1228` |
| New stage `PayoutPending` (not terminal). After `payout_wait_secs` (new setting, default 300 s wall clock) in `Claimed` with the credit still in the escrow, the engine logs and emits `payout_pending { receiver, method: "withdraw", args: {"token", "owner"}, gas: "200000000000000", deposit: "0", credit }`: "claimed; payout pending … Any NEAR account can send it: call <escrow> withdraw {…} with 200 Tgas and no deposit (the caller needs about 0.2 NEAR free for the gas check)". It keeps polling (every 10 s) and ends `Completed` when the payout arrives. | `taker.rs:66`, `:1230-1259`, `config.rs:267`, `engine.rs:70` |
| Browser worker: core `PayoutPending` keeps the page on `claimed` and shows a `waiting` activity that names the call. | `realEngine.ts` `case 'PayoutPending'`, `catchUp` |

Test `payout_pending_is_reported_with_the_call`. Docs: `docs/ENGINE.md` §2.2, §2.3, §3;
`web/ENGINE_API.md`.

## 8. Testnet timing

Measured on 2026-11-13 through the light-wallet server `zcash-testnet.chainsafe.dev`
(`GetBlockRange` block times, 3000 intervals ending at block 4,615,373; script in the
session scratchpad, not in the repository): mean 20.4 s, median 13 s, 99th percentile
101 s, 99.9th 211 s, maximum 451 s; three consecutive intervals (k_Z = 3): median 52 s,
99.9th percentile 467 s, maximum 538 s; two: 99.9th percentile 419 s.

| Value | Before | After | Reason |
|---|---|---|---|
| `fund_secs` (∆fund) | 900 | 600 | ≥ 538 s (three blocks, worst seen) + D (30 s). |
| `k_z` | 3 | 3 | A security choice, not a timeout. |
| `windows.t0_secs` | 1800 | 900 | Must exceed F + ∆fund = 610; 290 s left for the open's finality, the stipend and a browser proof. |
| `t1_secs`, `t2_secs`, `reveal_secs` | 600, 900, 600 | unchanged | NEAR only. |
| taker `quote_timeout_secs` (and the website's `quoteTimeoutSecs`) | 1800 | 600 | Two blocks at the 99.9th percentile (419 s) plus proofs and scans. Also needed so that the maker's `quote_ttl_secs` (3600) exceeds 2 × the timeout + one block, which 1800 broke. |

Inequalities (W > F + B_R, t0 − open > F + ∆fund, t1 − t0 > F + B_R, t2 − t1 > D + F + B_R):
600 > 120, 900 > 610, 600 > 120, 900 > 150. Files:
`crates/zecret-node/config/testnet-maker.json`, `testnet-taker.json`,
`web/public/config/testnet.json` (timing and `quoteTimeoutSecs` only; the browser checks
the maker's quote with these values, so they must match). The worker's block-time
estimate for testnet progress is 25 s instead of 75 s (`realEngine.ts` `blockMs`).

Tests: `crates/zecret-node/src/testnet_config.rs` (`testnet_maker_keeps_the_paper_bounds`,
`testnet_taker_accepts_what_the_maker_quotes`, `website_testnet_config_matches_the_taker_template`,
`a_window_that_breaks_a_bound_is_refused`).

## 9. Interface changes

- `TakerSwap.claim_raw` (new, optional). The review's PoC file
  (`crates/zecret-core/tests/poc2_taker.rs`) builds a `TakerSwap` by struct literal, so the
  copy adds `claim_raw: None` (one line with a comment). Otherwise it is the review's file
  with the `common` import changed as review-2.md §7 says.
- `TakerStage::PayoutPending`; events `refused` and `payout_pending`; `TakerConfig.payout_wait_secs`;
  `MakerConfig.max_refusals_per_hour`; `MakerEntry.limits`, `MakerEntry::digest`,
  `MakerLimits`; `keys::directory_digest_v2`; `msg::{Refusal, RefuseCode, Envelope::Refuse}`;
  maker store key `refusal/<rfq_id>`; `Maker::refusals()`.
- `keys::pok_prove` is deterministic.
- `Aborted` reasons that start with `MAKER REFUSED (<code>)`.
- The directory entry printed by `zecret directory` has `limits`, and its signature covers
  them. A taker of this version verifies old entries too; a taker of the previous version
  cannot verify a new entry (it computes the first digest).

## 10. Not done, and what was not verified

- The refusal reply is sent at most 6 times an hour by default; requests beyond that are
  still refused silently, and their takers wait for their timeout.
- `PayoutPending` only reports the call; the engine does not send it (the stipend cannot
  pay for it). The browser shows it as a `waiting` activity on `claimed`, not as an error
  screen; no screen text was changed.
- Old `refusal/*` records stay in the maker's store directory (one file each; at most 6
  an hour by default); they are loaded at start.
- `PayoutPending`, a Zcash outage during the claim and the R2r hand-off failure are tested
  with the scripted transport only, not on the chains.
- The browser scenarios do not include a refusal or a two-note refund.
- The new timeouts were not tried in a swap on testnet.

## 11. Test runs

See the report of this change (section added after the runs).
