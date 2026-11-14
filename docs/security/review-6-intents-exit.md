# Security review 6: the exit through NEAR Intents 1Click

Date: 2026-11-14. Repository state: `main` at `8ebbc8a`, with the changes listed under
"Fixes". Nothing was run with funds. No transaction was sent on mainnet. The only
writes to a third party were two firm 1Click quotes (a deposit address each; nothing
was deposited and `/v0/submit-intent` was not called).

## Summary

The exit code was checked against the 1Click documentation, the OpenAPI spec, the
official TypeScript SDK and the Verifier source, and against live read-only answers of
1Click and of `intents.near` on mainnet.

- **The signed payload is right.** Every payload the crate can sign (`transfer` and
  `ft_withdraw`, `raw_ed25519` and `nep413`), and the exact payload of a live 1Click
  exit for a fresh key, passes every check of the mainnet Verifier except the balance:
  `simulate_intents` on `intents.near` answered "insufficient balance or overflow". A
  wrong salt and a wrong signature give "invalid salt" and "invalid signature", so the
  simulation tells these errors apart. This is now an opt-in test and a preflight check.
- **The request fields are right** for an origin that is already inside NEAR Intents
  (`depositType: INTENTS`, `refundType: INTENTS`, `recipientType: DESTINATION_CHAIN`,
  `swapType: EXACT_INPUT`), and the quote signature algorithm and key match SDK 0.1.26,
  the latest release.
- **The pre-signing check holds** against a hostile 1Click that holds the quote key:
  eleven new adversarial tests sign changed quotes with a test key that the policy pins,
  so they reach the field checks behind the signature.
- **The engine trusted 1Click's unsigned status too much** (R6-01 to R6-03). It now
  reads the taker key's balance in NEAR Intents before each attempt, after each final
  status and before it believes a delivery, and a swap restored after its exit watches
  for a refund. These were the important gaps: a refund that came back late or short, or
  a false SUCCESS, could leave USDC in NEAR Intents while the page said otherwise.
- **Recovery without 1Click** now exists as a command-line rescue that moves the USDC out
  of NEAR Intents with only the recovery file (the swap key's own NEAR account pays the
  gas from the rest of the stipend). The site has no button for it yet (gap X-1).

What only a funded run can prove is listed at the end.

| ID | Severity | Finding | Status |
|---|---|---|---|
| R6-01 | Medium | After REFUNDED or FAILED the engine said "the USDC is back under your key" without reading the balance, and the next attempt always signed the full payout | Fixed |
| R6-02 | Medium | A swap restored after its exit ended in `done` for good; a later 1Click refund into the key was never noticed | Fixed |
| R6-03 | Low | `/v0/status` answers were not checked; a false SUCCESS ended the swap as delivered | Fixed |
| R6-04 | Low | The deposit address was not checked as a Verifier account | Fixed |
| R6-05 | Low | `exit({ acceptMinimum })` accepted any value below the approved minimum | Fixed |
| R6-06 | Info | No path to move the USDC out without 1Click | CLI added; site button is gap X-1 |
| R6-07 | Info | Fees without an API key: the docs say +25 bp, the quotes show 20 bp (1 bp for stablecoins) | Documented; unverifiable without funds |
| R6-08 | Info | `generate-intent` and `submit-intent` are documented as partner-authenticated; they answer without a key | Documented; fails closed if enforced |
| R6-09 | Info | The quote key is pinned in the code; a rotation refuses every exit | Preflight check added |
| R6-10 | Info | USDC to XRP answers 400 "Internal server error" at every amount (2026-11-14) | Documented |
| R6-11 | Low | The minimum from the price preview is scaled linearly; with a withdraw fee and a maker paying exactly its minimum, the firm minimum can fall below it and the user is asked again | Not fixed (no loss) |
| R6-12 | Info | The price preview does not check the signature of its dry quotes | Not fixed (display only) |

## Scope

