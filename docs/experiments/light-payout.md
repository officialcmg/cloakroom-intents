# Light payout: can the taker's key finish the payout alone?

**Question.** After `claim(a)`, the old escrow paid the taker with `withdraw` (200 Tgas).
NEAR accepts that call only from an account that holds about 0.2 NEAR, and the taker's
key holds only the maker's stipend. Can a changed escrow let the taker's key complete
`commit_claim`, `claim` and the payout with what the stipend leaves, with no other party?

**Answer.** Yes. The changed escrow keeps its tokens in its own account inside the
Verifier (`intents.near`) from the moment the maker opens a swap, and pays the taker with
one `mt_transfer` inside the Verifier. The payout call (`withdraw(id)`) needs 19 Tgas;
clients attach 25 Tgas. A new implicit account that holds 0.03 NEAR completes all three
calls and keeps 0.0272 NEAR; with 0.06 NEAR it keeps 0.0572 NEAR. The largest balance
that NEAR checks during the sequence is 0.0278 NEAR (before `withdraw`). The stipend is
raised to 0.06 NEAR: 2.16 times that balance.

Date: 2026-11-14. Sandbox: nearcore 2.14.1, the live `intents.near` code
(`$ZECRET_WASM_DIR/intents.near.wasm`, sha256 `a1afab98…b547`), the live `wrap.near`
code, and the USDC code from mainnet (code hash `DVJx7F3d…NX8q`). Branch
`escrow-light-payout`.

## 1. The rule behind "about 10 times the gas cost"

Source: nearcore tag `2.14.1`, read on 2026-11-14.

- `runtime/runtime/src/config.rs`, `calculate_tx_cost`: the cost of a transaction is

  ```text
  cost = gas_price · burnt
       + max(gas_price, min_gas_purchase_price) · (attached_gas + send_fees + exec_fees + receipt_exec_fee)
       + deposits
  ```

  `burnt` is the gas burnt to turn the transaction into a receipt (about 0.32 Tgas for
  these calls). The second term buys the attached gas and the fees of the receipts that
  the transaction creates, at no less than `min_gas_purchase_price`. Unused gas is
  refunded later. The refund includes the difference between the purchase price and the
  price at which the gas was burnt.
- `runtime/runtime/src/verifier.rs`, `verify_and_charge_tx_ephemeral`: NEAR rejects the
  transaction with `NotEnoughBalance` if the signer's balance is below `cost`. Then it
  checks storage staking (`check_storage_stake`). An account that uses at most 770 bytes is
  a zero-balance account (NEP-448) and needs no stake. An implicit account with one full
  access key uses about 182 bytes, so storage staking does not apply to the taker's key.
- `core/parameters/res/runtime_configs/85.yaml`: `min_gas_purchase_price: { old: 0, new:
  1_000_000_000 }` and `account_creation_charge: { old: 0, new: 0.007 N }`. In
  `parameters.yaml`, `gas_refund_penalty` is 0/100 and `min_gas_refund_penalty` is 0, and
  no diff file from 70 to 88 changes them. `pessimistic_gas_price_inflation` (1.03) is
  still a parameter, but `calculate_tx_cost` does not use it.

The networks, read with RPC on 2026-11-14 at about 02:55 UTC:

| | Mainnet (`free.rpc.fastnear.com`) | Testnet (`rpc.testnet.fastnear.com`) |
|---|---|---|
| `gas_price` | 100 000 000 yoctoNEAR per gas | 100 000 000 |
| Protocol version | 88 | 88 |
| `min_gas_purchase_price` (`EXPERIMENTAL_protocol_config`) | 1 000 000 000 | 1 000 000 000 |
| `account_creation_charge` | 0.007 NEAR | 0.007 NEAR |

So today a signer must hold about 10^9 yoctoNEAR (0.001 NEAR) for each Tgas it attaches:
10 times the gas price, because `min_gas_purchase_price` is 10 times `gas_price`. The
factor falls when the gas price rises above 10^9.

Check against the sandbox (its gas price is also 10^8). The rejection of a transaction
from an account holding 0.001 NEAR reports the cost
(`stipend_alone_completes_commit_claim_and_payout` in `tests/sandbox.rs`):

| Call | Attached | Cost reported by `NotEnoughBalance` | Formula |
|---|---|---|---|
| `commit_claim` | 15 Tgas | 0.015919 NEAR | 15.92 Tgas at 10^9 plus about 0.32 Tgas at 10^8 |
| `claim` | 25 Tgas | 0.025919 NEAR | the same, with 25 |
| `withdraw` | 25 Tgas | 0.025919 NEAR | the same, with 25 |

## 2. Gas of the light payout

Measured in `tests/sandbox.rs`, `gas_needed_for_claim_and_withdraw` (the smallest prepaid
gas of the values tried that works) and in each happy path:

