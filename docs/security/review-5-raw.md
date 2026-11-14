# Security review 5 (raw): light payout escrow

Date: 2026-11-14. Source: branch `escrow-light-payout` at `afcce98`. The diff against
`main` was reviewed: `contracts/near-escrow/src/lib.rs`, `curve.rs`, `hexbytes.rs` and
the tests. The Verifier code is `near/intents` at
`16d04f6ec4401263f8914638710260da50bb46b0`. The sandbox runs the live `intents.near`
code from `$ZECRET_WASM_DIR` on nearcore 2.14.1.

This review did not read `docs/DESIGN.md`, `docs/experiments/`, `README.md` or `STATUS.md`.
No source of the escrow was changed. The tests are in
`contracts/near-escrow/tests/review5.rs`.

Run:

```sh
./scripts/setup-env.sh && source .tools/env.sh
cd contracts/near-escrow
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test review5 -- --nocapture --test-threads 1
```

## Findings

| ID | Severity | Title | Test | Result on current code |
|---|---|---|---|---|
| R5-01 | low | The taker's backing check reads "not backed" while any payout is in flight | `r5_backing_check_holds_while_a_payout_is_in_flight` | **fails** (demonstrates the finding) |
| R5-02 | info | The Verifier's operators can move the escrow's tokens at any time | `r5_verifier_operator_can_drain_the_escrow` | passes (documents a trust assumption) |

No finding of class (1) pay twice, (2) lose or lock tokens, (3) fund ZEC against tokens
that are not there, or (4) break the windows was demonstrated. The evidence is in
"Checked and not exploitable".

### R5-01 (low): the backing check gives a false "not backed" during a payout

**Scenario.** A payout is two receipts on two accounts. `mt_transfer` runs on the
Verifier in block N and lowers the escrow's Verifier balance by the swap's amount.
`on_payout` runs on the escrow in block N+1 or later and only then lowers `get_owed`.
At every block in between, `balance < owed`, although every swap is fully backed.

A taker reads both values at one final block before it sends ZEC. If that block is
inside the window, the check fails. Anyone can call `withdraw(id)` on any Due payout.
Clients claim with 25 Tgas, which leaves the payout Due (the payout starts in the same
call only at about 33.14 Tgas, see below), so Due payouts are common. A third party can
send `withdraw` so that the window covers the block a taker reads, and makes the taker
stop the swap. The same happens without an attacker at every honest payout. Under
congestion in the escrow's shard, `on_payout` can be delayed, and the window is longer
than one block.

Nobody loses funds: the check fails closed, before the taker sends ZEC. The effect is a
failed swap and wasted fees and time for the taker and the maker (griefing, class 5).

**Test.** `r5_backing_check_holds_while_a_payout_is_in_flight`. It opens swap 1, claims
swap 2 with 25 Tgas (payout Due), and a stranger calls `withdraw` on swap 2. It then
reads the balance and `get_owed` at each block from the `mt_transfer` block to the
`on_payout` block. Output:

```text
mt_transfer ran in block 66, on_payout in block 67
block 66: escrow Verifier balance 3000000000000000000000000, owed 4000000000000000000000000
thread 'r5_backing_check_holds_while_a_payout_is_in_flight' panicked at tests/review5.rs:
the taker's backing check fails while every swap is fully backed: [(66, 3000000000000000000000000, 4000000000000000000000000)]
```

**Possible fixes (not applied).** (a) In the client: when the check fails, wait some
blocks and read again before giving up. Or add the amounts of swaps whose payout is
`sending` back to the balance; the client cannot list them without an index, so (b) is
simpler. (b) In the escrow: keep a second counter `sending[token]`. Add the amount when
the payout goes Due -> Sending. Subtract it in `on_payout` in both branches. Expose a
view that returns `owed - sending`, or both values. Then `balance >= owed - sending`
holds at every block. This view is only safe if `on_payout` always runs; it does (see
the gas margin below).

### R5-02 (info): the Verifier's operators can move the escrow's tokens