Code read in full: `crates/zecret-intents/src/*` (oneclick, payload, signer, nonce,
quote_sig, check, direct, client, http, error, time), its tests and fixtures;
`crates/zecret-wasm/src/exit.rs`, `session.rs`, `web.rs` (exit bindings);
`web/src/engine/real/realEngine.ts` (the exit, `exit_retry`, restore),
`payoutCheck.ts`, `web/src/lib/oneclick.ts`, `price.ts`, `netconfig.ts` (mainnet
checks), `web/src/app/useExit.ts`, `usePrice.ts`, `stageCopy.ts` (exit copy),
`web/public/config/mainnet.json`, `web/vercel.json`, `web/ENGINE_API.md`;
`docs/INTENTS.md`, `docs/DESIGN.md` (plain words), `docs/MAINNET.md` (gap list),
`docs/security/review-1.md` and `review-3.md` for what they covered.

Earlier reviews: review 1 covered the quote signature reconstruction, every rule of
`check_quote` and `check_generated_intent`, strict decoding, hashing and nonces (F-02:
freshness from the signed timestamp). Review 3 covered the engine's exit steps (H3 links,
H4 lost submit answers, H5 direct exit, H9 wording). Neither checked the engine's
handling of final statuses against the balance, the restore path for 1Click, the live
Verifier with the crate's payloads, or a recovery without 1Click.

## Sources (all read on 2026-11-14)

| Source | What was used |
|---|---|
| `https://docs.near-intents.org/llms.txt` | Index of the docs |
| `.../integration/distribution-channels/1click-api/quickstart/signed-intent-execution.md` | `depositType: INTENTS` uses `generate-intent` and `submit-intent`; "the only required change is setting `depositType` (and matching `refundType` / `recipientType` …)"; both endpoints "use partner authentication" |
| `.../1click-api/verify-quote-signature.md` | Signed message `stringify({...quoteRequest, ...quoteResponse, timestamp})`, base58 SHA-256; use `verifyQuoteSignature()` of the SDK (0.1.24 or later). The page does not print the key |
| `.../1click-api/authentication.md`, `.../resources/fees.md` | Without a key "additional 0.25 % (25 bps) on every non-ANY_INPUT quote"; with a key 20 bp, 1 bp for stablecoins; protocol fee 1 pip |
| `.../resources/swap-limits.md` | USD minimums apply to `ORIGIN_CHAIN` deposits only, not to Intents; "Amount is too low for bridge, try at least" is the bridge minimum |
| `.../1click-api/quickstart/making-a-request.md` | Status values and their meaning |
| `.../verifier-contract/simulating-intents.md`, `.../deposits.md`, `.../withdrawals.md` | `simulate_intents` takes the `execute_intents` input; deposit `msg` names the owner; `ft_withdraw` unprefixed token, 100 Tgas for a direct call |
| `.../market-makers/message-bus/rpc.md` | `publish_intent` on `solver-relay-v2.chaindefuser.com` needs an API key and quote hashes: not a general gasless relay |
| `.../changelog/overview.md` | 2026-10-27 swaps settle confidentially (no swap hash in `nearTxHashes`); 2026-11-09 new token ids for BNB, Polygon, Avalanche |
| `https://1click.chaindefuser.com/docs/v0/openapi.yaml` | `info.version` 0.1.10. Schemas `QuoteRequest` (new optional fields `insured`, `rebates`, `sessionId`, `connectedWallets`), `Quote`, `QuoteResponse`, `GetExecutionStatusResponse` (`quoteResponse` required), `SwapDetails`, `GenerateSwapTransferIntentRequest`, `SubmitSwapTransferIntentRequest`; security `X-API-Key` or `JWT-auth` |
| npm `@defuse-protocol/one-click-sdk-typescript` 0.1.26 (latest; published 2026-09-22), `dist/index.mjs` | `ONE_CLICK_MANAGER_PUB_KEY = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc"`; `buildSignedQuoteRequest` / `buildSignedQuote` field lists; `DepositTypeEnum` etc. |
| `github.com/near/intents` at `dbad9c8` (2026-11-13) | `contracts/defuse` unchanged since `fce0936` (the commit the crate cites). `core/src/engine/mod.rs`: order of checks; `core/src/error.rs`: messages |
| NEAR mainnet RPC `rpc.mainnet.fastnear.com` | `intents.near` code hash `BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk` (the same as `.tools/wasm/intents.near.wasm`, which the sandbox tests run); `current_salt` `252812b3` at block 224,519,518; `is_valid_salt("252812b3")` true |

