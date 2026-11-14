# Security review 5: the light payout escrow

Date: 2026-11-14. Source: branch `escrow-light-payout` at `afcce98` (contract source
unchanged since `dccd718`). Code hash of the reviewed wasm (rustc 1.97.0, the escrow's
`Cargo.lock`): `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536`, 471 730 bytes.

The reviewer was an independent session. It got only the diff of
`contracts/near-escrow` against `main`, the contract source and tests, the earlier
security material (`nv02-usdc.md`, reviews 1 to 4, fixes 1 to 3, the findings files) and
the Verifier source at `near/intents@16d04f6`. It did not read the design notes or the
author's measurement. Its raw report is [`review-5-raw.md`](review-5-raw.md). Its tests
are in `contracts/near-escrow/tests/review5.rs`.

The task was to find ways to (1) pay twice, (2) lose or lock tokens, (3) let a taker fund
ZEC against an escrow whose Verifier balance does not back the swap, (4) break refund
timing, (5) grief the maker or third parties, each with a failing test.

## Result

No way was found to pay twice, lose or lock tokens, fund against tokens that are not
there, or break the windows. One low finding and one info item:

| ID | Severity | Finding | Fix | Test |
|---|---|---|---|---|
| R5-01 | low | The taker's backing check (escrow Verifier balance ≥ `get_owed`) reads "not backed" in the blocks between a payout's `mt_transfer` and its `on_payout`. Anyone can open this window by sending `withdraw` on a Due payout. | No contract change. The taker engine reads the check again until its funding window nearly closes, so a short false "not backed" costs a re-read. The test now pins down what the client relies on. | `r5_backing_check_fails_closed_only_while_a_payout_is_in_flight` (passes); `taker_funds_only_when_the_escrow_balance_backs_the_swap` in `crates/zecret-core/tests/taker_fixes.rs` (passes) |
| R5-02 | info | The Verifier's operators (roles `DAO`, `UnrestrictedWithdrawer`, the account locker, the pause manager) can move or freeze the escrow's tokens at any time, also after a taker has checked the backing. | Accepted, as before (`fixes-2.md` section 5). The light payout keeps all swap tokens in the Verifier for the whole swap, not only during a payout; this is recorded in the design (`docs/DESIGN.md` section 10). | `r5_verifier_operator_can_drain_the_escrow` (passes; documents the assumption) |

No finding of medium severity or higher, so no second review round was run.

### Why R5-01 is fixed in the client and not in the escrow

The reviewer proposed a second counter `sending[token]` and a view `owed - sending`. That
view would make the check pass during the window, but it would also pass when the escrow
is short by up to the amount in flight (a Verifier operator moved tokens out while a
payout was not yet executed): the relaxed check can say "backed" when the escrow is not.
The strict check never says "backed" when it is not; its only error is a false "not
backed" for one or two blocks per payout, and each payout can be sent only once. The
taker engine reads every poll (2 s) and aborts only when `now + fund_secs + B_R ≥ t0`,
which leaves minutes of reads. So the strict check stays, and the escrow is unchanged.

The window's length was measured in the sandbox: `mt_transfer` in block N, `on_payout` in
block N + 1; the shortfall is exactly the amount in flight; from the `on_payout` block on,
the check passes. Under mainnet congestion the window can be longer; not measured.

## The NV-02 analysis, done again for the new flow

The escrow makes three kinds of external calls. For each: what can fail, when, and the
state that results.

