# Status

Updated: 2026-11-13 06:58 UTC. Deadline: 2026-11-14 23:59 UTC.

Any session can resume from this file. Read `docs/BUILD_PLAN.md` for the packages and
their checks.

## Done

| Package | Result |
|---|---|
| K. README, submission notes, demo script | Written. The agent ran every "do X, see Y" step in the README (escrow tests incl. real USDC, one scenario and the full 19, testnet `verify`, the simulated website, the Intents tests). The main session read the opening sections but did not rerun the steps. The README's Status table must be updated when package E finishes. |
| S2-fix (maker side) | PASS, rerun by the main session: core 26 unit tests, 11 bounds tests, the review's proof of concept, node tests, and scenarios s1 and s3 with the new escrow. F2-04 fixed: with 3,000 stuck requests a maker tick takes about 20 ms and 3 to 6 RPC calls, and the refund still goes out. Also: a Zcash sync failure no longer stops the maker's NEAR duties; the file store writes owner-only files atomically; the test-only `misbehave` setting is refused without a cargo feature; a `premium_check` setting refuses a premium worth less than the stipend and costs; `withdraw` uses 200 Tgas and the maker calls `sweep` before a retry. One clippy lint in `taker.rs:576` is open. See `docs/security/fixes-2-core.md`. |
| T. A swap on public test networks | Done. The native maker and taker swapped 0.1 testnet ZEC for 3 test USDC on Zcash testnet and NEAR testnet through `escrow2`, then the taker's signed withdrawal moved the tokens out of the stand-in verifier. Taker finished in 222 s, 322 s with the exit. The main session checked on NEAR by RPC: the claim was sent by the taker's own key and succeeded, and the escrow shows the swap as Claimed for 3,000,000 units. The Zcash side was checked by the agent only (6 transactions, all fully shielded). Testnet ZEC came from the Zcash Foundation faucet. Evidence: `docs/TESTNET.md`, `testnet/EVIDENCE.json`. The two wallets in `.tools/testnet/` (untracked) still hold funds. |
| NV-02 fix. Two-step payout | PASS, rerun by the main session: 57 unit tests, 6 sandbox tests, and 8 tests against the USDC code from mainnet (pause, blocklist and races between delivery and resolve: the taker is paid exactly once). The old escrow fails the same tests. New testnet escrow: `escrow2.zecret-3f109eea1e.testnet`, code hash `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`, no access keys (checked by RPC). `withdraw` now needs 200 Tgas. Still open: the client code and configs in `crates/` use 100 Tgas and the old escrow id. See `docs/security/fixes-2.md`. |
| S2. Security review 2 (core, native client) | Done. 4 bugs: F2-01 medium (taker erases its key half after a refund while its funding is still at the shared address), F2-02 medium (first claim sent without re-checking the reveal deadline), F2-04 medium (unpaid requests slow the maker without limit), F2-03 low (an oversized reveal window freezes both sides). 2 open questions: one NEAR RPC endpoint is trusted; the premium is not tied to the stipend. Report and proofs: `docs/security/review-2.md`, `poc2/`. The taker-side fixes (F2-01, F2-02, F2-03) wait until package E finishes, because E is editing `taker.rs`. |
| CD. Shared core, NEAR adapter, native maker and taker | PASS. The agent ran 19 two-party scenarios (maker and taker as separate programs, Zebra regtest plus NEAR sandbox with the real escrow and the live `intents.near` code); the main session reran s1 (happy path), s3 (maker refund) and s4 (punish path) and the 25 core unit tests, all passing. `crates/zecret-core` has no I/O and builds for WebAssembly. The binary is `zecret` in `crates/zecret-node`. Reorganization scenarios s13 and s14 were dropped: NEAR final blocks cannot be reorganized. Not verified: a swap on public test networks. See `harness/RESULTS.md`, `docs/ENGINE.md`. |
| NV-02. Real USDC check | Done: the risk is real. USDC's resolve step fails when USDC is paused or an account is blocklisted between delivery and resolve. The escrow then restores a credit that was already paid, and a second withdraw pays twice. Reproduced in the sandbox with the USDC code from mainnet. Only Circle can trigger it. A fix is designed and in progress. See `docs/security/nv02-usdc.md`. `docs/INTENTS.md` is in line with the security fixes. |
| I1. Deployment tooling, live on NEAR testnet | Done. Escrow `escrow.zecret-3f109eea1e.testnet`, test USDC, stand-in verifier and a funded maker are deployed (`deploy/deployment.testnet.json`). The main session checked by RPC: the escrow has 0 access keys, its code hash equals the local build, and its Intents account is the stand-in verifier. Two full contract-level swaps ran on testnet (hashes in `deploy/e2e.testnet.json`). The tool's own tests were not rerun by the main session. Keys are in `deploy/.keys/` (untracked); losing this machine loses them, and the deployment must then be redone. Mainnet estimate: about 6 NEAR. See `docs/DEPLOY.md`. |
| F. Website | PASS, rerun by the main session: type-check and lint clean, 54 unit tests, build, 34 browser tests on desktop and phone. Screens reviewed by eye in both themes. Runs against a simulated engine behind the interface in `web/ENGINE_API.md`; the real engine replaces it with a one-line change in `web/src/engine/index.ts`. Not yet done: Content-Security-Policy header, Firefox and Safari. |
| S1. Security review 1 (escrow, Intents library) | Done. 2 bugs: F-01 medium (a third party can block a maker's swap through a fake token), F-02 low (quote freshness read from an unsigned field). 2 items need facts from outside the repo (escrow deployment without keys; USDC's resolve step). No way found to steal funds. Report: `docs/security/review-1.md`. |
| S1-fix. Fixes for review 1 | PASS, rerun by the main session. F-01, F-02 and NV-01 fixed, each with a regression test: escrow 50 unit and 6 sandbox tests, Intents library 24 unit and 21 recorded tests. The escrow's `new` is private, so deployment is: create the account, then one transaction signed by the escrow that deploys, calls `new` and deletes its key. NV-02 (USDC resolve step) is still open. See `docs/security/fixes-1.md`. |
| G. Exit through Intents | PASS, rerun by the main session: 24 unit, 18 recorded-response and 2 sandbox tests against the live `intents.near` code; builds for wasm; clippy and fmt clean. No API key is needed for quote, generate or submit (tested live with an unfunded key). Execution with real funds is not verified. See `docs/INTENTS.md`. |
| A. Escrow contract | PASS, rerun by the main session: 48 unit tests, 5 sandbox tests against the live `intents.near` and `wrap.near` code, clippy and fmt clean. `claim` 16.8 Tgas, taker withdraw into Intents 42.4 Tgas. Findings: the stipend must be 0.03 NEAR; contracts need `--cfg near`; the maker prepays swap storage. See `contracts/near-escrow/README.md`. |
| B. Browser spike | PASS, rerun by the main session. In headless Chromium the wallet in a Web Worker receives a shielded payment, reads its memo, and builds, proves and broadcasts a fully shielded transaction with a 512-byte memo on Zebra regtest. Key build about 20 s, one send about 27 s. Run: `spike/browser-wallet/run.mjs`. Findings and the recommended engine structure: `docs/experiments/browser-wallet.md`. |
| Wave 0: repository setup | Cloakroom imported and tagged `pre-hackathon`. `scripts/setup-env.sh` prepares the sandbox, the Intents contract code, the open-file shim and Docker. |

