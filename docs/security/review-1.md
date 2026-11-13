# Security review 1: NEAR escrow and the Intents exit

Date: 2026-11-13. Reviewed code: the working tree of `/home/user/zecret` on that date
(the directory has no git metadata, so there is no commit id). Machine-readable
records: `findings-1.json` (validated with the method's `validate-findings.cjs`: 16
records, PASS). Proof-of-concept tests: `poc/`.

## 1. Result

| Id | Severity | Title |
|---|---|---|
| F-01 | Medium | A third party can take a maker's swap id with a fake token and block the maker's open |
| F-02 | Low | The quote freshness check reads `quoteRequest.deadline`, which 1Click does not sign on firm quotes |

Needs validation: NV-01 (escrow immutability and `new` front-running), NV-02 (USDC
resolve step and `on_withdraw`). No finding lets anyone take the escrowed tokens or the
ZEC, or makes the crate sign a transfer to an address, token or amount that the user
did not approve.

## 2. Scope and method

**In scope.**

1. `contracts/near-escrow/` (`src/lib.rs`, `src/curve.rs`, `src/hexbytes.rs`), compared
   with `contracts/src/CloakroomEscrow.sol` and `contracts/audit/REPORT.md`.
2. `crates/zecret-intents/` (all of `src/`), against `docs/INTENTS.md`.

Code outside the targets was read only where a finding depends on it:
`crates/zecret-core/src/{taker.rs,maker.rs,keys.rs,near/escrow.rs}` and
`crates/zecret-node/src/devnet.rs`.

**Method.** Cloudflare's published security-audit method
(github.com/cloudflare/security-audit-skill), in guidance mode, by one reviewer: recon,
hunt, validate, record. The companion files used were
PROTOCOLS-RPC-AND-MESSAGING, RESOURCE-EXHAUSTION-AND-AVAILABILITY,
DATA-ISOLATION-AND-LIFECYCLE and SUPPLY-CHAIN-AND-RELEASE. A finding is confirmed only
with a full source trace and a local test. Each candidate was argued against before it
was kept. A missing second layer of defence is a hardening note, not a finding.
Severity is likelihood × impact.

**Execution.** Only local unit tests ran. Nothing touched a deployed endpoint, testnet
or mainnet.

- Escrow: the existing 48 unit tests and the F-01 PoC ran in a scratch copy of the
  source, built into `contracts/near-escrow/target`. The PoC fails on the reviewed code.
  With the proposed fix, the PoC and all 48 tests pass.
- Intents: `cargo test --lib` (24 pass) and the F-02 probe on the recorded live answers
  ran in a scratch copy, built into `crates/zecret-intents/target`. That directory is
  new and uses 987 MB; delete it if the disk space is needed.
- The scratch copies were deleted. No crate source was changed.
- The sandbox tests were not run, because no finding needed them.

**Covered.**

- Escrow: every entry point; the state machine and its windows; the exactly-one-exit
  invariant; the claim/refund no-token rule; storage accounting; `ft_on_transfer`
  trust; predecessor and `sender_id` identity; withdraw, callback and gas; integer
  overflow; the commit message domain; front-running; Pallas and scalar decoding.
- Intents crate: the quote signature reconstruction; every rule in `check_quote` and
  `check_generated_intent`; strict payload decoding; NEP-413 and raw hashing; nonce
  handling; signer domains across raw, NEP-413, escrow commit and NEAR transaction
  signatures; panics on hostile input; key handling.

**Not covered.**

- The yellow paper was not read. The protocol rules were taken from DESIGN.md §3–§5
  and the Solidity escrow.
- The rest of `zecret-core`: the taker and maker state machines, memo crypto and
  wallet code.
- The website and the Web Worker.
- The Zcash side.
- The source of the Verifier (`intents.near`) and of Circle's USDC. Only wasm is in
  the repository. The parser-differential reasoning therefore rests on both sides
  using serde_json, not on reading the Verifier's types.
- No on-chain race was run for F-01.
- Deployment and key management (see NV-01).

## 3. Confirmed findings

### F-01 (Medium): swap id squatting through a fake token

