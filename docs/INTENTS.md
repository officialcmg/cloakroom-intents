# The exit through NEAR Intents

This document describes step 10 of "How one swap runs" in `DESIGN.md`: the app signs
one message, and NEAR Intents sends the destination tokens to the user's address on the
selected chain. The code is `crates/zecret-intents`.

Everything below was checked on 2026-11-13 unless it says otherwise. "Measured" means
a request to the live 1Click API or a test against the Verifier code in a sandbox. The
section "Not verified" lists what needs real funds on mainnet.

## 1. Starting point

- The escrow pays the USDC into `intents.near` with `msg` set to the taker's account
  id. The taker's account id is the lowercase hex of the taker's ed25519 public key.
  This was the first design. The escrow now pays in two steps (the NV-02 fix,
  implemented and tested; `contracts/near-escrow/README.md`, `docs/security/fixes-2.md`):
  it deposits into its own Verifier account, then moves the tokens to the taker's
  account with `mt_transfer`. The taker's account id is the same in both designs.
- That account does not have to exist on NEAR. The Verifier accepts the key for that
  account without `add_public_key` (implicit account).
- USDC on NEAR is the 1Click asset
  `nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`
  (6 decimals).

## 2. The exit, step by step

| Step | Request | Who | Result |
|---|---|---|---|
| 1 | `GET /v0/tokens` | page | The list of destination assets (230 on 2026-11-13). |
| 2 | `POST /v0/quote`, `dry: true` | page | A price. The page shows `amountOut` and `minAmountOut`. The user approves an amount, an asset, a recipient and a minimum. |
| 3 | `POST /v0/quote`, `dry: false` | worker | A firm quote with a `depositAddress` (a Verifier account, 64 hex). |
| 4 | quote check | worker | The 1Click signature and the fields of the firm quote are checked against the approval (section 3). |
| 5 | `POST /v0/generate-intent` | worker | An unsigned payload for the taker's key. |
| 6 | payload check | worker | The payload is decoded and checked (section 3). If a check fails, nothing is signed. |
| 7 | sign | worker | One ed25519 signature, `raw_ed25519` or `nep413`. |
| 8 | `POST /v0/submit-intent` | worker | `intentHash`. The crate checks that it equals the hash of the signed payload. |
| 9 | `GET /v0/status?depositAddress=…` | page or worker | Poll until `SUCCESS`, `REFUNDED` or `FAILED`. |

All quote requests use:

```json
{
  "dry": false,
  "swapType": "EXACT_INPUT",
  "slippageTolerance": 100,
  "originAsset": "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1",
  "depositType": "INTENTS",
  "destinationAsset": "<asset id from /v0/tokens>",
  "amount": "<USDC, 6 decimals>",
  "refundTo": "<taker account id>",
  "refundType": "INTENTS",
  "recipient": "<address on the destination chain>",
  "recipientType": "DESTINATION_CHAIN",
  "deadline": "<now + 30 min, ISO 8601 UTC>"
}
```

`refundType: INTENTS` with `refundTo` set to the taker's account keeps any refund under
the taker's key inside the Verifier. The app can then try again.

What `/v0/generate-intent` returned (recorded, `raw_ed25519`):

```json
{"intent":{"standard":"raw_ed25519","payload":"{\"verifying_contract\":\"intents.near\",\"signer_id\":\"0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0\",\"deadline\":\"2026-11-16T03:47:00.000Z\",\"nonce\":\"Vij2xgAlKBKzAAiuJtZp6Bi2nNUwe8iiTwE+NLlqr4Q=\",\"intents\":[{\"intent\":\"transfer\",\"receiver_id\":\"4d95d25cdc3c8a577c6a41ff0ee8fa75658d9724786cc2ea643399571902c17f\",\"tokens\":{\"nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1\":\"5000000\"}}]}"},"correlationId":"1cac3d1d-ed9e-46f9-8d8f-d4368fe0aec9"}
```

