# near-escrow

The NEAR escrow for one swap between shielded ZEC and a NEP-141 token (USDC in the
product). It is a port of `contracts/src/CloakroomEscrow.sol` (v2) and
`contracts/src/Pallas.sol` to Rust, near-sdk 5.30.

The state machine, the time windows and the checks are the same as in the Solidity
escrow. The differences are listed in [Differences from the Solidity escrow](#differences-from-the-solidity-escrow).

## How a swap works

The taker holds key share `a`. The maker holds key share `b`. `A = a·G` and `B = b·G` on
Pallas. G is the Orchard spend-authorization base point (RedPallas SpendAuth). The escrow
holds the maker's tokens and has two exits. Each exit publishes one share:

- `claim(a)` stores `a` and pays the taker. The maker reads `a` and takes the ZEC with `a + b`.
- `refund(b)` stores `b` and pays the maker back. The taker reads `b` and takes the ZEC back.

The tokens stay inside the NEAR Intents Verifier (`intents_account`) for the whole swap,
in the escrow's own Verifier account. The maker opens a swap with `mt_transfer_call` on
the Verifier, from its own Verifier account. The escrow stores the swap only when the
Verifier calls it after it moved the tokens. A payout is one `mt_transfer` inside the
Verifier to the taker (claim) or to the maker (refund). No call of the escrow touches the
token contract.

States and windows (`t0 < t1 < t2`, `W` = `reveal_window`, times in seconds):

```text
(none)     --mt_on_transfer from the Verifier (maker's mt_transfer_call)-->  Open
Open       --set_ready (maker, before t0)-->                   Ready
Ready      --commit_claim (taker-signed, before t1)-->         Locked
Open/Ready --commit_claim (taker-signed, from t2)-->           Locked   (punish path)
Locked     --claim(a) (before locked_at + W)-->                Claimed
Open       --refund(b) before t0-->                            Refunded
Open/Ready --refund(b) in [t1, t2)-->                          Refunded
Locked     --refund(b) from locked_at + W-->                   Refunded
```

Each swap also has a payout state. It changes only in Claimed and Refunded:

```text
None     --claim / refund-->                                     Due
Due      --claim / refund with enough gas left, or withdraw(id)-->  Sending  (mt_transfer started)
Sending  --on_payout: mt_transfer succeeded-->                   Paid     (final)
Sending  --on_payout: mt_transfer failed-->                      Due
```

`claim` and `refund` store the share and the new state first. They start the payout in
the same call only if enough gas is left ([`GAS_PAYOUT_START`](src/lib.rs), 18 Tgas);
otherwise the payout stays Due and anyone sends `withdraw(id)`. A failed payout never
undoes a published share.

## Interface

All byte strings in arguments and views are hex, 64 characters for 32 bytes, with no
`0x`. Times are seconds since the Unix epoch (`env::block_timestamp() / 10^9`).

### Init

`new(intents_account: AccountId)`, private. `intents_account` is the NEAR Intents
Verifier (`intents.near` on mainnet). Only the escrow account itself can call `new`.
Another account that calls it gets `Method new is private`, so nobody else can initialise
a deployed escrow with their own `intents_account`.

The contract has no owner and no upgrade method. That is a property of the code. The
escrow is immutable only if its account also has no access keys. See
[Deployment](#deployment).

### Deployment

The escrow must end with the reviewed code, its state from `new`, and no access keys.
Whoever holds a key of the escrow account can deploy other code, for example after
`claim` has published `a`, and keep both the ZEC and the tokens.

1. Create the escrow account with one temporary full-access key. Use a named account
   (for example `escrow.<owner>.near`), not an implicit hex account: the commit message
   tells networks apart only by the escrow account id, and an implicit account has the
   same id on testnet and mainnet.
2. Sign one transaction with that key, from the escrow account to itself, with these
   actions in this order:
   1. `DeployContract` with the reviewed wasm;
   2. `FunctionCall` `new({"intents_account": "intents.near"})`;
   3. `DeleteKey` of the key that signs the transaction.

   The actions run together or not at all. `new` must be in the same transaction as
   `DeleteKey`, and it is private, so it must be called by the escrow account itself. A
   transaction from the parent account cannot call `new`.
3. Check the result with the checks below. Publish the code hash.

The escrow needs no storage registration on the token: it never holds the plain token.
Its Verifier account is created by the first transfer to it.

`deploy_escrow` in `tests/common/mod.rs` deploys the escrow this way, and
`new_is_private_and_deployment_leaves_no_keys` checks it.

Before the taker funds the shared Zcash address, she checks the escrow at one final block:

- `view_access_key_list` of the escrow account is empty;
- the code hash (`view_account`, `code_hash`) equals the hash of the reviewed wasm
  (sha256 of the wasm file, base58);
- `get_intents_account()` returns the Verifier she expects (`intents.near` on mainnet);
- the swap (`get_swap(id)`) has the quoted terms and is Open;
- the backing check: `mt_balance_of(<escrow>, "nep141:<token>")` on the Verifier is at
  least `get_owed(<token>)` on the escrow, both read at the same final block.

If one check fails, she does not fund. Why the backing check is enough: every swap's
amount is in `get_owed` from the receipt that stores the swap until the receipt in which
its payout is known to have succeeded, and only the escrow can move tokens out of its
Verifier account. The escrow stores a swap only in `mt_on_transfer` called by the
Verifier, after the Verifier moved exactly that amount to the escrow, and it never
counts the same tokens for two swaps. So a balance at least `get_owed` means that every
unpaid swap, hers included, is covered. Tokens can leave the escrow's Verifier account
without the escrow only by the Verifier operators' force methods; the check then fails.
A payout of another swap can lower the balance one or two blocks before `get_owed`
falls, so a failed check is read again a few blocks later before the taker gives up.

### Storage funds

Each new swap uses contract storage: 522 bytes (0.00522 NEAR) for the first swap in a
token (the swap and the token's owed entry), less after. The maker pays for it in
advance, per token:

- `add_storage_funds(token, account_id?)`, payable. Adds the attached NEAR to the
  caller's funds for swaps in `token`. Returns the new balance. `account_id` is optional;
  if it is given and is not the caller, the call fails with `BadParams` (review-1 F-01).
- `withdraw_storage_funds(token, amount?)`, exactly 1 yoctoNEAR attached. Sends unused
  funds back to the caller.
- `get_storage_funds(account_id, token)`, view.

### Open

The maker first holds the token in its own Verifier account (a deposit:
`ft_transfer_call(receiver_id = <Verifier>, amount, msg = "")` on the token). Then it
calls `mt_transfer_call` on the Verifier, with 1 yoctoNEAR:

```json
{
  "receiver_id": "<escrow>",
  "token_id": "nep141:<token>",
  "amount": "3000000",
  "msg": "{\"salt\":\"<32 bytes>\",\"taker_pk\":\"<32 bytes>\",\"a\":\"<A, 32 bytes>\",\"b\":\"<B, 32 bytes>\",\"t0\":1763000000,\"t1\":1763001000,\"t2\":1763002000,\"reveal_window\":600}"
}
```

The Verifier moves the amount from the maker to the escrow, then calls the escrow's
`mt_on_transfer(sender_id, previous_owner_ids, token_ids, amounts, msg)`, then its own
`mt_resolve_transfer`. The escrow accepts the call only from `intents_account`
(`NotVerifier` otherwise), so `sender_id` is an account that the Verifier authenticated:
it becomes the maker. It stores the swap, adds the amount to `get_owed(token)` and
returns `["0"]` (all tokens kept). On bad input it panics; the Verifier's
`mt_resolve_transfer` then returns the whole amount to the maker and no swap exists. It
rejects:

- not exactly one token, or `previous_owner_ids` other than `[sender_id]`; a token id
  that is not `nep141:<account>` (`BadParams`);
- `amount` = 0; not `now < t0 < t1 < t2`; `reveal_window` = 0; `taker_pk` all zero;
- `a` or `b` that is not a canonical, non-identity Pallas point;
- unknown fields in `msg` (for example `relayer_fee`);
- a swap id that is already used (`BadState`);
- maker storage funds for this token below the storage cost (`InsufficientStorage`).

The token contract is not part of the open. A USDC pause or blocklist can only make the
maker's own deposit into the Verifier fail, before any swap exists. With USDC the
deposit's result can report a failure after the Verifier credited the maker
(docs/security/nv02-usdc.md), so a maker reads its Verifier balance instead of trusting
the result (`race_during_the_maker_deposit_at_each_delay` in `tests/usdc.rs`).

### Transitions

| Method | Caller | Allowed | Effect |
|---|---|---|---|
| `set_ready(id)` | maker only | Open, before `t0` | Ready |
| `commit_claim(id, commitment, signature)` | anyone | Ready before `t1`, or Open/Ready from `t2` | Locked; stores `commitment` and `locked_at` |
| `claim(id, a)` | anyone | Locked, before `locked_at + W`; `commitment_of(id, a)` matches; `a` canonical, not 0, `a·G = A` | Claimed; stores `a`; payout Due, and starts it if the gas is there |
| `refund(id, b)` | anyone | Open before `t0`; Open/Ready in `[t1, t2)`; Locked from `locked_at + W`; `b` canonical, not 0, `b·G = B` | Refunded; stores `b`; payout Due, and starts it if the gas is there |
| `withdraw(id)` | anyone | the payout is Due; at least 18 Tgas left | starts the payout |

`signature` (64 bytes) is the ed25519 signature by `taker_pk` over the bytes that the view
`commit_message(id, commitment)` returns.

### Payout

A payout moves the swap's amount from the escrow's Verifier account to the recipient's
Verifier account: the taker account (`taker_account`, the hex of `taker_pk`) after a
claim, the maker after a refund. The recipient is fixed by the swap; no caller chooses
it. It is:

1. Due → Sending, then `mt_transfer(receiver_id = recipient, token_id =
   "nep141:<token>", amount)` on the Verifier with 1 yoctoNEAR, 7 Tgas reserved and all
   gas left over;
2. the callback `on_payout(id)` (private, 8 Tgas reserved): if `mt_transfer` succeeded,
   Paid and `get_owed` falls by the amount (event `paid`); if it failed, Due again
   (event `payout_failed`).

`mt_transfer` is one receipt in the Verifier with no further calls: it moves the whole
amount or nothing, and only from the escrow's own Verifier account. So a payout is paid
at most once: only Due can become Sending, only `on_payout` leaves Sending, and only a
successful `mt_transfer` makes it Paid. A second `withdraw` while one is in flight fails
with `BadState`. The recipient's Verifier account needs no NEAR and does not need to
exist before.

Who sends it: the taker's key sends `claim` (25 Tgas) and then `withdraw(id)` (25 Tgas)
from the stipend. The maker's `refund` attaches 50 Tgas and pays in the same call. Any
account can send `withdraw(id)`.

What can make a payout fail: the Verifier's operators pause `mt_transfer` or lock the
escrow's Verifier account. Nothing moves; the payout is Due again and is paid once after
they stop (`verifier_refuses_payout_then_one_retry_pays_once`). The token contract is not
involved: USDC pauses and blocklists change nothing here
(`usdc_restrictions_do_not_touch_the_escrow`).

### Stray tokens

Tokens that reach the escrow's Verifier account outside a swap (a plain `mt_transfer` or
deposit to the escrow) are a surplus above `get_owed`. Nobody can take them: the escrow
has no owner and no method that moves them. Plain tokens sent to the escrow account on
the token contract are also stuck. There is no `sweep`: the old one moved tokens from the
escrow's Verifier account back to its token balance, and the escrow no longer uses a
token balance.

### Views

- `get_swap(id)`: the swap, or `null`. Fields: `id, maker, token, amount, taker_pk,
  taker_account, a_pub, b_pub, t0, t1, t2, reveal_window, state, locked_at, commitment,
  a, b, payout, recipient`. `locked_at` and `commitment` are `null` until `commit_claim`;
  `a` is set only in Claimed; `b` only in Refunded. `payout` is `none`, `due`, `sending`
  or `paid`. `recipient` is the taker account in Claimed, the maker in Refunded, else
  `null`. Clients read the published share here.
- `get_owed(token)`: the sum of the amounts of the swaps in `token` whose payout is not
  Paid (`"0"` when none).
- `swap_id(maker, salt)`, `commit_message(id, commitment)`, `commitment_of(id, a)`.
- `get_intents_account()`, `get_storage_funds(account_id, token)`.

### Events

NEP-297 logs: `EVENT_JSON:{"standard":"near_escrow","version":"2.0.0","event":...,"data":[...]}`
with events `opened`, `ready`, `locked`, `claimed` (has `a`), `refunded` (has `b`),
`paid` and `payout_failed` (each with `id, token, recipient, amount`).

### Errors

A panic message starts with a code, then `: ` and a detail. The codes are the Solidity
error names: `BadParams`, `BadState`, `TooEarly`, `TooLate`, `BadSecret`,
`BadSignature`, `NotMaker`, `NotClaimable`, plus `InsufficientStorage`, `NotVerifier`
(an `mt_on_transfer` not from the Verifier) and `NotEnoughGas` (`withdraw` with less
than 18 Tgas left).

## Encodings

| Value | Encoding |
|---|---|
| Scalar (`a`, `b`) | 32 bytes, little-endian, canonical: less than q (Pallas group order) and not 0 |
| Point (`A`, `B`) | 32-byte compressed Pallas encoding: x as 32 bytes little-endian; bit 255 (top bit of byte 31) = parity of y. Only canonical encodings decode. The identity (32 zero bytes) is rejected. This is `pasta_curves` `GroupEncoding::to_bytes`, the encoding the client uses. |
| Swap id | `sha256("near-escrow/v1/swap-id" ‖ len(maker) as 1 byte ‖ maker ‖ salt)` |
| Commitment | `sha256("near-escrow/v1/commitment" ‖ id ‖ a)` |
| Commit message (signed) | `"near-escrow/v1/commit-claim" ‖ len(escrow) as 1 byte ‖ escrow account id ‖ id ‖ commitment` |
| Taker account | lowercase hex of `taker_pk` (a NEAR implicit account; the same id inside Intents) |

The commit message starts with a fixed ASCII domain. It cannot be read as an Intents
`raw_ed25519` payload (JSON, starts with `{`) or as a NEAR transaction (signed as a
32-byte hash), so a signature for one cannot be used for another.

### Mapping to the Solidity values

The Solidity escrow takes `uint256` scalars and affine `(x, y)` points as `uint256`
pairs. For the same key:

- scalar: the 32-byte value here is the `uint256` written little-endian (reverse the bytes
  of the big-endian 32-byte form).
- point: take `x` little-endian and set the top bit of the last byte to `y mod 2`.
  `x < p < 2^255`, so that bit is always free.

G is `GX = 0x375523b3...b875c963`, `GY = 0x1ad0357f...7b0c3bc9` in `Pallas.sol`. Its
compressed form is `63c975b884721a8d0ca1707be30c7f0c5f445f3e7c188d3b06d6f128b32355b7`.
The unit tests check that this equals the verification key of scalar 1 for
`reddsa::orchard::SpendAuth` (how `node/src/keys.rs` derives G), that `(GX, GY)` is on the
curve, and that the `A_SEC/AX/AY` and `B_SEC/BX/BY` vectors from
`contracts/test/CloakroomEscrow.t.sol` match after this mapping.

## Differences from the Solidity escrow

| Solidity | Here | Why |
|---|---|---|
| `open(...)` with `transferFrom` and a balance check (audit L-01) | the Verifier's `mt_transfer_call` → `mt_on_transfer` | The tokens stay in the Verifier, where the payout is one atomic `mt_transfer`. The escrow records the amount the Verifier moved, and only the Verifier can call it. |
| `payout` address, secp256k1 `ecrecover`, EIP-191 digest with `chainid` | `taker_pk` (ed25519), `env::ed25519_verify`, domain-separated message with the escrow account id | One taker key on NEAR. The escrow account id binds the network in practice; there is no chain id on NEAR. |
| `relayer`, `relayerFee` | removed | The maker sends a NEAR stipend to the taker key; no relayer (design decision of 2026-11-13). |
| keccak256 over `abi.encode` | sha256 with domain strings | Native NEAR host function. |
| `uint256` scalars and `(x, y)` points | 32-byte little-endian scalars and compressed points | The client's native encodings. Mapping above. |
| Shares only in events | Shares also stored in the swap (`a`, `b`) | Clients read them with a view call at a final block. |
| The exit transfers in the same transaction | The exit starts the payout; a callback records its result; `withdraw(id)` retries | NEAR cross-contract calls are asynchronous. The share is published whatever the payout does. |
| No storage payment | Maker prepays storage per token | `mt_on_transfer` has no attached NEAR. Without it, anyone could fill the escrow's storage. Only the owner can add funds for itself (review-1 F-01). |
| Unknown id: `setReady` → `NotMaker`, `refund` → `TooEarly` | Unknown id → `BadState` everywhere | Clearer. Behaviour on known swaps is unchanged, including `refund` on Claimed/Refunded → `TooEarly`. |
| `payout != 0` | `taker_pk` not all zero | Same intent. |

All later transitions write fixed-size fields only, and the token's owed entry exists
from the first open. So `set_ready`, `commit_claim`, `claim`, `refund`, `withdraw` and
`on_payout` never need new storage, and they cannot fail because the escrow lacks
storage balance.

## Gas

Measured in the sandbox (nearcore 2.14.1, gas price 10^8 yoctoNEAR per gas;
`tests/sandbox.rs` and docs/experiments/light-payout.md):

| Call | Burnt (Tgas) | Smallest prepaid gas that works | Clients attach |
|---|---|---|---|
| maker deposit into its Verifier account (`ft_transfer_call`, wNEAR) | 8.1 | | 100 |
| open (`mt_transfer_call` → `mt_on_transfer` → resolve) | 14.95 | | 100 |
| `commit_claim` (ed25519 check) | 2.6 | | 15 |
| `claim`, payout left Due | 16.6 | 17 | 25 |
| `claim` that also pays | 24.0 | 34 | (anyone with more NEAR) |
| `withdraw(id)` (escrow 2.64, Verifier `mt_transfer` 3.93, `on_payout` 2.24) | 9.6 | 19 | 25 |
| `refund` that also pays | about 24 | 34 | 50 |

Gas reserved by the contract (constants in `src/lib.rs`): `mt_transfer` 7 (plus all gas
left over), `on_payout` 8, and `GAS_PAYOUT_START` = 7 + 8 + 3 for the receipt fees.

### The taker's stipend

Before NEAR accepts a transaction, the signer must hold its attached gas at
`max(gas price, min_gas_purchase_price)` = 10^9 yoctoNEAR per gas today, plus fees
(nearcore 2.14.1, `calculate_tx_cost`; docs/experiments/light-payout.md). The taker's key
sends `commit_claim` (15 Tgas), `claim` (25) and `withdraw` (25). Measured
(`stipend_alone_completes_commit_claim_and_payout`):

- the largest balance NEAR checks in that sequence is 0.0278 NEAR (before `withdraw`);
- the three calls cost 0.0028 NEAR in total;
- a new implicit account holding exactly 0.06 NEAR (or 0.03 NEAR) completes all three
  alone and ends with the tokens under its key in the Verifier.

The maker sends 0.06 NEAR: 2.16 times the largest balance checked.

## Build and test

```sh
cd contracts/near-escrow
source ../../.tools/env.sh

# Contract
cargo build --target wasm32-unknown-unknown --release
# -> target/wasm32-unknown-unknown/release/near_escrow.wasm

# Unit tests (near-sdk mocked blockchain)
cargo test --lib

# Sandbox tests (live wrap.near and intents.near code from $ZECRET_WASM_DIR)
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox -- --nocapture --test-threads 1

# Review 5's sandbox tests (gas sweeps, the backing check during a payout, operator powers)
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test review5 -- --nocapture --test-threads 1

# Sandbox tests with the USDC code from mainnet. Download it once (not in git):
./fetch-usdc-wasm.sh        # -> $ZECRET_WASM_DIR/usdc.near.wasm, checks the code hash
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test usdc -- --nocapture --test-threads 1

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown --release -- -D warnings
```

Without `usdc.near.wasm`, each test in `tests/usdc.rs` fails with a message that names
the missing file and `fetch-usdc-wasm.sh`. To skip them on purpose, set
`ZECRET_SKIP_USDC=1`: each test then prints `SKIPPED` and passes, and the run tested
nothing. `fetch-usdc-wasm.sh` reads the code with `view_code` from
`https://free.rpc.fastnear.com` (set `NEAR_MAINNET_RPC` to use another) and writes the
file only if its code hash is `DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q`.

`.cargo/config.toml` adds `--cfg near` for the wasm target. cargo-near sets this flag.
Without it near-sdk does not call the NEAR host functions for `panic_str` and the hash
functions: a contract panic then traps with no message, and sha256 runs in wasm. The
sandbox tests build the wasm themselves unless `ESCROW_WASM` names a file.

The code hash and size of the reviewed build are recorded in
`deploy/deployment.testnet.json` (section `light_payout`).

### What the tests cover

Unit tests (`src/tests.rs`): every transition; every rejection (wrong state, too early,
too late, not the maker, wrong share, commitment mismatch, bad signature, signature for
another swap, another escrow or another commitment, non-canonical scalar, zero scalar,
invalid and non-canonical points, identity, duplicate swap id, each open parameter);
opens only from the Verifier, with one nep141 token owned by the sender; storage funds
added for another account; the review-1 F-01 proof of concept; the punish path from Open
and from Ready; each of the three refund windows and their edges; the late commit
signature replayed at `t2`; a grid over times and paths that checks exactly one exit and
that nothing is paid before `on_payout`; the payout receipts (recipient, token id,
amount, 1 yoctoNEAR, gas); claim and refund without the payout gas (Due, then
`withdraw`); a failed payout (Due again, one retry pays); every sequence of up to six
payout results (paid at most once, owed falls once); `withdraw` in each state;
`on_payout` only from Sending; the owed sum over several swaps; `on_payout` within its
reserved gas in the mock; G and the encodings against `reddsa` and the Solidity vectors.

Sandbox tests (`tests/sandbox.rs`), with the live `wrap.near` and `intents.near` code:

- the taker's new key, holding only the stipend, sends `commit_claim`, `claim` and
  `withdraw` itself; the tokens end under its key in the Verifier; it then signs one
  `ft_withdraw` intent and the tokens reach another account; `on_payout` is private;
- the smallest prepaid gas for `claim`, for a `claim` that also pays, and for `withdraw`;
- refunds pay the maker inside the Verifier (in the same call, or later by a stranger's
  `withdraw`); the maker takes plain tokens out with `ft_withdraw`;
- bad opens are refunded by the Verifier; an account that is not the Verifier cannot
  open; a used swap id and a two-token batch are refused;
- the stipend measurement above (0.06 and 0.03 NEAR), and the cost NEAR checks for each
  call, read from `NotEnoughBalance`;
- the backing check over several swaps, payouts, a Due refund and a gift;
- the Verifier pauses `mt_transfer` or locks the escrow's account: the payout fails,
  then one retry pays once;
- two `withdraw` calls in the same block pay once;
- `new` is private; the documented deployment leaves the reviewed code, no access keys
  and the configured Verifier.

USDC sandbox tests (`tests/usdc.rs`), with the USDC code from mainnet, initialised as in
`circlefin/stablecoin-near`. A second swap stays open in each test as "other swaps'
funds" and is refunded at the end to show it is intact:

- a normal swap: no USDC receipt in the payout;
- a pause, a blocklist of the Verifier, of the escrow and of the maker, each in place
  while swaps are claimed, paid and refunded: no effect;
- each restriction before the maker's deposit: the deposit fails, nothing moves, an open
  above the Verifier balance leaves no swap;
- the race of nv02-usdc.md moved to the maker's deposit: each restriction sent 0 to 5
  blocks after the deposit; at 0 to 2 blocks USDC reports failure after the Verifier
  credited the maker; the maker's USDC is unchanged in total and it opens from what the
  Verifier shows;
- each restriction sent 0 to 4 blocks after the maker's `ft_withdraw` out of the
  Verifier: the maker's USDC is unchanged in total;
- payouts of two swaps and a duplicate `withdraw` race with a Verifier pause sent 0 to 3
  blocks later: every swap is paid exactly once, and the escrow ends holding exactly what
  it owes.

Review 5 tests (`tests/review5.rs`, written by the reviewer of
[`docs/security/review-5.md`](../../docs/security/review-5.md)): `claim` and `refund` swept
from the smallest working gas to 44 Tgas (and every 0.001 Tgas at the payout threshold of
about 33.13 Tgas) with no failing band; `on_payout`'s gas margin with the longest ids; the
backing check during a payout in flight (it fails closed only until `on_payout`); a
Verifier operator moving the escrow's tokens (the payout stays Due).
