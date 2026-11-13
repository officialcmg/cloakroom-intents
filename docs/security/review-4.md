# Security review 4

Date: 2026-11-13. Source: `ac9c2ff79ae4a400c00312c637d6004c663ae78f`, branch `codex/review`. This is a scoped second opinion after the first three reviews and their fixes. No source was changed.

## Result

**No new confirmed security finding** was identified in the requested scope. This means no additional attack with a demonstrated source path survived review and comparison with the existing reports. It does not establish that the protocol is safe under arbitrary chain, service, timing or runtime failures.

There are therefore no new severity assignments, attack reproductions or failing-test code blocks in this report. A speculative attack is not presented as a failing test. Previously reported findings, accepted risks and hardening notes are not counted again.

## Scope, prior material and method

Reviewed:

- `contracts/near-escrow/src/`, including the curve checks and existing tests.
- `crates/zecret-core/src/taker.rs`.
- `crates/zecret-core/src/maker.rs`.
- `crates/zecret-node/src/relay.rs`.

The existing security material was read before the scoped review: `review-1.md`, `review-2.md`, `review-3.md`; `findings-1.json`, `findings-2.json`, `findings-3.json`; `fixes-1.md`, `fixes-2.md`, `fixes-2-core.md`, `fixes-2-taker.md`, `fixes-3.md`; `nv02-usdc.md`; both files in `poc/`, all three files in `poc2/`, and both files in `poc3/`. This includes rejected candidates, residual risks and untested changes, not just the headline findings. No separate prior coverage-ledger file exists in that directory.

Method: read-only source inspection, state-transition tracing, inspection of existing tests and comparison with prior findings. Reads used `rg`, `sed`, `nl` and file listings. References below are lines at the source commit, not historical line numbers copied from old reports. The relay received a second source pass for replay, request framing and worker accounting.

**Execution:** no security test, exploit, fuzzing, sandbox chain or attack request was run in this pass. The available environment did not provide the complete isolated runtime needed for fault injection, and the earlier screening's Cargo attempts stopped at dependency/toolchain gates. The installed Rust 1.94.0 is below several locked dependencies' Rust 1.95 minimum; the documented chain setup also needs Linux x86-64. Historical green results were read, not reproduced. The earlier screening's read-only public RPC checks do not validate adversarial behavior.

## Escrow coverage — no new finding

| Attack area | Current source checked | Result and limit |
|---|---|---|
| One party takes both assets through two escrow exits | `contracts/near-escrow/src/lib.rs:619–671` | Claim/refund terminal transitions exclude a second exit. Locked-state claim and refund have disjoint time conditions. Fixed recipients prevent a copied reveal from redirecting payment. Cross-chain safety remains subject to the previously documented participation and chain-observation assumptions. |
| Funds locked with no exit | `lib.rs:619–671`, `:688–808`, `:825–887` | Traced Open, Ready and Locked exits, credit creation, zero-before-call withdrawals, private callbacks and sweep. No additional source-level state with no protocol exit was found. Actual callback gas, upstream token behavior and cross-shard receipt ordering are **not checked** in execution. |
| Third-party theft or swap squatting | `lib.rs:378–403`, `:451–543` | Storage ownership/token restrictions and prepaid swap entries match the earlier squatting fix. Terminal reveals update existing records. |
| Unauthorized Ready/commit and replay | `lib.rs:311–341`, `:570–610` | Ready authenticates the maker. Commitment authenticates the taker, binds the signature domain, escrow and swap, and only progresses the allowed state. Public copies cannot choose a different payee. |
| Deadline and arithmetic errors | `lib.rs:625`, `:656` | Lock time plus reveal window is computed in `u128`. Checked the transition boundaries; no additional overflow route found. |
| Invalid or substituted key share | `contracts/near-escrow/src/curve.rs:42–65` | Checks canonical nonidentity points, canonical nonzero scalars and agreement between the disclosed scalar and stored point. |
| Duplicate withdrawal or sweep theft | `lib.rs:688–808`, `:825–887` | Credits are cleared before external calls; restoration is in private callbacks. Sweep sends to the escrow and excludes current in-flight funds without minting credit. Existing external-contract failure residuals are not new findings. |

The transition, replay, withdrawal retry/concurrency and sweep tests at `contracts/near-escrow/src/tests.rs:1088–1571` were inspected. They were **not run**. Deployed immutability was observed in the screening, but a fresh build-to-deployment comparison is **not checked**.

## Taker and maker coverage — no new finding

Paths in this table are under `crates/zecret-core/src/`.

