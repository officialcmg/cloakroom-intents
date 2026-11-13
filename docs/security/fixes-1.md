# Fixes for security review 1

Date: 2026-11-13. Scope of these fixes: `contracts/near-escrow/` and
`crates/zecret-intents/`. Review: [`review-1.md`](review-1.md). Nothing was committed.

## 1. Summary

| Id | State | Where |
|---|---|---|
| F-01 | Fixed | `contracts/near-escrow/src/lib.rs`, `add_storage_funds` |
| F-02 | Fixed | `crates/zecret-intents/src/check.rs`, `check_quote` |
| NV-01 | Fixed in the contract and the README. The taker checks belong to `zecret-core` (section 4). | `contracts/near-escrow/src/lib.rs`, `new`; `contracts/near-escrow/README.md`, "Deployment" |
| NV-02 | Open. It needs the USDC source (section 5). | `contracts/near-escrow/README.md`, "Withdraw" |
| H1 | For the maker team (section 4) | `crates/zecret-core/src/maker.rs` |
| H2 | Not done; a possible later hardening (section 3) | — |
| H3 | Documented: deploy to a named account | `contracts/near-escrow/README.md`, "Deployment" |
| H4 | Partly done: versioned nonce with deadline = intent deadline. The salt is not compared. | `crates/zecret-intents/src/check.rs` |
| H5 | Done | `crates/zecret-intents/src/signer.rs` |
| H6 | For the maker team (section 4) | `crates/zecret-core/src/maker.rs` |
| H7 | No change; parameter work (section 3) | `docs/DESIGN.md` §7 |
| H8 | Part of the NV-02 check (section 5) | — |

Each fix has a regression test. Each test was run on the code without the fix and
failed, then passed with the fix.

## 2. Changes

### F-01: swap id squatting through a fake token

- `add_storage_funds(token, account_id?)` adds funds only for the caller. If
  `account_id` is given and is not the caller, the call fails with
  `BadParams: storage funds can only be added for the caller`. The parameter was kept,
  so the ABI does not change. No client passes it (`zecret-core/src/near/escrow.rs`
  sends only `token`).
- Tests:
  - `poc_f01_third_party_cannot_take_the_makers_swap_id` (the review's proof of concept,
    with its helper `poc_tx`), in `src/tests.rs`.
  - `add_storage_funds_only_for_the_caller`, in `src/tests.rs`.
  - In the sandbox test `bad_open_is_refunded_by_the_token`: a third account's
    `add_storage_funds` for the maker in a fake token is refused, and the fake token's
    `ft_on_transfer` in the maker's name then fails.
- The comment on `fake_token_cannot_spend_maker_storage_funds` now says that keying
  funds by token alone does not stop the attack; the caller-only rule does.
- README: "Storage funds" and the storage row of the differences table.
- The swap id formula was not changed (another component depends on it).

### NV-01: `new` and escrow immutability

- `new` is `#[init] #[private]`. Only the escrow account itself can call it. Another
  account gets `Method new is private`.
- `get_intents_account()` already existed. It is the view a client uses to check the
  Intents account. No new view was needed.
- The sandbox helper `deploy_escrow` now deploys as the README says: the escrow account
  sends one transaction to itself with `DeployContract`, `new` and `DeleteKey`.
- Test `new_is_private_and_deployment_leaves_no_keys` (sandbox):
  - another account calls `new` on a deployed, uninitialised escrow, and it fails;
  - the escrow account itself can call `new`;
  - after the documented deployment, the escrow has no access keys, its code equals the
    built wasm, and `get_intents_account` returns the configured account;
  - a second `new` fails.
- `#[private]` is checked only in the wasm entry point that near-sdk generates, so the
  unit tests (which call the Rust method) cannot test it. The sandbox test does.
- README "Deployment": the steps, and the three checks the taker makes.

Deployment in one transaction: the review says "create the account, deploy, call `new`
and delete the key in one batched transaction". With `#[private]`, `new` must be called
by the escrow account itself. A transaction from the parent account has the parent as
predecessor, so it cannot call `new`. The README therefore uses two transactions:

1. The parent creates the escrow account with one temporary full-access key. The
   account has no code, so there is nothing to initialise.
2. The escrow account signs one transaction to itself: `DeployContract`, `new`,
   `DeleteKey` of the signing key. These run together or not at all.

`crates/zecret-node/src/devnet.rs` (`deploy_locked`) already does the same.

### F-02: quote freshness from signed data

- `Policy::max_quote_age` (default 5 minutes) and `MAX_QUOTE_CLOCK_AHEAD` (1 minute).
- In `check_quote`, after the existing deadline tests, the signed `timestamp` (read from
  the answer as received, which is what the signature covers) must satisfy
  `timestamp + max_quote_age > now` and `timestamp <= now + 1 min`.
- New refusals: `Refusal::QuoteTooOld { timestamp, max_age_secs }` and
  `Refusal::QuoteFromFuture { timestamp }`.