So the exit is one `transfer` intent: the taker's USDC goes to the deposit address
inside the Verifier, and 1Click swaps and delivers from there. The `nep413` form has the
same content: `recipient` is `intents.near`, the nonce is in the payload, and `message`
holds `signer_id`, `deadline` and `intents`.

Facts about the generated payload (measured):

- The intent deadline equals the quote's `deadline`, which is the request deadline plus
  72 hours (in every firm quote made: 30 min, 1 min and 7 days requested).
- The nonce is a versioned nonce (magic `5628f6c6`) with the Verifier's current salt
  (`252812b3`, which is also what `current_salt` on `intents.near` returned) and a nonce
  deadline equal to the intent deadline.
- `intentHash` from `/v0/submit-intent` is base58 SHA-256 of the raw payload, or the
  NEP-413 hash for `nep413`. The crate's hash matched the API's hash for both
  standards.

## 3. The safety check

The payload from `/v0/generate-intent` comes from a third party over the network. The
crate does not sign it until it passes `check::check_generated_intent`. A failure
returns a `Refusal` and nothing is signed.

**The quote.**

1. 1Click's signature over the quote is valid. The algorithm and key
   (`ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc`) are taken from
   `verifyQuoteSignature` in `@defuse-protocol/one-click-sdk-typescript` 0.1.26. The
   signature covers the deposit address, recipient, assets, amounts, refund address,
   the quote's `timestamp` and one deadline. On a firm quote that deadline is
   `quote.deadline` (the deposit deadline): it replaces `quoteRequest.deadline` in the
   signed message, so `quoteRequest.deadline` is not signed. All recorded quotes (dry,
   firm and the copy in `/v0/status`) verify.
2. The quote is firm (`dry: false`) and has a deposit address and no deposit memo.
3. `swapType` is `EXACT_INPUT`; `depositType` and `refundType` are `INTENTS`;
   `recipientType` is `DESTINATION_CHAIN`.
4. `originAsset`, `amount`, `quote.amountIn`, `destinationAsset` and `recipient` equal
   the approved values. `refundTo` is the taker's account.
5. `customRecipientMsg`, `virtualChainRecipient` and `virtualChainRefundRecipient` are
   not set.
6. `quote.minAmountOut` is not below the minimum the user approved.
7. The signed `timestamp` is at most `max_quote_age` (5 minutes) old and at most
   1 minute in the future. The deposit deadline is in the future. The request
   deadline is also tested, but 1Click does not sign it on a firm quote.

**The payload.** It is decoded strictly: an unknown field, an unknown intent type, a
duplicate key or a NEP-413 `callbackUrl` is refused.

1. The standard is the one requested.
2. `verifying_contract` (NEP-413: `recipient`) is `intents.near`.
3. `signer_id` is the taker's account.
4. The deadline is in the future, not after the quote's deposit deadline, and not more
   than 96 hours from now.
5. The nonce is versioned and its deadline equals the intent deadline. The salt is
   not compared with `current_salt`; the Verifier refuses an invalid salt.
6. There is exactly one intent, and it is a `transfer`.
7. Its `receiver_id` is the quote's deposit address.
8. It moves exactly one token, the approved origin asset, and exactly the approved
   amount (as a canonical decimal string).
9. It has no memo and no notification (`msg`), so it cannot call a contract.

The signer can only sign an `UnsignedPayload`, and only the crate can make one: from
its own intent builder, or from a payload that passed this check. The signature covers
the exact bytes that were checked.