## Checks

### 1. The 1Click API

| Item | Code | Verdict | Evidence |
|---|---|---|---|
| `GET /v0/tokens` | `oneclick.rs` `TokenInfo`; `web/src/lib/oneclick.ts` `selectDestinations` | CONFIRMED | 231 tokens; USDC on NEAR `nep141:1720…33a1`, 6 decimals. New field `coingeckoId` is ignored by both parsers. Unauthenticated limit 1200 per 60 s |
| `swapType` | `QuoteRequest::exit` | CONFIRMED | `EXACT_INPUT`: "if deposit is less than amountIn, the deposit is refunded by deadline; if above, the excess is refunded". No partial fills |
| `slippageTolerance` | 100 (basis points) | CONFIRMED | `minAmountOut / amountOut` = 0.99 on every route quoted |
| `originAsset` / `destinationAsset` | asset ids from `/v0/tokens` | CONFIRMED | dry quotes accepted for Base USDC, Arbitrum ETH, SOL, BTC, Stellar, TON |
| `depositType` / `refundType` for an origin inside Intents | `INTENTS` / `INTENTS` | CONFIRMED | Docs (signed intent execution) and OpenAPI enum (`ORIGIN_CHAIN`, `INTENTS`, `CONFIDENTIAL_INTENTS`) |
| `recipientType` for another chain | `DESTINATION_CHAIN` | CONFIRMED | OpenAPI: "assets are transferred to the chain of destinationAsset" |
| `deadline` format | `time::format`, `2026-11-14T13:55:36.000Z` | CONFIRMED | Accepted; firm quote `deadline` and `timeWhenInactive` = request deadline + 72 h |
| `quoteWaitingTimeMs` | not sent | CONFIRMED | Default 0 ms (fastest quote) |
| `dry` | page: `true`; engine: `false` | CONFIRMED | Dry has no `depositAddress`/`deadline` |
| `referral`, `appFees` | not sent | CONFIRMED | Without a key, 1Click adds its own `appFees` entry (`5880ad2b…47dd`) |
| JWT needed? | none sent | DOCS SAY YES, LIVE SAYS NO (R6-08) | `generate-intent` answered 201 without a key today; `submit-intent` answered 201 on 2026-11-13 (not called today) |
| Fee without a key | shown from `appFees` and from USD values | UNVERIFIABLE WITHOUT FUNDS (R6-07) | Quotes: 1 bp (stablecoin) or 20 bp in `appFees`; the cost from `amountInUsd`/`amountOutUsd` equals that plus `withdrawFee` (USDC→Base 2 USDC: 13.3 bp = 1 bp + 2400 withdraw fee). The docs' 25 bp is not visible in any quote |
| Response amounts | `Quote::amount_in/out/min_amount_out` strict integers | CONFIRMED | |
| `timeEstimate` | `f64` seconds | CONFIRMED | 12 s Base, 9 s SOL, 11 s Arbitrum ETH, 27 s Stellar, 67 s TON, 455 s BTC |
| Quote signature | `quote_sig.rs` | CONFIRMED | Algorithm and field lists equal `verifyQuoteSignature` in SDK 0.1.26 (latest). All live quotes of today (dry and firm) verify. New echo fields (`insured`, `confidentiality`, `depositMode`, `appFees[].limitOrderId`) are not in the signed set, so they do not break it |
| Key published where | `ONE_CLICK_QUOTE_KEY` | CONFIRMED / R6-09 | Only in the SDK source (`ONE_CLICK_MANAGER_PUB_KEY`); the docs page refers to the SDK. Pinned in the crate; a preflight check now compares it with the latest SDK |
| `/v0/deposit/submit` | not used | CONFIRMED | For `ORIGIN_CHAIN` deposits only; an `INTENTS` origin uses `submit-intent` |
| `/v0/generate-intent`, `/v0/submit-intent` | `type: swap_transfer`, `standard`, `signerId`, `depositAddress`; `signedData` = MultiPayload | CONFIRMED | Live generate today for both standards |
| `/v0/status` states | `SwapStatus` | CONFIRMED | `PENDING_DEPOSIT`, `KNOWN_DEPOSIT_TX`, `PROCESSING`, `SUCCESS`, `INCOMPLETE_DEPOSIT`, `REFUNDED`, `FAILED`. An unfunded firm quote reads `PENDING_DEPOSIT` with a signed `quoteResponse`; an unknown address gives 404 |
| Refunds: where and when | `refundType: INTENTS`, `refundTo` = the key's account | DOCUMENTED; UNVERIFIABLE WITHOUT FUNDS | "REFUNDED: Swap failed, funds returned to refund address"; refund "begins" at the request deadline (30 min). Quotes show `refundFee: "0"` |
| Memo / tag chains | site refuses tags (`chains.ts`), check refuses `depositMemo` | CONFIRMED | The API has no destination memo field: routing data must be in the address. Deposit memos only exist for `ORIGIN_CHAIN` |

