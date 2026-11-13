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

- `claim(a)` credits the taker and stores `a`. The maker reads `a` and takes the ZEC with `a + b`.
- `refund(b)` credits the maker and stores `b`. The taker reads `b` and takes the ZEC back.

States and windows (`t0 < t1 < t2`, `W` = `reveal_window`, times in seconds):

```text
Open       --set_ready (maker, before t0)-->                   Ready
Ready      --commit_claim (taker-signed, before t1)-->         Locked
Open/Ready --commit_claim (taker-signed, from t2)-->           Locked   (punish path)
Locked     --claim(a) (before locked_at + W)-->                Claimed
Open       --refund(b) before t0-->                            Refunded
Open/Ready --refund(b) in [t1, t2)-->                          Refunded
Locked     --refund(b) from locked_at + W-->                   Refunded
```

`claim` and `refund` make no cross-contract calls. They only record a credit. Tokens
move in `withdraw`. A problem on the token side cannot make a call that publishes a
share fail.

## Interface

All byte strings in arguments and views are hex, 64 characters for 32 bytes, with no
`0x`. Times are seconds since the Unix epoch (`env::block_timestamp() / 10^9`).

### Init

`new(intents_account: AccountId)`, private. `intents_account` is the NEAR Intents
verifier (`intents.near` on mainnet). Only the escrow account itself can call `new`.
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
3. Check the result with the three checks below. Publish the code hash.

`deploy_escrow` in `tests/sandbox.rs` deploys the escrow this way, and
`new_is_private_and_deployment_leaves_no_keys` checks it.

Before the taker funds the shared Zcash address, she checks the escrow at a final block:

- `view_access_key_list` of the escrow account is empty;
- the code hash (`view_account`, `code_hash`) equals the hash of the reviewed wasm
  (sha256 of the wasm file, base58);
- `get_intents_account()` returns the Intents account she expects (`intents.near` on
  mainnet).

If one check fails, she does not fund.

### Storage funds

Each new swap uses contract storage (742 bytes measured, 0.00742 NEAR, when both credit
entries are new). The maker pays for it in advance, per token:

- `add_storage_funds(token, account_id?)`, payable. Adds the attached NEAR to the
  caller's funds for swaps in `token`. Returns the new balance. `account_id` is optional;
  if it is given and is not the caller, the call fails with `BadParams`. Nobody can add
  funds for another account: such funds would let a fake token open a swap in that
  account's name and take its swap id (review-1 F-01).
- `withdraw_storage_funds(token, amount?)`, exactly 1 yoctoNEAR attached. Sends unused
  funds back to the caller.
- `get_storage_funds(account_id, token)`, view.

### Open

The maker calls `ft_transfer_call` on the token:

```json
{
  "receiver_id": "<escrow>",
  "amount": "3000000",
  "msg": "{\"salt\":\"<32 bytes>\",\"taker_pk\":\"<32 bytes>\",\"a\":\"<A, 32 bytes>\",\"b\":\"<B, 32 bytes>\",\"t0\":1763000000,\"t1\":1763001000,\"t2\":1763002000,\"reveal_window\":600}"
}
```

The escrow's `ft_on_transfer(sender_id, amount, msg)` checks the terms and stores the
swap. The token is the caller (`predecessor_account_id`). The maker is `sender_id`. It
returns `"0"` (all tokens kept). On bad input it panics, and the token returns the whole
amount to the maker. It rejects:

- `amount` = 0; not `now < t0 < t1 < t2`; `reveal_window` = 0; `taker_pk` all zero;
- `a` or `b` that is not a canonical, non-identity Pallas point;
- unknown fields in `msg` (for example `relayer_fee`);
- a swap id that is already used (`BadState`);
- maker storage funds for this token below the storage cost (`InsufficientStorage`).

### Transitions