**Where.** `contracts/near-escrow/src/lib.rs`

| Line | What happens |
|---|---|
| 308–311 | `add_storage_funds` takes the funds owner from `account_id`, a parameter any caller sets. |
| 378 | In `ft_on_transfer`, the caller becomes `token`. |
| 400 | `id = compute_swap_id(sender_id, salt)`. `sender_id` is the caller's claim, and the token is not in the id. |
| 447–464 | The storage gate checks only `(sender_id, token)` funds. |
| 401–402 | The maker's real open then fails with "swap id already used". |

**Lower-trust principal.** Any NEAR account.

**Attack.**

1. The attacker reads the salt from the maker's `ft_transfer_call` while it is pending
   or just included. The `msg` is public.
2. The attacker calls `add_storage_funds(token = fake-token, account_id = maker)` with
   about 0.01 NEAR attached.
3. The attacker calls `ft_on_transfer(sender_id = maker, amount = 1, msg = the maker's
   terms)` directly from `fake-token`. This path is one receipt long. The maker's path
   is two receipts (maker, then token, then escrow), so the attacker wins the race if it
   sends at the same time.
4. The maker's id now holds a swap with `token = fake-token` and `maker = maker`. The
   maker's open is refunded by USDC.

The existing test `fake_token_cannot_spend_maker_storage_funds` (tests.rs:546) and its
comment claim that this cannot happen. The test misses step 2. On Base the property
held (`check_open_cannotTouchOtherMakersId`, mutant M15, REPORT.md:138), because
`msg.sender` is authenticated. This is a regression in the port.

**Impact.**

- The salt is `escrow_salt(rfq_id, s_t, s_m)` (`zecret-core/src/keys.rs:145`), so the
  maker cannot retry with a new salt. The swap is dead.
- The taker has already paid the premium with the acceptance memo.
- On retry, the maker daemon accepts any existing swap at the id as its own and sends
  the 0.03 NEAR stipend (`maker.rs:596–603`).
- The taker is protected: `taker.rs:664` checks `token`, so it does not fund.
- No escrowed tokens or ZEC are lost.
- The attack can be repeated against every swap for about 0.0074 NEAR each. It is a
  denial of service on the whole service.

**Severity.**

- Likelihood: medium. The attacker needs a bot and a one-receipt race, and no funds.
- Impact: medium. It permanently blocks any chosen swap and costs the victims small
  amounts. Nothing is stolen.
- Overall: medium.

**Reproduce.** Append `poc/f01_swap_id_squat.rs` to
`contracts/near-escrow/src/tests.rs`, then run
`cd contracts/near-escrow && cargo test --lib poc_f01`. On the reviewed code it fails
with: `F-01: a third party (funded=true, squatted=true) created swap 6852…3147 under
the maker's id with token fake-token.near`. The PoC has a small helper, `poc_tx`, that
rolls back a call that panics, as the chain does. The mock keeps such writes.

**Fix (smallest).** `lib.rs`, `add_storage_funds`:

```rust
let owner = env::predecessor_account_id();
if account_id.is_some_and(|a| a != owner) {
    fail(ERR_BAD_PARAMS, "storage funds can only be added for the caller");
}
let key = FundsKey { owner, token };
```

No client passes `account_id` (`zecret-core/src/near/escrow.rs:164` sends only
`token`), so nothing else breaks. This was verified: 48 tests plus the PoC pass. Also
make these changes:

- the README "Storage funds" text (README.md:50);
- the comment on `fake_token_cannot_spend_maker_storage_funds`;
- a test that `add_storage_funds` with another `account_id` is refused.

Dropping the parameter also works, but it changes the ABI. Defence in depth (optional,
changes the id formula in the contract and both clients): add the token to the swap
id, `sha256(DOMAIN ‖ len(maker) ‖ maker ‖ len(token) ‖ token ‖ salt)`, and bump the
domain to `v2`.

### F-02 (Low): quote freshness rule uses an unsigned field

**Where.**