| Call | What can fail | Result | Test |
|---|---|---|---|
| Maker's deposit into its own Verifier account (`ft_transfer_call` on USDC; not an escrow call) | USDC pause or blocklist before the call | USDC refuses; nothing moves; no swap exists | `restriction_before_the_maker_deposit_fails_closed` |
| Same | Pause or blocklist between the Verifier's `ft_on_transfer` and USDC's resolve | USDC reports failure after the Verifier credited the maker. The maker reads its Verifier balance, not the result. No escrow state exists yet. Seen at delays 0 to 2 blocks for each of three restrictions; total USDC of the maker unchanged at every delay | `race_during_the_maker_deposit_at_each_delay` |
| Open: Verifier `mt_transfer_call` → escrow `mt_on_transfer` → Verifier `mt_resolve_transfer` | Escrow panics (bad terms, no storage funds, used id, wrong token, out of gas) | No swap stored; the Verifier's resolve refunds `min(amount, escrow balance)` = the amount, because the escrow's balance is at least `owed + amount` at that point | `bad_open_is_refunded_by_the_verifier`, `open_rejects_*` |
| Same | Verifier `mt_transfer` paused | The first receipt fails; nothing moves | (Verifier behaviour; `verifier_refuses_payout_then_one_retry_pays_once` for the payout) |
| Same | Resolve step fails after a successful `mt_on_transfer` | Refund 0 was going to be applied anyway: the tokens stay with the escrow and back the stored swap | source of `mt_resolve_transfer` at `16d04f6` |
| Same | Caller other than the Verifier | `NotVerifier`; nothing stored | `only_the_verifier_can_open`, `bad_open_is_refunded_by_the_verifier` |
| Payout: escrow `mt_transfer` → `on_payout` | Verifier pauses `mt_transfer` or locks the escrow account | One receipt; nothing moves; `on_payout` sets Due; paid once after they stop | `verifier_refuses_payout_then_one_retry_pays_once`, `payouts_race_with_a_verifier_pause_at_each_delay` |
| Same | `mt_transfer` out of gas | Fails as a whole; Due; `withdraw` resends it with all spare gas | review: gas analysis |
| Same | Two payouts of one swap at once | Only Due → Sending once; the second `withdraw` fails `BadState` | `concurrent_withdraws_pay_once`, `payout_is_paid_at_most_once_over_retries` |
| Same | USDC pause or blocklist (any account) | No effect: no USDC receipt in a payout | `usdc_restrictions_do_not_touch_the_escrow`, `normal_swap_with_usdc` |
| Same | `on_payout` out of gas | Would leave Sending forever. It burns 2.25 of 8 Tgas with the largest ids | `r5_on_payout_gas_margin_with_longest_ids`, `on_payout_fits_in_its_reserved_gas` |
| `claim`/`refund` with the payout gas boundary | A call that passes the gas check and then runs out would revert while `a` or `b` is public in its arguments | No such band from the smallest working gas to 44 Tgas, checked every 0.25 Tgas and every 0.001 Tgas at the threshold (33.13 Tgas) | `r5_claim_gas_sweep_has_no_failing_band`, `r5_refund_gas_sweep_has_no_failing_band`, `r5_refund_at_the_exact_payout_threshold` |
| Maker's `ft_withdraw` out of the Verifier after a refund (not an escrow call) | USDC restriction before or 0 to 4 blocks after | The Verifier gives the maker its balance back; total unchanged | `restriction_during_the_maker_withdrawal_at_each_delay` |

The residual case of `fixes-2.md` section 5 (an Intents deposit failure and a USDC resolve
panic in the same window) no longer touches the escrow: the escrow makes no deposit. It
can still happen to the maker's own deposit; the maker then holds less in the Verifier
than it sent, and opens only what the Verifier shows.

## What was not checked

- The client code (`crates/`, `web/`) was not part of this review. It was tested by the
  author: unit tests, the native harness (23 scenarios) and the browser scenarios.
- Opens through a signed `Transfer` intent with notification: Verifier source only, not
  run.
- Mainnet: nothing was deployed there. Congestion and the length of the R5-01 window on
  mainnet were not measured. Whether the deployed `intents.near` matches the local wasm
  was checked by code hash only (`BtA1BEFN…`, read on 2026-11-13 and 2026-11-14).
- Verifier upgrades after `16d04f6`.
- `curve.rs` was not reviewed again; the diff does not change it.
