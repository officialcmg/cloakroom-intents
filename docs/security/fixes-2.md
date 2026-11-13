# Fixes 2: the escrow's Intents withdraw with the real USDC (NV-02)

Date: 2026-11-13. Finding: [`nv02-usdc.md`](nv02-usdc.md) (review-1 NV-02). Code:
`contracts/near-escrow/src/lib.rs`. Tests: `contracts/near-escrow/src/tests.rs`,
`tests/sandbox.rs`, `tests/usdc.rs`, `test-receiver/`.

## 1. The bug

`withdraw` paid a taker credit with `ft_transfer_call(intents_account, x, msg = taker)`
on the token, and `on_withdraw` restored the credit when that call reported failure. USDC
on NEAR checks pause and blocklist again in `ft_resolve_transfer`. If Circle pauses USDC,
or blocklists the escrow or the Intents contract, after the Intents contract has credited
the taker and before USDC's resolve step, `ft_transfer_call` reports failure although the
tokens were delivered. The escrow restored the credit, and the next `withdraw` paid the
taker again from other swaps' tokens. The analysis reproduced it in the sandbox with the
USDC code from mainnet.

## 2. The change

The client interface is unchanged: `ft_on_transfer` (open), `set_ready`, `commit_claim`,
`claim`, `refund`, `withdraw`, `add_storage_funds`, `withdraw_storage_funds` and every
view keep their names, arguments and results. The `withdrawn` event keeps its fields.