Live dry quotes, 2 USDC in, refund to a fresh key, 2026-11-14 13:22 UTC:

| Route | `amountOut` | `minAmountOut` | `appFees` | `withdrawFee` | All-in cost | Time |
|---|---|---|---|---|---|---|
| USDC on Base | 1.997343 | 1.977369 | 1 bp | 0.0024 USDC | 13 bp | 12 s |
| ETH on Arbitrum | 0.001053160721645426 | 0.001042629114428971 | 20 bp | 1.2e-6 ETH | 30 bp | 11 s |
| SOL | 0.026314772 | 0.026051624 | 20 bp | 132088 lamports | 65 bp | 9 s |
| USDC on Solana | 1.989623 | 1.969726 | 1 bp | 9974 | 52 bp | 7 s |
| XLM (Stellar) | 13.9410827 | 13.8016718 | 20 bp | 1 XLM | 101 bp | 27 s |
| BTC | 400 "Amount is too low for bridge, try at least 5609479" | | | | | |
| BTC, 20 USDC | 0.00028033 | 0.00027752 | 20 bp | 1900 sat | 645 bp | 455 s |
| XRP | 400 "Internal server error" (also at 5 and 20 USDC, two addresses) | | | | | |

Smallest `amount` each route accepts (from "try at least N", micro-USDC): USDC on Base
2429, USDC on Arbitrum 5359, ETH on Base 1077, ETH on Arbitrum 2304, SOL 78286, ZEC
1189762, XLM 14420, TON 44066, BTC 5609479; USDC on Solana answered "try at least 0"
for 0.001 USDC. Below about 1 USDC the withdraw fee dominates (0.1 USDC to Base costs
242 bp).

### 2. The signed intent

| Item | Verdict | Evidence |
|---|---|---|
| NEP-413 payload (`message`, `nonce`, `recipient`, optional `callbackUrl`; tag 2^31+413, borsh, SHA-256) | CONFIRMED | Crate test against the Verifier's own vector; mainnet simulation below |
| `raw_ed25519` (the key signs the UTF-8 payload string; hash = SHA-256) | CONFIRMED | Crate test against the Verifier's vector; mainnet simulation |
| `signer_id` = lowercase hex of the public key (implicit account); no `add_public_key` needed | CONFIRMED | Simulation passed the public key check for fresh keys |
| `verifying_contract` / `recipient` = `intents.near` | CONFIRMED | |
| `deadline` ISO 8601 string | CONFIRMED | |
| Nonce layout | CONFIRMED | Versioned: magic `5628f6c6`, variant `0`, salt (4 bytes from `current_salt`), deadline i64 ns LE, 15 random bytes. Salt today `252812b3`; the generated nonces carry it and a nonce deadline equal to the intent deadline. Legacy nonces still accepted by the source |
| The intent for a deposit into a 1Click deposit address | CONFIRMED | One `transfer`, `receiver_id` = `depositAddress`, `tokens: {"nep141:<usdc contract>": "<amount>"}` (prefixed). `ft_withdraw` takes the unprefixed contract id |
| Submission without gas | CONFIRMED (path) / UNVERIFIABLE (execution) | `POST /v0/submit-intent`; 1Click's relayer sends `execute_intents`. The solver relay's `publish_intent` needs an API key and quote hashes; not used |