**Scenario.** The live Verifier has `mt_force_transfer`, `mt_force_withdraw` and
`ft_force_withdraw` for the roles `DAO` and `UnrestrictedWithdrawer`. They can also lock
the escrow's Verifier account, disable its auth by predecessor, or pause `mt_transfer`.
A role holder can move the escrow's tokens after a taker has checked the backing and
sent ZEC. The payout then fails, `on_payout` sets it back to Due, and it stays Due until
the tokens come back.

**Test.** `r5_verifier_operator_can_drain_the_escrow`. It grants `UnrestrictedWithdrawer`
to the test admin, claims a swap with the payout Due, force-transfers the escrow's
tokens out, and shows that `withdraw` emits `payout_failed`, the taker gets nothing and
the payout stays Due. It passes. This is the trust assumption that `fixes-2.md` already
lists. It is recorded here because the light payout puts all swap tokens in the
Verifier for the whole swap, not only during a payout.

Full run of `tests/review5.rs` (last run): 5 passed, 1 failed
(`r5_backing_check_holds_while_a_payout_is_in_flight`, the R5-01 demonstration).

## Checked and not exploitable

### Gas reservations

- **No gas band where more gas fails.** If `GAS_PAYOUT_START` did not cover the receipt
  fees, a call with just enough gas to pass `has_payout_gas` would run out of gas after
  it. The whole call would revert, while the key share is already public in the
  transaction's arguments. Tests:
  - `r5_refund_gas_sweep_has_no_failing_band`: `refund` from 14 to 42 Tgas (0.25 Tgas
    steps from 30 to 38). Every call from 16 Tgas succeeds. Due up to 33.00, Paid from
    33.25. Passes.
  - `r5_claim_gas_sweep_has_no_failing_band`: `claim` from 16 to 44 Tgas (0.25 Tgas
    steps from 30 to 40). Every call succeeds. Due up to 33.00, Paid from 33.25. Passes.
  - `r5_refund_at_the_exact_payout_threshold`: bisection to the threshold, then every
    0.001 Tgas across it. The payout starts between 33.1348 and 33.1445 Tgas; every
    probe succeeds. Passes.
- **`on_payout` margin.** With a 64-character maker account (the largest record and
  event), `on_payout` burns 2.25 Tgas of its 8 Tgas, and `mt_transfer` burns 4.0 Tgas
  of its 7 Tgas (`r5_on_payout_gas_margin_with_longest_ids`, passes). `on_payout` has a
  fixed cost: one swap of fixed size, one counter, one event of bounded size, and a
  promise result read capped at 64 bytes. It gets exactly 8 Tgas (weight 0), so a caller
  cannot starve it.
- **If `mt_transfer` runs out of gas.** It is one receipt with no calls; it fails as a
  whole, `on_payout` reads Failed and sets Due. `withdraw` can resend it with more gas,
  because `mt_transfer` gets all unused gas (weight 1).
- **If `on_payout` failed** the payout would stay Sending forever. The only cause found is
  running out of gas, which the margin above excludes. `on_payout` does not grow
  storage. The escrow's NEAR balance only loses 1 yoctoNEAR per successful payout and
  the storage-fund withdrawals, which are accounted. So a storage-staking failure of the
  receipt was not found.

### Pay twice (class 1)

- Payout states: only `start_payout` sets Sending, and only from Due. Only `on_payout`
  leaves Sending. `withdraw` refuses anything but Due. Concurrent withdraws are covered
  by `concurrent_withdraws_pay_once`, retries by `verifier_refuses_payout_then_one_retry_pays_once`.
- `on_payout` reads only `Failed` as failure. The Verifier's `mt_transfer` returns
  nothing, so a successful result is empty. A `TooLong` result (more than 64 bytes) would
  count as success. `mt_transfer` cannot return such a value, and a successful receipt
  of `mt_transfer` always moved the amount (see `internal_mt_batch_transfer`).
- The Verifier's `mt_resolve_transfer` after an open: the escrow returns `["0"]` on
  success. That is 5 bytes and passes `promise_result_checked_json_with_len`. The refund
  is 0. On a failed open, the refund is `min(amount, receiver balance)`. The escrow's
  balance at that point is at least `owed + amount`, so the refund only takes the failed
  open's own tokens.