| Call | Smallest prepaid gas that works | Burnt in total | Clients attach |
|---|---|---|---|
| `claim` (publishes `a`; payout Due) | 17 Tgas | 16.56 Tgas | 25 Tgas |
| `claim` that also pays | 34 Tgas | 24.0 Tgas | not used by the taker |
| `withdraw(id)` (one `mt_transfer`) | 19 Tgas | 9.6 Tgas (receipts: escrow 2.64, Verifier 3.93, `on_payout` 2.24, refunds) | 25 Tgas |

Two designs were compared:

- **The payout inside `claim`.** One call. It needs at least 34 Tgas, so the taker must
  hold about 0.035 NEAR before the claim. A 0.06 NEAR stipend gives a margin of 1.7.
- **`claim`, then a light `withdraw`.** Two calls. The largest balance checked is 0.0278
  NEAR. A 0.06 NEAR stipend gives a margin of 2.16.

The escrow supports both: `claim` starts the payout if enough gas is left, otherwise the
payout stays Due. The taker uses the second. The maker's `refund` uses the first (it
attaches 50 Tgas).

## 3. The taker's key alone, from the stipend

`stipend_alone_completes_commit_claim_and_payout`: the maker sends the stipend to the new
implicit account. The account then sends `commit_claim` (15 Tgas), `claim` (25 Tgas) and
`withdraw` (25 Tgas) itself and ends with the tokens under its key in the Verifier.
Balances after the refunds arrived:

| Stipend | After `commit_claim` | After `claim` | After `withdraw` | Spent |
|---|---|---|---|---|
| 0.06 NEAR | 0.059744 (−0.000256) | 0.058088 (−0.001656) | 0.057177 (−0.000912) | 0.002823 |
| 0.03 NEAR | 0.029744 (−0.000256) | 0.028088 (−0.001656) | 0.027176 (−0.000912) | 0.002824 |

The balance NEAR checks before each call: 0.015919 (commit), 0.000256 + 0.025919 =
0.026175 (claim), 0.001912 + 0.025919 = 0.027831 (withdraw). The largest is 0.0278 NEAR.

The maker's transfer that creates the implicit account burnt 0.007115 NEAR on top of the
stipend (`tokens_burnt` of its outcomes). Since protocol 85 an account creation costs
0.007 NEAR (`account_creation_charge`). The old design paid this too.

## 4. Decision rule

"The taker completes everything alone with a stipend of at most 0.06 NEAR, on testnet and
mainnet gas prices, with a margin of 2."

| Reading of the margin | 0.03 NEAR | 0.06 NEAR |
|---|---|---|
| Stipend ≥ 2 × the largest balance checked (0.0278) | no (1.08) | yes (2.16) |
| Still works at 2 × today's gas price | yes: works up to 2.1 × today's price | yes: works up to 13 × today's price |

Gas price tolerance: before `withdraw` the account needs `19.12 Tgas · p` (what commit and
claim burnt) plus `25.92 Tgas · max(p, 10^9)`. With 0.03 NEAR this holds for `p` up to
2.13 · 10^8; with 0.06 NEAR, for `p` up to 1.33 · 10^9.

Both readings pass with 0.06 NEAR. The phase gate passes. The stipend becomes 0.06 NEAR.
It costs the maker 0.03 NEAR more for each opened swap. The taker accepts quotes with a
stipend of at least 0.03 NEAR (`min_stipend`), which works at today's prices.

## 5. The maker's extra cost at open

| | Old escrow | Light payout escrow |
|---|---|---|
| Maker's tokens before open | plain USDC | in the maker's own Verifier account. One deposit (`ft_transfer_call` to the Verifier, 8.1 Tgas burnt) covers many swaps |
| Open | `ft_transfer_call` on the token | `mt_transfer_call` on the Verifier: 14.95 Tgas burnt (Verifier 4.66, escrow `mt_on_transfer` 6.62, Verifier resolve 2.89) with 100 Tgas attached |
| Storage of one swap | 846 bytes on the first swap in a token, 742 after | 522 bytes on the first swap in a token (5.22 · 10^21 yoctoNEAR) |
| Refund | `refund` then a `withdraw` with `ft_transfer` (two calls) | `refund` with 50 Tgas: pays the maker's Verifier account in the same call |
| Stipend | 0.03 NEAR + 0.0071 NEAR account creation | 0.06 NEAR + 0.0071 NEAR account creation |

At 10^8 yoctoNEAR per gas, 15 Tgas of gas costs 0.0015 NEAR. The larger stipend is the
main extra cost.

## 6. Not measured

- Mainnet. Nothing was sent there. The gas price and protocol parameters were read only.
- How often the gas price on mainnet rises above 10^9 yoctoNEAR per gas.
- A Verifier upgrade that makes `mt_transfer` heavier. `withdraw` passes all spare gas to
  `mt_transfer`, so a caller with more NEAR can still pay out; the taker's key alone could
  then need more than 25 Tgas.
- The throwaway measurement test of this phase was replaced by the tests named above;
  its raw log is not in the repository.
