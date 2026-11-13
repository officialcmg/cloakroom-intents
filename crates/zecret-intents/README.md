# zecret-intents

The exit through NEAR Intents. After a swap, the taker's USDC is inside the Verifier
contract `intents.near`, owned by the taker's fresh ed25519 key. This crate moves it to
the token and chain the user selected, with one signature and no NEAR gas.

The crate does no I/O and never reads the clock. Each function builds an
`HttpRequest` or parses an `HttpResponse`; the caller sends them (a synchronous
`XMLHttpRequest` in a Web Worker, a blocking client natively). "Now" is an argument.
It builds natively and for `wasm32-unknown-unknown`.

The exit flow, the safety check, what needs an API key, measured quote behaviour and
error handling are in [`docs/INTENTS.md`](../../docs/INTENTS.md).

## Modules

| Module | What it does |
|---|---|
| `oneclick` | Request builders and parsers for the 1Click API: `GET /v0/tokens`, `POST /v0/quote` (dry and firm), `POST /v0/generate-intent`, `POST /v0/submit-intent`, `GET /v0/status`. |
| `quote_sig` | Checks 1Click's ed25519 signature over a quote, with the algorithm and key of the official TypeScript SDK 0.1.26. |
| `check` | The pre-signing check. Refuses to sign unless the quote is signed by 1Click, is recent by its signed `timestamp`, and matches what the user approved, and the generated payload is exactly one `transfer` of the approved amount of the approved token to the quote's deposit address, from the taker's account, on `intents.near`, with a deadline in range and a versioned nonce that expires with the intent. |
| `signer` | The taker's ed25519 key: account id (hex of the public key), `raw_ed25519` and `nep413` signatures. It can sign only payloads this crate built or checked. |
| `payload` | Intents (`transfer`, `ft_withdraw`), the Verifier payload for both standards, NEP-413 hashing, intent hashes, the signed `MultiPayload`. |
| `nonce` | Versioned (salted, expiring) and legacy Verifier nonces. |
| `direct` | Intents built and signed without 1Click, `execute_intents` arguments, and NEAR RPC view calls (`current_salt`, `mt_balance_of`, `is_nonce_used`, `simulate_intents`). |
| `client` | `ExitClient` and `VerifierRpc`: the same calls chained over a caller's `HttpTransport`. |
| `error` | Errors; 1Click HTTP errors are classified (`ApiErrorKind`) from recorded answers. |

## Use

```rust
use zecret_intents::*;

let signer = IntentSigner::from_seed(&seed);              // the taker's key
let approved = ApprovedExit::mainnet(
    signer.account_id(), oneclick::USDC_NEAR_ASSET, 5_000_000,
    "nep141:sol.omft.near", "<solana address>", min_out_the_user_saw,
);
let client = ExitClient::new(my_transport, OneClick::default());
let exit = client.execute_exit(&signer, &approved, 100, now + 30min, Standard::RawEd25519, now)?;
// then poll client.status(exit.deposit_address()) until status.is_final()
```

### Quote freshness

1Click signs a firm quote's `timestamp` (when the quote was made) and `quote.deadline`
(the request deadline plus 72 hours). It does not sign `quoteRequest.deadline` on a
firm quote: the signed message takes `deadline` from `quote.deadline`. So the check
takes freshness from the signed `timestamp` (review-1 F-02):

- the quote is refused (`Refusal::QuoteTooOld`) when `timestamp + Policy::max_quote_age
  <= now`. `max_quote_age` defaults to 5 minutes;
- the quote is refused (`Refusal::QuoteFromFuture`) when `timestamp` is more than
  `MAX_QUOTE_CLOCK_AHEAD` (1 minute) after `now`;
- `quoteRequest.deadline` and `quote.deadline` must also be after `now`
  (`Refusal::QuoteExpired`). The first is an extra check only, because it is not
  signed.

This also refuses a replayed old quote. "Now" is an argument: the caller passes its
clock, and the tests pass the time of the recording. The caller must get the firm
quote and run the check within `max_quote_age`; `execute_exit` does both in one call.

### Nonce

1Click chooses the nonce of the payload it generates. The check accepts only a
versioned nonce whose deadline equals the intent deadline (`Refusal::BadNonce`), as
1Click returned in every recording. The salt is not compared with the Verifier's
`current_salt`, because the crate does no I/O; the Verifier refuses an invalid salt.

`HttpTransport` has one method, `send(&HttpRequest) -> Result<HttpResponse, String>`.
It is the contract of `zecret_core::net::Transport::post` plus GET, which
`/v0/tokens` and `/v0/status` need. An adapter over the core's transport needs a GET.

## Tests

```sh
cd /home/user/zecret && source .tools/env.sh
cd crates/zecret-intents
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test     # unit, recorded, sandbox
cargo clippy --all-targets -- -D warnings
cargo build --lib --target wasm32-unknown-unknown
wasm-check/run.sh                                 # runs the wasm build in Node
```

- **Unit tests** (`src/`): builders, parsers, nonces, NEP-413 hashing against the
  Verifier's own test vector, the raw intent hash against the Verifier's test vector,
  error classification.
- **Recorded tests** (`tests/recorded.rs`): the whole exit replayed against answers
  recorded from the live API on 2026-11-13 (`tests/fixtures/`), for `raw_ed25519` and
  `nep413`. Each request must equal the recorded one byte for byte (after JSON
  parsing), so the signature our code makes is the one 1Click accepted. Every refusal
  of the safety check is tested on real payloads: wrong receiver, amount, token,
  signer, contract, standard, intent type, intent count, memo, contract call,
  deadlines, a quote that differs from the approval, a tampered or unsigned quote, a
  quote older than `max_quote_age` or from the future, the review-1 F-02 proof of
  concept (an edited, unsigned request deadline), and a legacy or long-lived nonce.
- **Sandbox tests** (`tests/sandbox.rs`): the Verifier code fetched from
  `intents.near`, in nearcore sandbox 2.14.1. A token is deposited for an unfunded
  key; the crate signs an `ft_withdraw` (`raw_ed25519`) and a `transfer` (`nep413`); a
  third account submits them with `execute_intents`; balances are checked. Also: a
  replayed payload, a wrong salt, a payload signed by another key and an amount above
  the balance are refused, a legacy nonce works, and two signers' payloads run in one
  call.
- **Wasm check** (`wasm-check/`): a small cdylib bound with wasm-bindgen 0.2.129 and
  run in Node 22 (not WASI): quote signature, check, signature, a refusal, and the
  browser random source. It has not been run in a browser.

`examples/record_live.rs` records new fixtures from the live API with an unfunded
key (nothing is spent). `measurements/` holds the quote drift script, its data and
an analysis script.

## Sources

- Verifier: `github.com/near/intents`, commit `fce0936` (2026-11-10),
  `contracts/defuse/core/src/{payload,nonce,engine,intents}`,
  `crates/signatures/nep413`.
- 1Click: `https://1click.chaindefuser.com/docs/v0/openapi.yaml` (0.1.10) and
  `docs.near-intents.org` pages on signed intent execution, signing, withdrawals and
  fees, read on 2026-11-13.
- Quote signatures: `@defuse-protocol/one-click-sdk-typescript` 0.1.26,
  `dist/index.mjs` (`verifyQuoteSignature`).