| Method | Caller | Allowed | Effect |
|---|---|---|---|
| `set_ready(id)` | maker only | Open, before `t0` | Ready |
| `commit_claim(id, commitment, signature)` | anyone | Ready before `t1`, or Open/Ready from `t2` | Locked; stores `commitment` and `locked_at` |
| `claim(id, a)` | anyone | Locked, before `locked_at + W`; `commitment_of(id, a)` matches; `a` canonical, not 0, `a·G = A` | Claimed; stores `a`; credits the taker |
| `refund(id, b)` | anyone | Open before `t0`; Open/Ready in `[t1, t2)`; Locked from `locked_at + W`; `b` canonical, not 0, `b·G = B` | Refunded; stores `b`; credits the maker |
| `withdraw(token, owner)` | anyone | owner has a credit | pays the credit (see below) |

`signature` (64 bytes) is the ed25519 signature by `taker_pk` over the bytes that the view
`commit_message(id, commitment)` returns.

### Withdraw

A credit is zeroed before its transfer starts. Then:

- A maker credit (route `direct`) goes out by `ft_transfer(owner, amount)`.
- A taker credit (route `intents`) goes out by
  `ft_transfer_call(intents_account, amount, msg = owner)`. Intents credits the account
  named in `msg`, which is the taker's implicit account. That account needs no NEAR and
  does not need to exist on chain.

The callback `on_withdraw` (private, 10 Tgas reserved) puts back what was not delivered:

| Result | Delivered | Put back |
|---|---|---|
| `ft_transfer` succeeded | amount | 0 |
| `ft_transfer` or `ft_transfer_call` failed | 0 | amount |
| `ft_transfer_call` returned `used` | `min(used, amount)` | the rest |
| `ft_transfer_call` succeeded with an unreadable result | amount | 0 |

The restore adds to the credit, so credits that arrive while a withdraw is in flight are
kept. Gas reserved: `ft_transfer` 15 Tgas, `ft_transfer_call` 60 Tgas (plus the unused gas
of the call), callback 10 Tgas. Attach 100 Tgas to `withdraw`. With too little gas the
whole call fails and the credit is unchanged.

One assumption: a failed `ft_transfer_call` result means nothing was delivered. That
holds for tokens built on the standard NEP-141 code (`wrap.near`, tested in the
sandbox), because the token reserves fixed gas for its own `ft_resolve_transfer` and no
caller can make it fail. A token whose resolve step can fail after the receiver kept the
tokens is paid twice: the credit is restored, and a second `withdraw` pays again from
the tokens of other swaps in the same token. The taker checks the token of a swap before
she sends ZEC.

**Warning: not checked for USDC (review-1 NV-02).** The product token is Circle's USDC
on NEAR (`17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`). Its source
is not in this repository, and the assumption above has not been checked for it. Before
mainnet, read its source and check that:

- `ft_resolve_transfer` cannot panic once the receiver's `ft_on_transfer` has run: no
  pause, blocklist, freeze or role check in it, and no other check that can fail;
- it reserves fixed gas for `ft_resolve_transfer`, and 60 Tgas (`GAS_FT_TRANSFER_CALL`)
  for `ft_transfer_call` is enough for its `ft_transfer_call` into `intents.near`;
- the result of `ft_transfer_call` is the JSON string of the amount used;
- a pause or blocklist of the escrow, of `intents.near` or of the taker between the
  transfer and the resolve step cannot cause a failed result after a delivery.

Then run `failed_withdraw_restores_credit` with the USDC wasm as the token, with a case
that pauses or blocklists between `ft_on_transfer` and `ft_resolve_transfer`. Do not
deploy to mainnet with USDC until this is done.

### Views

- `get_swap(id)`: the swap, or `null`. Fields: `id, maker, token, amount, taker_pk,
  taker_account, a_pub, b_pub, t0, t1, t2, reveal_window, state, locked_at, commitment,
  a, b`. `locked_at` and `commitment` are `null` until `commit_claim`; `a` is set only in
  Claimed; `b` only in Refunded. Clients read the published share here.