| Location | What happens |
|---|---|
| `crates/zecret-intents/src/check.rs:295–303` | Tests `quoteRequest.deadline > now`. |
| `crates/zecret-intents/src/quote_sig.rs:119–121` | Copies the SDK: on a firm quote, the signed message takes `deadline` from `quote.deadline` (the request deadline + 72 h). The request deadline is not signed. |

**Lower-trust principal.** A network attacker, or a compromised 1Click API without
1Click's quote key. This is the threat model of INTENTS.md §3.

**Attack.** After the 30-minute request deadline has passed, the attacker sets
`quoteRequest.deadline` to a future time. The attacker can also replay an old genuine
firm quote that has the same approved values. The signature still verifies, and
`check_generated_intent` returns `Ok`. This defeats quote rule 7 of INTENTS.md §3.

**Reproduce.** Append `poc/f02_request_deadline_unsigned.rs` to
`crates/zecret-intents/tests/recorded.rs`, then run
`cargo test --test recorded poc_f02`. It fails on the reviewed code. The same probe,
written as a positive test, passed for both `raw_ed25519` and `nep413`.

**Impact.** The transfer still goes to a genuine 1Click deposit address. That address
has a signed `refundTo` that is the taker's own Verifier account, with
`refundType: INTENTS`. By 1Click's documentation, a late deposit is refunded there. The
loss is delay, a retry and any signed `refundFee` (0 in the recordings). Funds are not
redirected.

**Severity.** Likelihood low, impact low, overall low.

**Fix.** In `check_quote`, take freshness from signed data:

- Add `Policy::max_quote_age` (default 5 minutes).
- Refuse the quote if the signed `timestamp` is older than that, or more than about
  1 minute in the future.
- Keep the `r.deadline` test only as an extra check.

This also refuses replayed old quotes. Add the PoC as a regression test.

## 4. Needs validation

### NV-01: the taker does not check that the escrow cannot change, and `new` can be front-run

**Fact that is not known.** No deployment script for testnet or mainnet is in the
repository. It is not known:

- whether the deployed escrow account keeps any access key;
- whether deploy and `new` are one batched transaction.

**Why it matters.**

- `devnet.rs:52,64` creates the escrow with a full-access key, deploys the code, and
  then calls `new` in a second transaction. `new` (lib.rs:292) is `#[init]` without
  `#[private]`.
- The taker's 10 checks (`taker.rs:663–674`) do not cover the escrow's code hash, its
  access keys or `get_intents_account()`.
- If the operator (who is also the maker) holds a key of the escrow account, it can
  redeploy the code after `claim` publishes `a`, and keep both the ZEC and the USDC.
- If `new` is a separate transaction, anyone can initialise the escrow first with their
  own contract as `intents_account`. Every taker payout then goes to that contract.

**Local check.** In the sandbox, deploy the escrow, then call `new` from a second
account first. Observe that the call succeeds.

**Owner check.** For the deployed escrow account, check that:

- `view_access_key_list` is empty;
- the code hash equals the reviewed wasm;
- `get_intents_account` returns `intents.near`.

**Fix.** Make these changes even if the deployment is correct today:

- Add `#[private]` to `new`.
- Create the account, deploy the code, call `new` and delete the key in one batched
  transaction.
- Make the taker check the code hash, an empty key list and `intents_account` at a
  final block before it funds.

### NV-02: `on_withdraw` and a token whose resolve step can fail after delivery

**Fact that is not known.** The source of the USDC contract at `17208628…133a1`. It is
not known whether its `ft_resolve_transfer` can panic after `intents.near` has accepted
the tokens, for example from a pause or blocklist check, or from its gas constants.

**Why it matters.** For the Intents route, `on_withdraw` (lib.rs:660) treats
`PromiseError::Failed` as "nothing delivered" and restores the full credit. If the
resolve step failed after delivery, a second `withdraw` would pay again from the USDC of
other swaps. With the standard implementation, as in wrap.near and the sandbox tests,
this cannot happen.

**Local check.** Run `failed_withdraw_restores_credit` with the real USDC wasm. Add a
case that pauses or blocklists between `ft_on_transfer` and the resolve step.

**Owner check.** Read Circle's NEAR USDC source for `ft_resolve_transfer`.

## 5. Rejected candidates