The recorded tests run every refusal on real API payloads, including a payload the live
API generated for `signerId: alice.near` (the API does not check that the signer owns
the quote's refund account).

What the check cannot prove: that 1Click delivers to the recipient. The quote signature
shows that 1Click promised to deliver to that recipient for that deposit address. The
delivery itself depends on 1Click and its solvers.

## 4. What needs an API key

Measured without any key or header, from this machine:

| Endpoint | Without a key |
|---|---|
| `GET /v0/tokens` | 200, full list |
| `POST /v0/quote`, dry | 201 |
| `POST /v0/quote`, firm | 201 with `depositAddress` |
| `POST /v0/generate-intent` | 201 with the payload |
| `POST /v0/submit-intent` | 201 with `intentHash` |
| `GET /v0/status` | 200 |
| any endpoint with `X-API-Key: not-a-real-key` | 401 `{"message":"Invalid token","error":"Unauthorized","statusCode":401,…}` |

The docs say that `generate-intent` and `submit-intent` use partner authentication. On
2026-11-13 they worked without it. The app needs **no API key** for the exit, as far as
can be checked without funds. A wrong key is worse than no key (401). The answers have
`access-control-allow-origin: *`, and the preflight allows `content-type` and
`x-api-key`, so the browser can call the API directly.

Rate limit without a key: `x-ratelimit-limit-unauth: 1200` per 60 s.

**Fees.**

- Without a key, every quote's `quoteRequest.appFees` contained a fee to account
  `5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd`: 1 bp for USDC to
  USDC on Base, 20 bp for USDC to SOL, ETH on Base and ZEC.
- The fees page says: without a key, 25 bp is added to every quote; with a key and no
  `appFees`, 20 bp (1 bp for stablecoin and same-asset routes). The measured `appFees`
  match the keyed schedule, not the 25 bp. Whether another 25 bp is inside the price
  could not be separated from the solver spread.
- `withdrawFee` is subtracted in `amountOut` already: 2400 (0.0024 USDC) to Base,
  124735 lamports to Solana, 32000 zatoshi to Zcash.
- The Verifier takes 1 pip (0.0001 %) per transfer.
- A key (a JWT from `partners.near-intents.org`) changes the fee schedule and allows the
  partner's own `appFees` split. If the app uses a key, the key must stay in the
  backend; a key in the browser is public.

Example prices for 5 USDC, dry, 2026-11-13 03:02 UTC: Base USDC 4.996757
(`minAmountOut` 4.946789 at 100 bp slippage, 12 s estimate); SOL 0.061934715 (9 s); ETH
on Base 0.002502303578251578 (14 s); ZEC 0.00492186 (127 s); BTC refused: "Amount is too
low for bridge, try at least 5909617".

## 5. Quote behaviour

Measured with `crates/zecret-intents/measurements/quote_drift.py`: a dry quote for 20
USDC every 20 s for 15 minutes (03:05 to 03:20 UTC, 2026-11-13).

| Route | Quotes | Range of `amountOut` | Worst fall within 60 s | Worst fall within 10 min | Later `amountOut` below an earlier `minAmountOut` |
|---|---|---|---|---|---|
| USDC to USDC on Base | 41 | 0.004 % | 0.004 % | 0.004 % | never |
| USDC to SOL | 40 (+1 error) | 1.140 % | 0.290 % | 0.324 % | never |
| USDC to ETH on Base | 40 (+1 error) | 1.051 % | 0.284 % | 0.284 % | never |

- `minAmountOut` is exactly `amountOut × (1 − slippageTolerance)`. It is not a separate
  guarantee from the solvers; it is the line below which 1Click refunds.
- Quote latency: median 0.6 to 0.8 s, max 1.2 s.
- Two errors at 03:16:07, for SOL and ETH at the same time: HTTP 400 "Quoting for this
  pair is not available". The next quote 20 s later worked. The crate classifies it as
  `NoQuote`, retryable.
- `timeEstimate`: 12 s to Base, 7 to 9 s to Solana, 12 to 14 s for ETH on Base, 127 s to
  Zcash.

Deadlines accepted by `/v0/quote`: any future instant from 10 s to 365 days (dry). A
past deadline gives 400 "deadline is not valid"; a non-date gives 400 "deadline must be
a valid ISO 8601 date string". Firm quotes return `deadline` and `timeWhenInactive`
equal to the request deadline plus 72 hours.

**What the website should show.** Show `minAmountOut` as the guaranteed minimum, and
`amountOut` as the expected amount. With 100 bp slippage, the measured 10-minute moves
(0.3 %) stay well inside the margin, so a minimum taken from a dry quote stays
reachable for a few minutes. The app gets a fresh firm quote at exit time anyway
(DESIGN.md §10), and step 4 refuses to sign if the firm minimum is below the approved
one.

## 6. Errors and what the app does

| Situation | How it shows | What the app does |
|---|---|---|
| Quote expired before signing | `Refusal::QuoteExpired`, `Refusal::QuoteTooOld` (signed `timestamp` older than 5 minutes), or HTTP 400 `deadline is not valid` (`ApiErrorKind::DeadlineInvalid`) | Get a new firm quote. If its minimum is not below the approved one, continue; otherwise ask the user again. |
| Quote from the future | `Refusal::QuoteFromFuture` (signed `timestamp` more than 1 minute after the local clock) | The local clock is behind. Show that to the user. Nothing was signed. |
| Payload nonce not accepted | `Refusal::BadNonce` (not a versioned nonce, or its deadline is not the intent deadline) | Do not sign. Get a new firm quote and payload. |
| Price moved below the approved minimum | `Refusal::MinimumTooLow` at step 4 | Show the new price and ask the user again. Nothing was signed. |
| Price moves below `minAmountOut` after submission | `/v0/status` `REFUNDED` (documented, not measured) | The USDC is back under the taker's key in the Verifier. Check `mt_balance_of`, then start again from step 3. |
| No solver quote | HTTP 400 `Quoting for this pair is not available` (`NoQuote`) | Retry after 20 s. Measured: it cleared within 20 s. |
| Amount too small for the destination bridge | HTTP 400 `Amount is too low for bridge, try at least N` (`AmountTooLow { min }`) | Tell the user the minimum. Offer another destination. |
| Invalid recipient address | HTTP 400 `recipient is not valid` | Ask for the address again. The burn address `0x…dEaD` is refused. |
| Refusal by the safety check | `Error::Refused(..)` | Do not retry with the same answer. Log it; get a new quote. Repeated refusals mean the API or the connection is not trustworthy: use the direct path (section 7). |
| Submit failed with no HTTP answer or 5xx | `Error::Transport`, `ApiErrorKind::Server` | Send the same signed payload again. It has the same hash and nonce, so it can execute at most once. |
| Submit answered 4xx | `ApiErrorKind::QuoteNotPending` (`Quote is in FAILED status, expected PENDING_DEPOSIT or INCOMPLETE_DEPOSIT`) or `BadSignedIntent` (`Cannot parse signed intent`) | New firm quote, new signature. |
| Status `FAILED` | `/v0/status` | Check `mt_balance_of` for the taker. If the USDC is still there, start again from step 3. Measured: with no funds, the status was `FAILED` with `updatedAt` 7 s after the recorded flow started (dry quote, firm quote, generate, submit). |
| Status stuck in `PENDING_DEPOSIT` or `PROCESSING` | no final status after `timeEstimate` × 10 | Keep polling at a slower rate. After the quote's request deadline, 1Click refunds by its documentation. The signed transfer can execute until the intent deadline (request deadline + 72 h). Check `mt_balance_of`; if the USDC is still there after the intent deadline, start again. |
| Unknown deposit address | HTTP 404 `Deposit address … not found` (`NotFound`) | The address is not from 1Click, or the record is gone. Check `mt_balance_of`. |

Notes:

- `/v0/submit-intent` accepted a payload with a zero signature (201 and the same
  `intentHash`). Acceptance does not mean the intent is valid. Only `/v0/status` and
  the Verifier balance tell what happened.
- A signed exit stays valid until its deadline (about 72 hours). If the app signs a new
  exit for a new quote while an older one is not final, both are valid, but the
  balance covers only one. If the first one is refunded, the second one can still
  execute later, at its own approved minimum. The app keeps polling every deposit
  address it signed for until each is final or its deadline has passed.
- `/v0/status` showed `depositedAmount: "5000000"` for a key that held nothing. Do not
  read that field as proof that funds moved.

## 7. The direct path, without 1Click

`direct::IntentDraft` builds and signs Verifier intents without 1Click. Anyone can
submit them with `execute_intents` on `intents.near` (the taker's own stipend account
can, or the maker's backend). Uses:

- the fallback when 1Click is down or refused: `ft_withdraw` the USDC to a NEAR account
  the user names, or `transfer` it to another Verifier account;
- local tests against the Verifier code.

It cannot swap to another chain alone; a swap needs a solver.

Rules (from `contracts/defuse/core/src/engine/mod.rs` and `nonce/` in `near/intents`,
commit `fce0936`, and checked in the sandbox):

- The deadline must be in the future when the transaction runs.
- A nonce works once for each signer ("nonce was already used").
- A versioned nonce: magic `5628f6c6`, variant byte `0`, the salt (`current_salt`), the
  nonce deadline as `i64` nanoseconds little-endian, 15 random bytes. The salt must be
  valid ("invalid salt"); the intent deadline must not be after the nonce deadline; the
  nonce deadline must not have passed. In the direct path the caller passes the nonce
  (`Nonce::versioned_random(salt, deadline)`), and `IntentDraft` refuses a versioned
  nonce whose deadline is before the intent deadline. 1Click sets the nonce deadline
  equal to the intent deadline.
- A legacy nonce (any other 32 bytes) still works; the source says legacy nonces will
  be prohibited.
- `ft_withdraw` takes the token contract id with no prefix (`usdc.near`); `transfer`
  takes it with the prefix (`nep141:usdc.near`). `ft_withdraw` without `msg` uses
  `ft_transfer` and is refunded if it fails; the crate does not offer `msg`.

Gas measured in the sandbox for `execute_intents`: one `ft_withdraw` 10.4 Tgas; one
`transfer` 5.3 Tgas; a `transfer` and an `ft_withdraw` by two signers in one call 12.5
Tgas; a refused call about 3.6 Tgas.

## 8. Tests

| Test | What it shows | Command |
|---|---|---|
| Unit (24) | Builders, parsers, nonces, NEP-413 hash against the Verifier's test vector, raw intent hash against the Verifier's test vector, error classes | `cargo test --lib` |
| Recorded (21) | The full exit replayed against live answers for `raw_ed25519` and `nep413`, byte-identical requests, all refusals on real payloads, quote freshness from the signed timestamp, the review-1 F-02 proof of concept, legacy or long-lived nonce | `cargo test --test recorded` |
| Sandbox (2) | With the live `intents.near` code: an unfunded key's `ft_withdraw` and `transfer` signed by the crate and submitted by a third account, balances checked; replay, wrong salt, wrong key and too large an amount refused; legacy nonce; two signers in one call | `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox` |
| Wasm | The crate built for `wasm32-unknown-unknown` runs: quote signature, check, signature equal to the recorded one, a refusal, browser random source | `crates/zecret-intents/wasm-check/run.sh` (Node 22, not a browser) |

## 9. Not verified

These need real funds on mainnet and the owner's approval:

- That 1Click executes a keyless signed exit. Submission is accepted without a key, but
  the only executions seen failed because the key held nothing.
- The status sequence of a funded exit (`KNOWN_DEPOSIT_TX`, `PROCESSING`, `SUCCESS`)
  and the real time from submit to delivery against `timeEstimate`.
- The refund path: that a swap that cannot meet `minAmountOut` is refunded into the
  taker's Verifier account, and when.
- The fees really charged without a key (the measured 1 or 20 bp `appFees`, or the
  documented 25 bp).
- Whether 1Click rejects a bad signature before it reaches the Verifier (it returned
  201 for one).
- `ft_withdraw` with memo `WITHDRAW_TO:<address>` for bridge tokens; the crate can build
  it but it was not tested.
- The crate in a browser Web Worker. It ran in Node 22 through wasm-bindgen, not in a
  browser.