- Opens via the `Transfer` intent with `notification` use the same
  `notify_and_resolve_transfer`, so they behave like `mt_transfer_call`. The deposit
  path (`ft_on_transfer` with a notify action) is commented out at this commit, so a
  token contract cannot call `mt_on_transfer` with a chosen `sender_id`.
- Batch transfers with more than one token, non-`nep141` tokens and
  `previous_owner_ids != sender_id` are refused, and the Verifier refunds them.

### Lose or lock tokens (class 2)

- Recipients cannot block a payout. The Verifier lets locked accounts receive. The
  taker account (64 hex) and the maker account (the authenticated sender) are valid ids
  and never equal to the escrow, so the `sender_id == receiver_id` check does not fire.
- The only lasting blocks are Verifier operator actions (R5-02). After they end, the
  payout can be resent.
- Tokens sent to the escrow outside a swap (plain `mt_transfer`) stay there as surplus.
  Nobody can take them (`escrow_balance_backs_every_unpaid_swap` shows it). This is a
  loss for whoever sends them by mistake, not for a swap.

### Fund against tokens that are not there (class 3)

- The escrow's Verifier balance goes down only through its own `mt_transfer` (with
  predecessor auth; the escrow never calls `add_public_key` and has no access keys),
  refunds of failed opens (see above), and Verifier operator actions (R5-02).
- `owed` goes up in the same receipt that stores the swap, and goes down only in
  `on_payout` after a successful `mt_transfer`. So `balance >= owed` holds at every
  block, except in the Sending window, where it fails closed (R5-01). Within the
  escrow's trust model, a passing check means every unpaid swap, including the taker's,
  is backed.
- `owed` and the balance use the same token key. `TokenId` parsing in the Verifier and
  `AccountId` parsing in the escrow are canonical, so two spellings cannot map to one
  balance.

### Windows and key shares (class 4)

- `claim` needs `now < locked_at + W`. Refund from Locked needs `now >= locked_at + W`.
  Open/Ready refund needs `now < t2`. Commit from Open/Ready needs `now >= t2`. These
  conditions do not overlap. The arithmetic is done in u128. The unit tests
  `exactly_one_exit_over_time_grid`, `refund_window_*` and `punish_path_*` cover the
  boundaries. No change in this diff touches them.
- A call that fails after the checks would publish its share in the arguments without
  changing state. The gas sweeps above show that no prepaid gas from the smallest
  working value up to 44 Tgas makes `claim` or `refund` fail. A claim sent too late still
  publishes `a` in its arguments. That is a client issue, listed in `findings-2.json`.

### Griefing (class 5)

- `withdraw` by anyone can only send a Due payout to its fixed recipient. A stranger
  who sends it with only the minimum gas still gives `mt_transfer` 7 Tgas; it burns about
  4 Tgas. So a stranger cannot make it fail. The one effect is R5-01.
- Swap id squatting with a fake token: `open` charges the maker's storage funds for
  that (maker, token), which only the maker can fund. The Verifier gives the real
  sender as `sender_id`, so nobody can open in another maker's name.
- The escrow's storage grows only through opens (paid by the maker) and
  `add_storage_funds` entries (paid by the caller).

### Cross-shard ordering

- In the sandbox, `mt_transfer` ran in block 66 and `on_payout` in block 67. While the
  payout is Sending, no method can change the swap: `claim`, `refund` and `commit_claim`
  need earlier states, and `withdraw` needs Due.

## Not checked

- The taker and maker client code (`crates/`), the relay and the web client. This
  includes how the client reacts to R5-01.
- Opens through a signed `Transfer` intent with `notification` were checked in the
  Verifier source only, not in the sandbox.
- Behaviour under real mainnet congestion (how long R5-01's window can last).
- Verifier upgrades after `16d04f6`, and whether the deployed `intents.near` matches the
  wasm in `$ZECRET_WASM_DIR`.
- The USDC test file `tests/usdc.rs` was not re-run.
- The curve code (`curve.rs`) was not reviewed again. It is unchanged in the light-payout
  diff, and earlier reviews covered it.