These are recorded in `findings-1.json` with reasons.

- **Exactly one exit.** The windows are disjoint half-open intervals. The terminal
  states are absorbing. `exactly_one_exit_over_time_grid` covers the time grid.
- **A token makes a share-revealing call fail.** `claim` and `refund` make no calls.
  They only overwrite entries that `open` created and paid for. "credit overflow" needs
  more than 2^128 credited units in one (token, owner) key, which USDC supply does not
  allow.
- **Commit signature replay across swaps, escrows or networks.** The signed message
  holds the domain, the escrow account and the id, and the id holds the maker and the
  salt. Replay on the same swap only performs the taker's own commit.
- **Front-running claim, refund or commit_claim.** The payees are fixed in the swap.
  The commit needs the taker's key. The claim and Locked-refund windows are disjoint.
- **Pallas and scalar input.** Only canonical, non-identity points decode. Pallas has
  prime order, so there are no small-order points. Scalars must be canonical and
  non-zero. Tests exist.
- **Withdraw reentrancy or double pay.** The credit is set to zero before the promise.
  The callback is private and has reserved gas. The one open case is NV-02.
- **Storage staking grief.** Each byte is paid by the account that opens the swap.
  Funds paid for another account are the payer's loss. The only abuse is F-01.
- **Predecessor vs signer confusion.** `signer_account_id` is never read. The
  unauthenticated identity is `sender_id` (F-01).
- **Parser differentials in the Intents payload.** The crate signs the exact bytes it
  checked. It refuses duplicate keys and unknown fields. It compares amounts and token
  ids as exact canonical strings.
- **Signature domain confusion.** The four message forms signed by the taker key are
  disjoint:
  - raw JSON;
  - the NEP-413 tagged hash;
  - the escrow commit message, with its `near-escrow/v1/commit-claim` prefix;
  - the NEAR transaction hash.
- **The nonce from 1Click is not checked.** A chosen nonce can only make the intent
  fail. See hardening note H4.
- **Panics on hostile input.** Non-test code has no `unwrap` or `expect`. Slices are
  bounded.

## 6. Hardening notes (not vulnerabilities)

- **H1. The maker retry path trusts any swap at its id.** In `maker.rs:596–603`,
  compare `token`, `amount`, `taker_pk`, `a_pub` and `b_pub` with its own record before
  it sends the stipend or sets Ready. After the F-01 fix nobody else can create the
  swap, but the check costs nothing.
- **H2. The swap id does not include the token.** See the optional F-01 change.
- **H3. The commit message has no network id.** The commit message relies on the escrow
  account name to tell networks apart. An escrow on an implicit (hex) account has the
  same name on testnet and mainnet. Deploy to a named account, or add a chain tag.
- **H4. The crate does not check the generated nonce.** Before signing, require a
  versioned nonce with the current salt and a nonce deadline equal to the intent
  deadline. Then a hostile API cannot pick a used or legacy nonce. The effect today is
  only a failed exit.
- **H5. Key copies.** `IntentSigner::generate` leaves the seed in a stack array, and
  `seed()` returns copies. Wrap them in `zeroize::Zeroizing`. `SigningKey` itself zeroes
  on drop through ed25519-dalek's default `zeroize` feature. This matters little,
  because by design the browser stores the keys anyway.
- **H6. Liveness duties.**
  - The maker must refund in [t1, t2) when Ready was not set. Otherwise the taker may
    claim through the punish path.
  - After t2, only the taker can settle an Open or Ready swap.
  - Both duties are the same as in the Solidity escrow and the paper. The maker daemon
    must alert on swaps that come near t1 and t2.
- **H7. Block timestamps.** The time windows use `block_timestamp`, which block
  producers set. The B_R budget absorbs normal skew. Keep the mainnet parameters
  measured, as DESIGN.md §7 says.
- **H8. Gas for the USDC transfer.** Check `GAS_FT_TRANSFER_CALL` (60 Tgas) against
  USDC's own requirement for `ft_transfer_call` on mainnet. If it is too low, the
  transfer fails and the credit is restored, so funds are safe. The exit is only
  delayed.