- The `quoteRequest.deadline` test stays as an extra check.
- Tests in `tests/recorded.rs`:
  - `poc_f02_expired_request_deadline_cannot_be_hidden` (the review's proof of concept);
  - `quote_freshness_comes_from_the_signed_timestamp`: the edges at 5 minutes old and
    1 minute ahead, a longer policy, and an edited timestamp breaks the signature.
- Every test passes an explicit "now". `Case::now` is the fixture's `recorded_at`. The
  recorded quotes were made 0.3 s to 1.7 s after it.
- `crates/zecret-intents/README.md`: new sections "Quote freshness" and "Nonce", and
  the test list.

**`docs/INTENTS.md` needs these changes** (not edited here; that file is outside this
scope):

- §4 "The quote", rule 7: "The request deadline and the deposit deadline are in the
  future." Change it to: "The signed `timestamp` is at most `max_quote_age` (5 minutes)
  old and at most 1 minute in the future. The deposit deadline is in the future. The
  request deadline is also tested, but 1Click does not sign it on a firm quote."
- §4 "The quote", rule 1: "The signature covers the deposit address, recipient, assets,
  amounts, refund address and deadlines." On a firm quote it covers `quote.deadline`
  and `timestamp`, not `quoteRequest.deadline`. Say so.
- §4 "The payload": add "The nonce is versioned and its deadline equals the intent
  deadline" (H4).
- §6 errors table, "Quote expired before signing": add `Refusal::QuoteTooOld` and
  `Refusal::QuoteFromFuture` (a wrong local clock), and `Refusal::BadNonce`.
- §8 tests table: the recorded test count is now 21, not 18.

### H4: generated nonce (partly)

`check_generated_intent` refuses a payload whose nonce is not versioned, or whose nonce
deadline is not the intent deadline (`Refusal::BadNonce`). All recorded 1Click payloads
pass. Test: `refuses_a_legacy_or_long_lived_nonce`.

Not done: comparing the salt with `current_salt`. The crate does no I/O. The caller
would have to read `current_salt` at a final block and pass it in. The Verifier refuses
an invalid salt itself, so the gain is small.

### H5: key copies

`IntentSigner::generate` keeps the seed in `zeroize::Zeroizing`, and `seed()` returns
`Zeroizing<[u8; 32]>` (it dereferences to `[u8; 32]`). `zeroize` 1.9.1 was already in
`Cargo.lock` through ed25519-dalek. No caller in the repository uses `seed()`.

## 3. Not done here

- **H2, the token in the swap id.** A possible later hardening:
  `sha256("near-escrow/v2/swap-id" ‖ len(maker) ‖ maker ‖ len(token) ‖ token ‖ salt)`.
  It changes the id formula in the contract and in every client, so it was not done.
  After the F-01 fix a third party cannot fund (maker, fake token), so the attack does
  not need it.
- **H3, a chain tag in the commit message.** It changes the signed message format in
  the contract and the clients. Instead the README requires a named escrow account.
- **H7, block timestamps.** Nothing to change in code. Keep the mainnet window
  parameters measured (DESIGN.md §7).

## 4. Requirements for the other teams

The escrow wasm changed, so its code hash changed. Any configured
`escrow_code_hash` must be recomputed from the new build.

**Taker (`crates/zecret-core/src/taker.rs`, NV-01).** Before the taker funds the shared
address, at a final block:

1. `view_access_key_list` of the escrow account is empty.
2. `view_account.code_hash` equals the configured hash of the reviewed wasm.
3. `get_intents_account()` equals the configured Intents account (`intents.near` on
   mainnet).

If one check fails, do not fund. At review time, `near/escrow.rs::check_escrow` and a
call in `taker.rs` existed for this; confirm it checks all three, at a final block,
before every funding.

**Taker and app (F-02).** `execute_exit` and `check_generated_intent` take `now`.
Pass the current clock, read right before the call, not a stored value. If the call
fails with `QuoteTooOld`, get a new firm quote. If it fails with `QuoteFromFuture`, the
local clock is behind; show that to the user.

**Maker (`crates/zecret-core/src/maker.rs`, H1).** In `open_escrow`, when a swap
already exists at the maker's swap id, compare `maker`, `token`, `amount`, `taker_pk`,
`a_pub`, `b_pub`, `t0`, `t1`, `t2` and `reveal_window` with the maker's own record.
Send the stipend or call `set_ready` only if all are equal. Otherwise abort the swap
and log it. At review time `not_ours` existed for this; confirm it compares every
field.

**Maker (H6).** The maker daemon must:

1. refund in `[t1, t2)` every swap of its own that is still Open or Ready; otherwise,
   from `t2`, the taker can commit and claim through the punish path;
2. alert when one of its swaps is near `t1` or `t2` and not in a terminal state;
3. after `t2`, expect that only the taker can settle an Open or Ready swap.

**Deployment (NV-01, H3).** Testnet and mainnet escrows are deployed as in the escrow
README, "Deployment", to a named account. Publish the code hash. The deployment script
checks that the key list is empty afterwards.

## 5. NV-02: what must be checked about USDC before mainnet

The escrow restores an Intents credit when `ft_transfer_call` fails
(`on_withdraw`, `PromiseError::Failed`). That is safe only if the token's
`ft_resolve_transfer` cannot fail after the receiver kept the tokens. Otherwise a second
`withdraw` pays again from other swaps' USDC.

For the USDC contract at
`17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`, the owner must
read the source of the deployed code (match it to the on-chain code hash) and confirm:

1. `ft_resolve_transfer` has no pause, blocklist, freeze or role check, and nothing
   else that can panic once `ft_on_transfer` of the receiver has run.
2. `ft_transfer_call` reserves fixed gas for `ft_resolve_transfer`, so the resolve
   step cannot run out of gas.
3. 60 Tgas (`GAS_FT_TRANSFER_CALL`) is enough for USDC's `ft_transfer_call` into
   `intents.near` (H8). Too little gas only delays the exit; the credit is restored.
4. The result of `ft_transfer_call` is the JSON string of the amount used.
5. Pausing USDC, or blocklisting the escrow, `intents.near` or the taker, between the
   transfer and the resolve step cannot make the result `Failed` after a delivery.
6. Whether the contract can be upgraded, and by whom. An upgrade can change all of the
   above.

Then run the sandbox test `failed_withdraw_restores_credit` with the USDC wasm as the
token, and add a case that pauses or blocklists between `ft_on_transfer` and
`ft_resolve_transfer`. If any item fails, change `on_withdraw` before mainnet. The
escrow README ("Withdraw") carries this warning.