| Attack area | Current source checked | Result and limit |
|---|---|---|
| Malicious maker directory entry | `taker.rs:284–296`, `:316–325` | Signature must verify and the signing key must be a full-access key of the named maker at a final block. Signed amount/premium limits are checked before sending. This authenticates a maker's entry; it does not make that maker honest. Endpoint trust and unsigned display metadata were already documented. |
| Malicious quote | `taker.rs:810–849` | Checks the maker's proof of knowledge, account, escrow, signature, minimum price, premium ceiling, stipend floor and timing bounds. No new bypass found. |
| Quote/open substitution | `taker.rs:875–901`; `maker.rs:936–956` | Taker checks stored token, amount, recipient, points, state, exact windows, code hash, keyless escrow and Verifier before funding. Maker checks an existing escrow against its recorded terms. |
| Replay and altered acceptance | `maker.rs:483–484`, `:711–734`; `taker.rs:676–678` | RFQ identifiers are deduplicated; acceptance is gated by stage and bound to the quote hash, committed seed and premium. Joint key and escrow identifier derive from both seeds and the RFQ identifier. |
| Both-assets theft at disclosure/refund | `taker.rs:1137–1174`; `maker.rs:1008–1057` | First disclosure rechecks the reveal deadline at handoff. Maker reads final state before refund, waits for lock expiry and rechecks the deadline-refund handoff. Later retransmission after a share is already public is distinct from first disclosure. |
| Peer supplies a false revealed share | `taker.rs:1287–1292`; `maker.rs:1153–1157` | Disclosed shares are checked against the peer's point before spending the joint notes. |
| Funds trapped by early erasure or recovery | `taker.rs:1264–1276`, `:1295–1307`, `:1325–1357` | Recovery prioritizes own funding, collects economical joint notes and keeps secrets while pending funding might still arrive. This checks the previous recovery fix; restart/reorg/fault behavior is **not checked** dynamically. |
| Post-claim/refund delivery | `taker.rs:1231–1256`; `maker.rs:1121–1149` | Taker exposes an independent withdrawal route; maker keeps polling refunded credit. Successful delivery still needs a funded caller and functioning upstream contracts, as already reported. |
| Deadline arithmetic | `taker.rs:822`, `:883–901`, `:1099`, `:1140` | Quote components are widened before summation, quoted windows compared exactly, and deadline/horizon checks use checked subtraction or saturation. Remaining plain maker timing additions depend on local configuration and chain timestamps; no independent malicious-quote overflow path was established. |
| Third-party request flooding | `maker.rs:319–331`, `:487–488`, `:828–837` | Deadline-bearing swaps precede unpaid work, pending requests are capped and quote work is limited. This does not eliminate the previously documented service and retained-record limits. |

One candidate was rejected: `find_quote` at `taker.rs:641–648` selects the first decryptable matching quote before signature verification. A generic outsider would need the current RFQ identifier, ephemeral public key and a taker wallet recipient address. The RFQ and diversified reply address are encrypted at `taker.rs:618–630`, within shielded memo transport. No source-backed outsider path to the necessary inputs was found. A malicious maker's ability to withhold a usable quote is already inherent; this is not a demonstrated new third-party attack.

## Relay coverage — no new finding

References are to `crates/zecret-node/src/relay.rs`.

| Attack area | Lines checked | Result and limit |
|---|---|---|
| Forge a victim's exit or consume its allowance | `:158–185`, `:241–304` | Checks one configured-token withdrawal, a payout-bearing swap and the amount ceiling. `simulate_intents` must succeed before the persistent allowance is incremented. The exact signed object is forwarded. A forged signer string alone is insufficient under the configured Verifier's signature checks. |
| Replay and gas consumption | `:289–345` | Counter is persisted before send and bounded at three per swap. Known hashes suppress resending. Lost responses can consume attempts; the bound is not an unconditional delivery guarantee. No new unbounded-spend path found. |
| Request-based blocking and memory growth | `:372–511`, `:515–607` | Checked four-worker/global slots, two connections per source, six relay requests per minute, source-map bounds, panic cleanup, whole-request read deadline and line/header/body limits. The previous blocking-network-call limitation is not re-reported. |
| HTTP framing ambiguity | `:575–607`, `:511` | Transfer-Encoding is rejected and each connection processes one request before shutdown. No new request-smuggling boundary was established. |

Production relay code and its unit tests at `:629–722` were read; **not run**. No slow-client, gas-drain, proxy, replay or malformed-payload probe was sent to the hosted service. Actual reverse-proxy source-IP behavior, RPC response time, filesystem crash consistency and transaction retry behavior are **not checked** in a deployed runtime.

## Remaining validation limits

This report adds no duplicate findings for the earlier RPC trust, missed maker refund window, upstream token/Verifier behavior, stipend economics, browser origin or liveness risks. Their existing reports remain applicable references; absence of a new finding does not close them.

The decisive remaining work is execution on the supported platform: run existing contract/client/relay regressions, inject response loss and restarts around persisted broadcasts, and test withdrawal callbacks with the pinned upstream binaries. Those checks, a formal proof of cross-chain atomicity, a funded mainnet exit and a new end-to-end adversarial swap are **not checked** here.