| Part | Before | After |
|---|---|---|
| Intents withdraw, step 1 | `ft_transfer_call(intents, x, msg = taker)` | `ft_transfer_call(intents, x, msg = <escrow account id>)`: deposit into the escrow's own Intents account. The result is not read. |
| Intents withdraw, step 2 | none | `on_deposit` (private, 45 Tgas): `mt_transfer(taker, "nep141:<token>", x)` on the Intents contract with 1 yoctoNEAR, then `on_withdraw` |
| `on_withdraw` | parsed the `ft_transfer_call` result (`used`), restored `x - used` | reads the result of `ft_transfer` or `mt_transfer`: failed → restore `x`, else restore 0 |
| In-flight sum | none | `in_flight[token]` += `x` in `withdraw`, −= `x` in `on_withdraw` (Intents route). Entry created by the first `open` in a token (104 bytes, paid by that maker). View `get_in_flight(token)`. |
| Stranded tokens | none | `sweep(token)`, public: `mt_balance_of(escrow)` → `on_sweep_balance` (private) → `ft_withdraw(token, receiver_id = escrow, balance − in_flight)` → `on_swept` (private, event `swept`) |
| Gas a client attaches to `withdraw` | 100 Tgas | 200 Tgas (110 measured minimum on the Intents route) |
| Wasm | 472 515 bytes, `D9YwW6hJoKEzWkr9XW3fGT7ByCiEQZoWhfb45Q3jA7Lz` | 525 881 bytes, `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` (rustc 1.97.0, the escrow's `Cargo.lock`) |

Changes to the patch in nv02-usdc.md section 6.2:

- `GAS_ON_DEPOSIT` is 45 Tgas, not 30: 15 for `mt_transfer`, 10 for `on_withdraw`, 20 for
  its own work. A unit test checks that `on_deposit` fits in it with the mock's fee
  table (44.2 Tgas used there, including the 25 it reserves). In the sandbox the
  receipt burns 2.49 Tgas.
- The in-flight sum and `sweep` (section 6.2 named `sweep` as needed but did not write it).
  `sweep` leaves the in-flight sum in the Intents account, so a sweep does not take the
  tokens of a withdraw that is between its two steps.
- `nep141_id(token)` builds the Intents token id. `parse_u128_json` is now used only by
  the sweep callbacks.

## 3. Why a credit is paid at most once

1. A taker is paid only by a successful `mt_transfer` started by `on_deposit`. Nothing
   else in the escrow moves tokens to a taker. (The deposit in step 1 credits the escrow
   itself.)
2. `withdraw` sets the credit to 0 in the same receipt that starts step 1. Each started
   withdraw reaches exactly one `on_deposit` (a `then` callback runs whatever step 1
   did) and exactly one `on_withdraw`. Each callback gets a fixed gas amount
   (`with_static_gas`, unused-gas weight 0) that covers the calls it starts. If the
   caller attached too little gas for these reservations, `withdraw` itself fails and
   its state change (the zeroed credit) is rolled back.
3. `on_withdraw` restores `x` if and only if that withdraw's `mt_transfer` failed.
   `mt_transfer` is one receipt in the Intents contract with no further calls, so a
   failed result means nothing moved and a successful result means all of `x` moved.
   So each unit of credit reaches at most one successful `mt_transfer`.
4. Only the escrow can move tokens out of its own Intents account. In the Intents source
   (`github.com/near/intents`, HEAD `fce0936`, read on 2026-11-13): `mt_transfer`,
   `mt_batch_transfer`, `ft_withdraw` and `add_public_key` take the sender from
   `ensure_auth_predecessor_id()`, that is the calling account. The escrow is a named
   account, so it has no implicit Intents key, and it never calls `add_public_key`, so
   no signed intent can name it as signer. Others can only add to that account. The
   exception is the Intents contract's own operators: roles such as `DAO`,
   `UnrestrictedAccountLocker`, `UnrestrictedWithdrawer` can lock the account, add keys
   or force transfers. Takers' funds already sit in the same contract under the same
   operators, so this adds no new party to trust. The source was read at HEAD, not at
   the deployed code hash; the sandbox tests ran the deployed code
   (`BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk`) for every path the escrow uses.

What can still go wrong is the escrow's custody, not the count of payments: tokens can
sit in the escrow's Intents account (fixed by `sweep`), or in the residual case of
section 5, nowhere the escrow can reach.

Custody invariant. Let `C` = the escrow's token balance + the escrow's Intents balance
of that token, and `O` = open swaps + credits + amounts in flight. `withdraw` moves `x`
from a credit to "in flight": `O` unchanged. A deposit moves tokens from the escrow's
token balance to its Intents account, or USDC returns them: `C` unchanged (except the
residual case). A successful `mt_transfer` lowers `C` by `x` and its `on_withdraw` lowers
`O` by `x`. A failed `mt_transfer` changes neither (the credit comes back). `sweep` moves
tokens from the escrow's Intents account to its token balance: `C` unchanged. So
`C ≥ O` holds after every receipt, and every payment is matched by a credit.

## 4. Interleavings

| # | Case | What happens | Result | Test |
|---|---|---|---|---|
| 1 | Step 1 fails before any transfer (USDC paused, escrow or Intents blocklisted before the withdraw) | Tokens stay with the escrow. `mt_transfer` fails for lack of balance (unless a surplus is there, see 4), credit restored | paid 0 now, once after the restriction is lifted | `restriction_before_withdraw_restores_then_pays_once`; race d = 0, 1 |
| 2 | Step 1 reports failure after the Intents contract credited the escrow (the NV-02 case) | The escrow's Intents balance holds `x`; `mt_transfer` succeeds | paid once; nothing restored | `restriction_between_deposit_and_resolve_pays_once` (exact); race d = 2, 3 |
| 3 | Step 1 succeeds partly (the Intents contract returns part as unused) | `mt_transfer(x)` fails unless a surplus covers the rest; credit restored; the kept part stays in the escrow's Intents account | paid 0 now, once later; `sweep` returns the kept part | not reachable with the live code (its `ft_on_transfer` returns 0 or panics); unit test `intents_withdraw_pays_only_through_mt_transfer` covers any step-1 result |
| 4 | `mt_transfer` paid from a surplus that is not this withdraw's deposit | The escrow's own tokens pay a credit the escrow owes; this withdraw's deposit either stays with the escrow (case 1) or becomes surplus | paid once; custody falls by `x` | `concurrent_withdraws_share_the_verifier_balance_safely` |
| 5 | Two takers' withdraws race on the one Intents balance; B's deposit fails, A's lands | B's `mt_transfer` may run after A's deposit and before A's `mt_transfer`. B is paid from A's tokens, A's `mt_transfer` then fails and A's credit is restored. A's leftover deposit is surplus | each paid at most its credit; custody falls by exactly what was paid; A is paid once on retry after a `sweep` | same test, B sent 0 to 4 blocks before A: B paid from A's deposit at k = 0, 1, 2, 3 |
| 6 | The Intents contract refuses `mt_transfer` (operators pause `mt_transfer` or lock the escrow's account) | Deposit stays in the escrow's Intents account; credit restored | paid once after they stop; `sweep` returns the deposit meanwhile (not while the account is locked) | `verifier_refuses_mt_transfer_then_sweep`, `verifier_locks_the_escrow_account` |
| 7 | `sweep` while a withdraw is between its steps | `sweep` withdraws only `balance − in_flight`. If receipts still interleave so that the withdraw's `mt_transfer` finds too little, it fails and the credit is restored | no loss; at most one retry | unit `sweep_returns_only_the_surplus_to_the_escrow` |
| 8 | A withdraw starts while a sweep is between its read and `ft_withdraw` | `ft_withdraw` takes at most the balance read minus the in-flight sum at the callback. If the balance has fallen meanwhile, `ft_withdraw` fails in the Intents contract and nothing moves | no loss | reasoning (Intents `ft_withdraw` checks the balance in one receipt) |
| 9 | Too little gas on `withdraw` | The call fails as a whole; credit unchanged; in-flight unchanged | no change | `withdraw_gas_needed` (100 Tgas fails, 110 works) |
| 10 | Gas exhaustion in a callback | `on_deposit` reserves 45 and burns 2.49; `on_withdraw` reserves 10 and burns 2.07; `mt_transfer` reserves 15 and burns 3.91. If `mt_transfer` ran out of gas, it would fail as one receipt and the credit would be restored | no loss | unit `on_deposit_and_on_sweep_balance_fit_in_their_reserved_gas`; sandbox gas per receipt |
| 11 | Reentrancy | The callbacks are private. Each writes its state before it starts calls and reads nothing external except the result it waits for. `ft_withdraw` back to the escrow uses `ft_transfer` (no `msg`), so the escrow's `ft_on_transfer` (open) is not called | no new path | sandbox: the four callbacks reject outside callers |
| 12 | A fake token | `withdraw` and `sweep` for a fake token touch only `nep141:<fake>` balances in the Intents contract and credits in the fake token | no effect on real tokens | design (token id in every call) |
| 13 | A restriction on the taker | The taker is only named in `mt_transfer` inside the Intents contract; USDC never sees it | no effect | analysis 4.1 item 5 |

Why no lock (one withdraw per token at a time): it would also prevent case 5. But a lock
that is never released (a bug, a callback that fails) would freeze every Intents withdraw
in that token forever. The custody argument shows that sharing the balance is safe; the
cost of case 5 is one retry and one `sweep`. Case 5 needs a deposit that fails while
another lands, which needs the Intents contract to refuse one deposit and accept the next.

## 5. Residual risks

- **Deposit fails and USDC's resolve panics in the same window.** If the Intents
  contract's `ft_on_transfer` fails (for example it is paused) and USDC's
  `ft_resolve_transfer` also panics (Circle pause or blocklist in the same blocks),
  USDC neither returns the tokens nor did the Intents contract credit them. They stay in
  the Intents contract's USDC balance, credited to nobody. The escrow pays nobody twice:
  `mt_transfer` fails, the credit is restored and paid once later from the escrow's other
  tokens. But the escrow is then short by `x`: the last withdraw in USDC fails until the
  Intents contract's operators return the tokens. Not tested (it needs both contracts to
  fail in the same window).
- The Intents contract's operators can pause `mt_transfer` or `ft_withdraw`, lock the
  escrow's Intents account, or force transfers. Withdraws then restore and pay nothing
  until they stop. This trust was already there for the takers' funds.
- Case 5 above: a taker may need a second `withdraw` and someone a `sweep`.
- Circle can upgrade USDC (full-access key on the USDC account, and the admin multisig).
  The tests hold for code hash `DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q` only.
- Mainnet runs on several shards; the window between deposit and resolve can be longer
  than in the one-shard sandbox. The fix does not depend on the window's length.

## 6. Tests

Run on 2026-11-13 in `contracts/near-escrow` with nearcore sandbox 2.14.1, the USDC code
from mainnet (`fetch-usdc-wasm.sh`, block 224315176, code hash checked) and the live
`intents.near` and `wrap.near` code from `$ZECRET_WASM_DIR`.

| Suite | Result |
|---|---|
| `cargo test --lib` | 57 passed (50 before: 1 rewritten, 1 replaced, 7 added) |
| `--test sandbox` (wrap.near, live intents) | 6 passed; `withdraw` now attaches 200 Tgas |
| `--test usdc` (USDC from mainnet) | 8 passed |
| `--test usdc` on the escrow before the fix (`ESCROW_WASM`, built from commit `e194a55`; its code hash is the one deployed on testnet, `D9YwW6…`) | fails as expected: exact pause → "the taker must be paid exactly once (holds 2000000000, credit was 1000000000)", escrow USDC 0; live race → same at d = 2 |
| `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo clippy --target wasm32-unknown-unknown --release -D warnings` | clean (escrow and `test-receiver`) |

Unit tests rewritten: `happy_path_commit_claim_withdraw` (step 1 has `msg` = the escrow,
callback `on_deposit` with 45 Tgas and weight 0; `on_deposit` creates `mt_transfer` with
the exact arguments, 1 yoctoNEAR, 15 Tgas, then `on_withdraw`) and
`on_withdraw_restores_unused_part_of_ft_transfer_call`, replaced by
`intents_withdraw_pays_only_through_mt_transfer` (five step-1 results × two `mt_transfer`
results). Added: `intents_credit_is_paid_at_most_once_over_retries`,
`in_flight_tracks_concurrent_intents_withdraws`, `open_creates_the_in_flight_entry`,
`on_deposit_and_on_sweep_balance_fit_in_their_reserved_gas`,
`sweep_reads_the_escrow_intents_balance`, `sweep_returns_only_the_surplus_to_the_escrow`,
`on_swept_logs_the_amount`. (A unit test of `#[private]` was dropped: the mock calls the
method directly and does not apply the check. The sandbox tests check it.)

USDC sandbox tests (`tests/usdc.rs`): see the escrow README, "What the tests cover". The
race with the live Intents code gave the same window as the analysis: USDC reported
failure after a delivery when the restriction was sent 2 or 3 blocks after the withdraw,
for a pause, a blocklist of the Intents contract and a blocklist of the escrow. The test
fails if no delay hits the window, so it cannot pass without exercising the race.

The test receiver (`test-receiver/`) rebuilds the analysis's tool: it stands in for the
Intents contract, keeps balances for the four methods the escrow uses, and applies the
restriction inside its own `ft_on_transfer`, so it lands before USDC's resolve step every
time. With it the exact cases do not depend on timing.

## 7. Gas

Sandbox, nearcore 2.14.1 (Tgas burnt):

| Call | Before | After |
|---|---|---|
| Taker `withdraw`, wrap.near into live Intents | 42.4 | 64.3 |
| Taker `withdraw`, USDC into live Intents | 13.8 (analysis) | 21.0 |
| Maker `withdraw` | 7.0 | 7.1 |
| `sweep`, nothing to sweep | — | 7.7 |
| `sweep`, stranded deposit returned | — | 20.3 |
| Testnet taker `withdraw` (tUSDC, stand-in Verifier) | 14.0 | 21.65 |

Prepaid gas needed (measured): taker `withdraw` 110 Tgas (100 fails as a whole); `sweep`
90 Tgas (80 fails). Recommended: `withdraw` 200 Tgas, `sweep` 150 Tgas.

Per receipt, taker `withdraw` with USDC: escrow `withdraw` 2.92, USDC
`ft_transfer_call` 2.88, Intents `ft_on_transfer` 3.49, USDC `ft_resolve_transfer` 1.63,
escrow `on_deposit` 2.49, Intents `mt_transfer` 3.91, escrow `on_withdraw` 2.07.

## 8. What clients must change

- **Attach 200 Tgas to `withdraw`** (at least 110). `crates/zecret-core/src/near/escrow.rs`
  has `GAS_WITHDRAW = 100 * TGAS`: with the new escrow, a taker withdraw with 100 Tgas
  fails as a whole (the credit stays; nothing is lost, but nothing is paid). Not changed
  here (`crates/` belongs to another work package). A taker with the 0.03 NEAR stipend
  could not send a 100 Tgas `withdraw` before either; the maker sends it.
- Use the new escrow and code hash on testnet (section 9):
  `escrow2.zecret-3f109eea1e.testnet`, `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`.
  `crates/zecret-node/config/testnet-taker.json` and the website's maker directory still
  name the old escrow and hash. Not changed here.
- No other change: the methods, arguments, views and the `withdrawn` event are the same.
  A client may ignore the new `swept` event and the `get_in_flight` view.
- `docs/DESIGN.md` (the withdraw step and the decisions log) must describe the two-step
  withdraw and `sweep` in the commit that carries this change, as the repository rules ask. Not
  done here: that file is outside this work package.

## 9. Testnet

The old escrow `escrow.zecret-3f109eea1e.testnet` (code `D9YwW6…`) has no keys and
cannot change. It stays as it is, recorded under `retired_escrows` in
`deploy/deployment.testnet.json`. The fixed escrow is a new account under the same owner:

| | |
|---|---|
| Escrow | `escrow2.zecret-3f109eea1e.testnet` |
| Code hash | `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` (525 881 bytes) |
| Created | `A2eEhqHVW7CRso43kmzUGGqfKHYjS3RsXmRyhiZTDD9y` (block 277465816) |
| Deploy + `new` + delete key | `Bwt8ARb8UJA3YPmhHfFAhHaWwAtrmorTdD99mK5Srgmk` (block 277465825) |
| Token registration | `5ssHbZBFgJte8yfNbWmo3JTrEQTYdSbxWeazgNaoextX` |
| Maker storage funds | `8Xaq3aAJm5sKPdsJ8hxyDvUWDq6dnFbGqvN69GcBqGyE` |
| `verify` | all checks passed at final block 277465868 (no access keys, code hash, Intents account, token, Verifier, maker) |

End-to-end run (`zecret-deploy e2e`, run 3 in `deploy/e2e.testnet.json`), all receipts
successful: stipend `DCgWZNgQXvnH5KLfiKQ8jHeiQXkSwP1UHvEwuK72KdMa`, open
`EUHDYSjCXfjWLteoM4FrCBRm1SDvwk1HdLQVi6YEi1Q7`, `set_ready`
`ErgGucmjf7Sh5hkahBMZvaWQ8AyHowzWHJyx47YyNCb2`, `commit_claim`
`GAjCc38PL8w93pRh6nZdMxpDRo87uLTEcpwgPU6UuZfX`, `claim`
`3WEk6ARNuYFxQ9NK38ECWvoe4ghAtrcRPaakLuKqKET4`, `withdraw` (200 Tgas attached, 21.65
burnt) `E1FSqhzLbWbBpitqXaCnFNCnN8VQZYQ7FEaKsDs64vqb`, `execute_intents` (`ft_withdraw`
signed by the taker key) `5wiuNp2LzCheAL2ipNnBD1fvk3wRwk26sNhKZb2hNYEA`. After the
withdraw the taker held 1 tUSDC in the Verifier and the escrow's own Verifier balance
was 0; the signed exit delivered it to the owner account.

Testnet proves the normal path with a standard NEP-141 token. The USDC-specific cases
are proven only in the sandbox (section 6).

## 10. Not verified

- The residual case of section 5 (both failures in one window).
- The clients in `crates/` and `web/` with the new escrow: they were not changed or run
  here. With `GAS_WITHDRAW = 100 Tgas` their taker withdraw will fail until it is raised.
- The Intents source was read at GitHub HEAD `fce0936`, not at the deployed code hash.
- Mainnet: nothing was deployed or run there. The USDC code was only read with
  `view_code`.
- A `sweep` whose `ft_withdraw` meets a paused USDC (the token's `ft_transfer` fails; the
  Intents contract then credits the escrow back in `ft_resolve_withdraw`): read in the
  source only, not run.