## Running

| Package | Started (UTC) | Where the work is |
|---|---|---|
| E. Taker engine in the browser, wired into the website | 04:44 | `crates/zecret-wasm/`, `web/`, `harness/` |
| K2. Bring stale documents in line with the facts | 06:58 | `docs/`, `README.md`, `BASELINE.md` |

## Next

After E: a swap on public test networks (needs testnet ZEC for the maker and taker), README and documentation (K), a second security review of the core, engine and website with fixes, and the final wording review (L).

Plan change: packages C and D are merged into one (CD), because the browser spike showed the right structure is one shared core crate with no I/O. The website (F) starts now against a documented engine interface with a simulated engine, so UX work runs in parallel.

## Next fix round (after package E finishes)

- Taker side of security review 2: F2-01, F2-02, F2-03, H1, H6, and compiling the `misbehave` branches out (`crates/zecret-core/src/taker.rs`; notes in `docs/security/fixes-2-core.md` section 6). Also the clippy lint at `taker.rs:576`.
- `deploy/maker/README.md` shows a premium of 20,000 zats, which fails the new premium check at realistic prices.
- From the testnet swap: the maker sends no answer to a request above its limit, so the taker waits the full quote timeout (`maker.rs` around line 440); the maker's directory entry should publish its limits; a wrong "0 spendable zats" warning in the maker log.
- The taker can never pay for `withdraw` (200 Tgas needs about 0.2 NEAR). If the maker stops after the claim, the USDC waits in the escrow until any account sends `withdraw`. Decide: a larger stipend, or say it plainly in the app and docs.
- Zcash testnet blocks arrive every 23 s on average; the configs are sized for 75 s blocks.

## For the owner

- The repository is private. It must be public before 2026-11-14 23:59 UTC.
- The build environment cannot push git tags. The baseline commit `2f8e2f7` is pushed as branch `pre-hackathon-baseline`, and the README's diff link uses it. The tag `pre-hackathon` exists only in the build clone.
- The README has one sentence saying parts of the code and documentation were written with AI coding assistance (hackathon rule Q-005 asks for this). It names no product. Keep, reword or remove.
- Decide how the taker's `withdraw` should be paid if the maker stops after a claim (see "Next fix round").
- Public hosting of the website and the maker, and anything on mainnet, wait for approval.

## Blocked or skipped

- Public hosting of the website and the maker: not approved by the owner. Prepared only.

## Notes for whoever resumes

- Usage window resets at 03:10 UTC on 2026-11-13, then every 5 hours.
- An hourly scheduled task wakes the main session (trigger `trig_01Bbv8VUrvwpGeEr3NWMoRAU`).
- Main session: `session_01UpiMzVjgad99bzMjoPrgGL`.
- Agents edit separate folders and do not commit. The main session reviews, runs the
  check and commits.
- Because this machine is temporary, the main session also pushes snapshots of
  work in progress. Their commit messages start with "WIP". A package counts as done
  only when it is listed under "Done" above.