**Format proof on mainnet** (`crates/zecret-intents/tests/live_mainnet.rs`, run
2026-11-14 13:25 UTC, `ZECRET_LIVE_MAINNET_READONLY=1`). Fresh key
`3a6bb2c6…bc7d` (direct payloads) and `b2a3b214…09c5` (1Click payload); `query`
`call_function` `simulate_intents` at final finality:

| Payload | Answer |
|---|---|
| `transfer` 1 USDC, `raw_ed25519` | `wasm execution failed with error: HostError(GuestPanic { panic_msg: "insufficient balance or overflow" })` |
| `ft_withdraw` 1 USDC, `raw_ed25519` | same |
| `transfer`, `nep413` | same |
| `ft_withdraw`, `nep413` | same |
| control: wrong salt `00000000` (both standards) | `… panic_msg: "invalid salt"` |
| control: another key's signature (both standards) | `… panic_msg: "invalid signature"` |
| 1Click exit, `raw_ed25519` (firm quote, deposit `a9c3f6d5…6a61`, generate-intent, pre-signing check, signature) | `… "insufficient balance or overflow"` |
| 1Click exit, `nep413` (same quote) | same |

The Verifier checks, in this order: signature, payload decoding, verifying contract,
deadline, public key of the signer, nonce (salt, nonce deadline, not used), and only
then the intents (`engine/mod.rs`). So "insufficient balance" means every format check
passed. The signed 1Click payload (raw):

```json
{"standard":"raw_ed25519","payload":"{\"verifying_contract\":\"intents.near\",\"signer_id\":\"b2a3b214a8e648fba1fe650327cd579d0312e07e71a1bc3e4b5c1690641509c5\",\"deadline\":\"2026-11-17T13:55:36.000Z\",\"nonce\":\"Vij2xgAlKBKzAHAyw6DZ6BiauGV/VTXdOm7mTp/nBTY=\",\"intents\":[{\"intent\":\"transfer\",\"receiver_id\":\"a9c3f6d598b785fa5a3f0b03d37c13a4e96a9febc0eab610abc70f6b77656a61\",\"tokens\":{\"nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1\":\"2000000\"}}]}","public_key":"ed25519:D2LLvM9ToSkL4jPk9TuDMWdHDvXWjQUus5JEcDbFbCda","signature":"ed25519:64Tdw8REZPKbpbB9CL1cauudEf5u4C2GvGY3oq7tpAcFtpytLEcy7vr4KP1YgUNJ6ZmP3wiNdSr85MFbSnJqVRJK"}
```

1Click has no dry or validation mode for `submit-intent` (OpenAPI 0.1.10); it accepted
a zero signature on 2026-11-13, so its answer proves nothing. `simulate_intents` is the
validation.

### 3. The safety checks

| Property | Verdict | Where |
|---|---|---|
| Nothing is signed whose receiver, token or amount differs from the approval | CONFIRMED | `check_generated_intent`: one `transfer`, receiver = signed deposit address, one token = approved origin, amount = approved (canonical decimal), no memo, no `msg`; strict decoding |
| A hostile answer cannot move more than approved or elsewhere | CONFIRMED | Amount and receiver come from the signed quote and the approval; the signer accepts only a checked payload (`UnsignedPayload` is crate-private) |
| Deadlines sane | CONFIRMED | Intent deadline in the future, not after the quote's deposit deadline, at most 96 h; nonce versioned with deadline = intent deadline; quote timestamp at most 5 min old |
| Quote signature verified against the right key | CONFIRMED | `verify_strict` with the SDK key; dry and firm; also now the quote inside each status answer |
| Key rotation | R6-09 | Fails closed: every quote is refused (`EXIT_REFUSED`), statuses are not believed, the USDC stays under the key. Procedure in `docs/INTENTS.md` §3 |
| Replays | CONFIRMED | Old quotes: refused by the signed timestamp; signed intents: one nonce per signer (sandbox test) |
| Amount = full balance vs approved | FIXED (R6-01) | Each attempt now moves `min(payout, balance)`; never more than the payout |
| Dust | CONFIRMED | Tokens above the payout stay under the key; the rescue moves the whole balance |
| Deposit address is a Verifier account | FIXED (R6-04) | Valid NEAR account id, not the signer, not the Verifier. A lower-case EVM address is a valid Verifier account, so only the signature tells whether 1Click chose it |