- `get_credit(token, owner)`: `{ "direct": "<u128>", "intents": "<u128>" }`.
- `swap_id(maker, salt)`, `commit_message(id, commitment)`, `commitment_of(id, a)`.
- `get_intents_account()`, `get_storage_funds(account_id, token)`.

### Events

NEP-297 logs: `EVENT_JSON:{"standard":"near_escrow","version":"1.0.0","event":...,"data":[...]}`
with events `opened`, `ready`, `locked`, `claimed` (has `a`), `refunded` (has `b`) and
`withdrawn` (`token, owner, route, amount` delivered, `restored`). `withdrawn` is
logged by the callback, so it states what the token actually did.

### Errors

A panic message starts with a code, then `: ` and a detail. The codes are the Solidity
error names: `BadParams`, `BadState`, `TooEarly`, `TooLate`, `BadSecret`,
`BadSignature`, `NotMaker`, `NotClaimable`, plus `InsufficientStorage`.

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
| `open(...)` with `transferFrom` and a balance check (audit L-01) | `ft_transfer_call` → `ft_on_transfer` | NEP-141 pattern. The escrow records the amount the token reports as transferred. |
| `payout` address, secp256k1 `ecrecover`, EIP-191 digest with `chainid` | `taker_pk` (ed25519), `env::ed25519_verify`, domain-separated message with the escrow account id | One taker key on NEAR. The escrow account id binds the network in practice; there is no chain id on NEAR. |
| `relayer`, `relayerFee` | removed | The maker sends a NEAR stipend to the taker key; no relayer (design decision of 2026-11-13). |
| keccak256 over `abi.encode` | sha256 with domain strings | Native NEAR host function. |
| `uint256` scalars and `(x, y)` points | 32-byte little-endian scalars and compressed points | The client's native encodings. Mapping above. |
| Shares only in events | Shares also stored in the swap (`a`, `b`) | Clients read them with a view call at a final block. |
| One credit per (token, owner) | One credit per (token, owner, route) | Maker credits go out by `ft_transfer`, taker credits into Intents. A maker that is an implicit account could also be a taker; the routes stay apart. |
| `withdraw` reverts if the transfer fails | Async transfer and callback that restores the credit | NEAR cross-contract calls are asynchronous. |
| No storage payment | Maker prepays storage per token | `ft_on_transfer` has no attached NEAR. Without it, anyone could fill the escrow's storage and block later calls. Funds are keyed by token because `ft_on_transfer` trusts the calling token for `sender_id`; a fake token cannot spend a maker's funds for a real token. Only the owner can add funds for itself, so a fake token has no funds in a maker's name and cannot open swaps or take swap ids in that name. Before this rule (review-1 F-01), a third party could fund (maker, fake token) and take the maker's swap id. |
| Unknown id: `setReady` → `NotMaker`, `refund` → `TooEarly` | Unknown id → `BadState` everywhere | Clearer. Behaviour on known swaps is unchanged, including `refund` on Claimed/Refunded → `TooEarly`. |
| `payout != 0` | `taker_pk` not all zero | Same intent. |

All later transitions write fixed-size fields only, and both credit entries exist from
`open`. So `set_ready`, `commit_claim`, `claim`, `refund` and the callback never need new
storage, and they cannot fail because the escrow lacks storage balance.

## Gas

Measured in the sandbox (nearcore 2.14.1, gas price 10^8 yoctoNEAR per gas), total gas
burnt by the whole transaction including token and Intents receipts:

| Call | Tgas | Attached in the tests |
|---|---|---|
| open (`ft_transfer_call` → `ft_on_transfer`, 2 point decompressions) | 11.6 | 100 |
| `set_ready` | 2.3 | default |
| `commit_claim` (ed25519 check) | 2.6 | 10 |
| `claim` (one Pallas scalar multiplication) | 16.8 | 25–100 |
| `refund` (one Pallas scalar multiplication) | 16.8 | 100 |
| `withdraw`, maker (`ft_transfer` + callback) | 7.0 | 100 |
| `withdraw`, taker (`ft_transfer_call` into Intents + callback) | 42.4 | 100 |
| `withdraw`, transfer failed, credit restored | 7.2 | 100 |
| `withdraw`, Intents account without code, credit restored | 11.2 | 100 |
| `intents.execute_intents` (`ft_withdraw`, signed by the taker key) | 25.3 | 50 |

### The taker's stipend

Before NEAR accepts a transaction, the signer must hold the attached gas at a pessimistic
price (about 10 times the gas price in these tests) plus fees. Measured
(`stipend_needed_for_taker_commit_and_claim`):

- With 0.01 NEAR the taker key cannot send a `commit_claim` with 10 Tgas attached
  (`NotEnoughBalance`, cost 0.0109 NEAR).
- With 0.03 NEAR it sends `commit_claim` (10 Tgas) and `claim` (25 Tgas). After the gas
  refunds arrive, 0.0281 NEAR is left: the two calls cost about 0.002 NEAR.

So the stipend in design step 11 must be about 0.03 NEAR, not 0.01 NEAR, if the taker
sends commit and claim herself. Most of it stays on the taker key. `withdraw` can be sent
by anyone, for example the maker.

## Build and test

```sh
cd contracts/near-escrow
source ../../.tools/env.sh

# Contract
cargo build --target wasm32-unknown-unknown --release
# -> target/wasm32-unknown-unknown/release/near_escrow.wasm (about 470 KB)

# Unit tests (near-sdk mocked blockchain)
cargo test --lib

# Sandbox tests (live wrap.near and intents.near code from $ZECRET_WASM_DIR)
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox -- --nocapture --test-threads 1

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown --release -- -D warnings
```

`.cargo/config.toml` adds `--cfg near` for the wasm target. cargo-near sets this flag.
Without it near-sdk does not call the NEAR host functions for `panic_str` and the hash
functions: a contract panic then traps with no message, and sha256 runs in wasm. The
sandbox tests build the wasm themselves unless `ESCROW_WASM` names a file.

The wasm is about 470 KB, so the escrow account locks about 4.7 NEAR for its code.

### What the tests cover

Unit tests (`src/tests.rs`): every transition in the table above; every rejection (wrong
state, too early, too late, not the maker, wrong share, commitment mismatch, bad
signature, signature for another swap, another escrow or another commitment, non-canonical
scalar, zero scalar, invalid and non-canonical points, identity, duplicate swap id, each
`open` parameter); storage funds added for another account; the review-1 F-01
proof of concept (a third party cannot take a maker's swap id); the punish path from Open and from Ready; each of the three refund
windows and their edges; the late commit signature replayed at `t2`; a grid over times
and paths that checks exactly one exit; storage funds; withdraw receipts, arguments and
every callback outcome; G and the encodings against `reddsa` and the Solidity vectors.

Sandbox tests (`tests/sandbox.rs`), with the live `wrap.near` and `intents.near` code:

- happy path: open → `set_ready` → `commit_claim` and `claim` by the taker's implicit
  account → `withdraw` → the taker holds the amount inside Intents → the taker key
  signs one `ft_withdraw` intent and the tokens reach another account;
- refund before `t0` (sent by a third party) and refund in `[t1, t2)` after sandbox
  fast-forward, then the maker's withdraw;
- withdraw failures restore the credit: maker unregistered on the token; Intents account
  unregistered; Intents account without code (unused amount returned);
- bad opens are refunded by the token; nobody can add storage funds for another
  account; a fake token cannot open in a maker's name;
- `new` is private; the documented deployment leaves the reviewed code, no access keys
  and the configured Intents account;
- the stipend measurement above.