Adversarial tests (`crates/zecret-intents/tests/adversarial.rs`, 11 tests; each quote is
changed and signed again with a test key the policy pins): another recipient,
destination asset, origin asset or recipient type; refund to another account or to the
origin chain; `customRecipientMsg`, `virtualChainRecipient`,
`virtualChainRefundRecipient`; a minimum one unit below the approval; another
`amountIn`, `amount` or `swapType`; deposit addresses that are not Verifier accounts
(Solana address, upper-case EVM address, the signer, `intents.near`, too short, leading
dot) and a missing one; a deposit memo; an expired deposit deadline, a 6-minute-old
timestamp, a dry quote; an empty signature; a payload to another receiver, for one unit
more, with a second intent, with `msg`, for another signer; a status answer for another
deposit address, with a changed quote, or without a quote. With 1Click's real key pinned,
every re-signed quote is refused at the signature.

### 4. Failure and recovery

| Case | Docs | Code before | Code now | Verdict |
|---|---|---|---|---|
| Quote expires before the signature | Refund begins at the request deadline | `exitSubmit` refuses a quote older than 5 min ("too old") and the engine quotes again | same | CONFIRMED |
| Transfer lands, 1Click never completes | Refund after the deadline | Polled every 5 s for ever | Every 5 s for 10 min, then every 30 s; the deposit address is in the state | CONFIRMED (behaviour); delivery UNVERIFIABLE |
| 1Click refunds into the key | REFUNDED, `refundTo` | Went to `exit_retry` saying "the USDC is back" without reading it (R6-01) | Reads the balance: tokens there → `exit_retry`; none yet → keeps watching with a message that names the deposit address | FIXED |
| FAILED | "Swap encountered an error" | Same as REFUNDED (R6-01) | Same balance rule. With no funds (2026-11-13) FAILED meant the transfer never ran; if it ran, the USDC is held by 1Click at the deposit address, and the page says so and keeps watching | FIXED |
| Refund shorter than the payout | `refundFee` (0 in every quote seen) | Next attempt signed the full payout and failed at the Verifier | Next attempt quotes what is there, with the minimum scaled at the approved rate (`exitPlan.ts`) | FIXED |
| Restore after the exit ran elsewhere | – | `payoutCheck.ts`: balance 0 → `done` for good (R6-02) | With deposit addresses in the file: follow the last. Without: `done` with `doneElsewhere`, and watch the balance once a minute for 96 h while the page is open; tokens back → `exit_retry` | FIXED |
| Partial fills | `EXACT_INPUT`: all or refund | – | – | CONFIRMED (docs); UNVERIFIABLE |
| Tab closed mid-exit | – | State and `signedDeposits` are stored before submit; the next attempt with no balance follows the stored deposit address instead of signing again | same, plus `signedAmounts` | CONFIRMED |
| Delivery below the minimum | – | `EXIT_BELOW_MINIMUM`, nothing signed, the user is asked | Also: accepting may not go below the offered minimum (R6-05) | CONFIRMED |
| False SUCCESS | Status is not signed | Ended the swap as delivered (R6-03) | Status must carry a signed quote for this deposit address, and the key must hold less than it signed | FIXED |

**What the user can do with only the recovery file.** The file holds the swap's NEAR key
(`records["swap/<id>"].near_sk`). Options:

1. Open the site and import the file: the engine runs the 1Click exit (needs 1Click).
2. Without 1Click: `scripts/mainnet-exit-test.sh rescue --recovery-file FILE
   --to-intents-account ACCOUNT` (a `transfer` to another NEAR Intents account, for
   example the user's own NEAR account id, which the user can then use on
   near-intents.org) or `--to-near-account ACCOUNT` (an `ft_withdraw` to a NEAR account
   registered with USDC; this uses the engine's own `direct_prepare` and
   `direct_submit_with_gas`). The tool signs, simulates on `intents.near`, and only with
   the flag sends `execute_intents` from the swap key's own NEAR account, which pays the
   gas from what is left of the stipend (40 Tgas for `ft_withdraw`, 20 Tgas for
   `transfer`; NEAR asks the sender to hold the attached gas at 10^9 yoctoNEAR per gas,
   so 0.04 or 0.02 NEAR; about 0.057 NEAR is left after the three taker calls at
   today's gas price, estimated, not measured on mainnet). `--gas-payer-key` lets any
   other account pay instead.
3. Without 1Click and without any gas: not possible. No public relay sends an arbitrary
   signed `ft_withdraw` or `transfer` for free; the solver relay's `publish_intent` needs
   an API key and quote hashes.

### 5. Direct exit versus 1Click on mainnet

| Item | Verdict | Evidence |
|---|---|---|
| Mainnet config can only be `oneclick` | CONFIRMED | `netconfig.ts`: "mainnet exits through 1Click"; e2e `mainnet-off`/`mainnet-on` |
| Token, Verifier, escrow anchored | CONFIRMED | `checkMainnet` compares with `MAINNET_ANCHORS` (USDC, `intents.near`) |
| Relay URL, stand-in Verifier, direct account | CONFIRMED not used | `createSwap` and the restore check skip `directExit` for `oneclick`; `exitStep` branches on the mode |
| Approval uses the configured token and Verifier | CONFIRMED | `approval()` uses `cfg.near.token`/`verifier`, anchored on mainnet |
| CSP covers every host the mainnet exit calls | CONFIRMED | 1Click (quote, generate, submit, status, tokens) and the three NEAR RPC hosts (payout check, balance reads) are in `connect-src`; `tests/unit/csp.test.ts` derives them from the configs. The explorer link is a link, not a fetch |

### 6. Fees and amounts shown

| Item | Verdict | Evidence |
|---|---|---|
| Fee model | UNVERIFIABLE WITHOUT FUNDS (R6-07) | Hackathon notes: 0.2 % without a key. Docs: +0.25 % without a key. Quotes: 0.20 %, 0.01 % for stablecoins, and amounts that match it |
| "You receive at least" | CONFIRMED | `price.ts` `combine`: `destMin = minAmountOut × usdcMin / amountIn` (both legs at their minimum); the engine signs nothing whose firm `minAmountOut` is below it |
| Slippage | CONFIRMED | 100 bp in both the preview and the firm quote; the margin for price moves between the preview and the exit is the maker's 1 % tolerance when the maker pays its expected amount |
| Linear scaling with a fixed withdraw fee | R6-11 | When the maker pays exactly `usdcMin`, `destMin` exceeds the firm minimum by about 1 % of the withdraw fee (about 19 sat on BTC at 20 USDC): the user is asked to accept, nothing is lost |
| Cost line | CONFIRMED | `exitCostBps` comes from 1Click's USD values and includes the withdraw fee |

## Fixes (files)

- `crates/zecret-intents/src/check.rs`: `Refusal::BadDepositAddress`, `valid_account_id`,
  `check_status` (the status answer must carry 1Click's signed quote for the deposit
  address). `src/lib.rs` exports it.
- `crates/zecret-intents/tests/adversarial.rs` (new, 11 tests),
  `tests/live_mainnet.rs` (new, opt-in, 2 tests).
- `crates/zecret-wasm/src/exit.rs`: `oneclick_status` checks the status answer and
  returns `refundReason` and `updatedAt`. `tests/oneclick_recorded.rs`: one test.
- `crates/zecret-wasm/examples/mainnet_exit.rs` (new) and `scripts/mainnet-exit-test.sh`
  (new): the owner's funded exit test and the rescue. `Cargo.toml`: `reqwest`
  dev-dependency for it.
- `web/src/engine/real/exitPlan.ts` (new): the balance decisions. `realEngine.ts`: the
  amount of each attempt, the balance check after a final status and before a SUCCESS,
  `signedAmounts`, the restore path, the refund watch in `done`, the accepted-minimum
  rule. `web/src/engine/api.ts`: `exit.minimumFor`, `offered.amountIn`.
  `web/src/app/stageCopy.ts`: the waiting message. `web/tests/unit/exitRefund.test.ts`
  (new, 15 tests).
- `scripts/mainnet-preflight.sh`: the SDK key check, the read-only `simulate_intents`
  check, and the exit tool's plan when `EXIT_OWNER_KEY` is set.
- Docs: this file, `docs/INTENTS.md`, `docs/MAINNET.md`, `docs/DESIGN.md` (plain words
  and decisions), `web/ENGINE_API.md`.

## Test results (2026-11-14)

- `cargo test` in `crates/zecret-intents` with the sandbox: unit 24, adversarial 11,
  recorded 21, sandbox 2, live 2 (skipped without the variable): all pass. With
  `ZECRET_LIVE_MAINNET_READONLY=1`: live 2 pass (output above).
- `cargo clippy --all-targets -- -D warnings`: zecret-intents and zecret-wasm clean.
- `cargo test -p zecret-wasm`: unit 3, recorded 4, sandbox 3 (pass with
  `--test-threads 1`; in parallel two sandboxes failed while creating their accounts,
  which is the test setup, not the exit code).
- Browser engine rebuilt with `crates/zecret-wasm/build.sh`.
- `web`: typecheck, lint, unit tests (13 files, 138 tests, 15 of them new), build, Playwright 96 passed,
  mainnet-switched Playwright 6 passed.
- `scripts/mainnet-preflight.sh` (`SKIP_ENGINE_BUILD=1` after the rebuild above, stand-in
  owner): 28 passed, 0 failed, `PREFLIGHT PASSED`. It includes the two new exit checks.
  The live test also checks that a live `/v0/status` answer carries the signed quote of its
  deposit address (`PENDING_DEPOSIT`, verified).
- `scripts/mainnet-exit-test.sh plan` with a stand-in owner (`aurora`, read-only):
  `PLAN OK`. `run` and `refund` without the flag are refused.

## What only a funded run can prove

1. That 1Click executes a keyless signed exit (`submit-intent` without a partner key) and
   delivers on each route; the real status sequence and time.
2. The fee really charged without a key (20 bp as quoted, or 25 bp more as documented).
3. A refund: that a failed swap is refunded into the key's NEAR Intents account, when,
   for how much (`refundFee`), and with which status sequence.
4. That a FAILED after the transfer ran leads to a refund, or what 1Click support needs.
5. The rescue on mainnet: `ft_withdraw` and `transfer` with the gas the tool attaches,
   paid from the real rest of the stipend.
6. That the engine's balance reads at final blocks see a refund within the waits it uses
   (30 s after a final status, 60 s in the refund watch).

## Gaps left

- **X-1.** The site has no rescue button. Specification: in `exit_retry` (and in `done`
  with `doneElsewhere`), a second action "Move the USDC without NEAR Intents 1Click"
  that asks for a NEAR Intents account or a NEAR account registered with USDC, signs one
  `transfer` (20 Tgas) or `ft_withdraw` (40 Tgas) for the whole balance with the swap
  key, simulates it on `intents.near`, and sends `execute_intents` from the swap key's
  own NEAR account; it needs one new WebAssembly binding over `exit::direct_prepare` and
  `direct_submit_with_gas` (or the `transfer` equivalent) and the engine method
  `rescue({ to })`. Until then the command-line rescue is the path.
- **X-2.** A partner key, if 1Click starts to require it for `submit-intent`, must live in
  a backend that forwards the two calls; a key in the browser is public.
  **Narrowed 2026-11-14:** the website project has that backend, a same-origin Vercel
  function per 1Click call (`web/api/oneclick/v0/`, `docs/ONECLICK_PROXY.md`). It adds
  the key from `ONECLICK_JWT` and forwards with no key without it. The engine treats it as
  untrusted transport; `crates/zecret-wasm/tests/oneclick_proxy.rs` shows that edited
  quotes, payloads and statuses are refused before anything is signed. Open: only its GET
  calls ran on Vercel (a preview), no key is set, and the configurations still call 1Click
  directly.
- **X-3.** A signed exit stays valid for about 72 hours. The engine never signs more than
  the key holds, so two signed exits cannot both run, but an earlier signed payload can
  still run later and move the USDC to its deposit address. The Verifier has no gasless
  way to cancel a nonce.
