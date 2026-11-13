# Status

Updated: 2026-11-13 08:18 UTC. Deadline: 2026-11-14 23:59 UTC.

Any session can resume from this file. Read `docs/BUILD_PLAN.md` for the packages and
their checks.

## Done

| Package | Result |
|---|---|
| E. Taker engine in the browser, wired into the website | PASS. The agent ran five browser scenarios with the taker only in headless Chromium through the real page and the maker as the native program, on Zebra regtest and a NEAR sandbox with the real escrow: (a) full swap with exit, (b) maker goes quiet and the browser recovers the ZEC, (c) two reloads in the middle of a swap, (d) restore from the recovery file in a fresh browser, (e) quote below the minimum. The main session reran (a): PASS in 181 s, every Zcash transaction fully shielded. Browser timings: key build 20 to 26 s, one proof 27 to 33 s. On local and test networks the maker relays the taker's signed exit (`zecret maker --relay`), because the leftover stipend cannot pay for it. Not verified: a browser swap on public test networks (a partial check ran), the 1Click exit with real funds, Firefox and Safari. See `harness/BROWSER_RESULTS.md`, `web/README.md`. |
| K. README, submission notes, demo script | Written. The agent ran every "do X, see Y" step in the README (escrow tests incl. real USDC, one scenario and the full 19, testnet `verify`, the simulated website, the Intents tests). The main session read the opening sections but did not rerun the steps. The README's Status table must be updated when package E finishes. |
| S2-fix (maker side) | PASS, rerun by the main session: core 26 unit tests, 11 bounds tests, the review's proof of concept, node tests, and scenarios s1 and s3 with the new escrow. F2-04 fixed: with 3,000 stuck requests a maker tick takes about 20 ms and 3 to 6 RPC calls, and the refund still goes out. Also: a Zcash sync failure no longer stops the maker's NEAR duties; the file store writes owner-only files atomically; the test-only `misbehave` setting is refused without a cargo feature; a `premium_check` setting refuses a premium worth less than the stipend and costs; `withdraw` uses 200 Tgas and the maker calls `sweep` before a retry. One clippy lint in `taker.rs:576` is open. See `docs/security/fixes-2-core.md`. |
| T. A swap on public test networks | Done. The native maker and taker swapped 0.1 testnet ZEC for 3 test USDC on Zcash testnet and NEAR testnet through `escrow2`, then the taker's signed withdrawal moved the tokens out of the stand-in verifier. Taker finished in 222 s, 322 s with the exit. The main session checked on NEAR by RPC: the claim was sent by the taker's own key and succeeded, and the escrow shows the swap as Claimed for 3,000,000 units. The Zcash side was checked by the agent only (6 transactions, all fully shielded). Testnet ZEC came from the Zcash Foundation faucet. Evidence: `docs/TESTNET.md`, `testnet/EVIDENCE.json`. The two wallets in `.tools/testnet/` (untracked) still hold funds. |
| NV-02 fix. Two-step payout | PASS, rerun by the main session: 57 unit tests, 6 sandbox tests, and 8 tests against the USDC code from mainnet (pause, blocklist and races between delivery and resolve: the taker is paid exactly once). The old escrow fails the same tests. New testnet escrow: `escrow2.zecret-3f109eea1e.testnet`, code hash `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`, no access keys (checked by RPC). `withdraw` now needs 200 Tgas. See `docs/security/fixes-2.md`. |
| S2. Security review 2 (core, native client) | Done. 4 bugs: F2-01 medium (taker erases its key half after a refund while its funding is still at the shared address), F2-02 medium (first claim sent without re-checking the reveal deadline), F2-04 medium (unpaid requests slow the maker without limit), F2-03 low (an oversized reveal window freezes both sides). 2 open questions: one NEAR RPC endpoint is trusted (still open); the premium is not tied to the stipend (addressed by `premium_check` in S2-fix). Report and proofs: `docs/security/review-2.md`, `poc2/`. The taker-side fixes (F2-01, F2-02, F2-03) wait until package E finishes, because E is editing `taker.rs`. |
| CD. Shared core, NEAR adapter, native maker and taker | PASS. The agent ran 19 two-party scenarios (maker and taker as separate programs, Zebra regtest plus NEAR sandbox with the real escrow and the live `intents.near` code); the main session reran s1 (happy path), s3 (maker refund) and s4 (punish path) and the 25 core unit tests, all passing. `crates/zecret-core` has no I/O and builds for WebAssembly. The binary is `zecret` in `crates/zecret-node`. Reorganization scenarios s13 and s14 were dropped: NEAR final blocks cannot be reorganized. See `harness/RESULTS.md`, `docs/ENGINE.md`. |
| NV-02. Real USDC check | Done: the risk is real. USDC's resolve step fails when USDC is paused or an account is blocklisted between delivery and resolve. The escrow then restores a credit that was already paid, and a second withdraw pays twice. Reproduced in the sandbox with the USDC code from mainnet. Only Circle can trigger it. Fixed by the two-step payout (row "NV-02 fix" above). See `docs/security/nv02-usdc.md`. `docs/INTENTS.md` is in line with the security fixes. |
| I1. Deployment tooling, live on NEAR testnet | Done. Escrow `escrow.zecret-3f109eea1e.testnet` (now retired; replaced by `escrow2` in the NV-02 fix), test USDC, stand-in verifier and a funded maker are deployed (`deploy/deployment.testnet.json`). The main session checked by RPC: the escrow has 0 access keys, its code hash equals the local build, and its Intents account is the stand-in verifier. Two full contract-level swaps ran on testnet (hashes in `deploy/e2e.testnet.json`). The tool's own tests were not rerun by the main session. Keys are in `deploy/.keys/` (untracked); losing this machine loses them, and the deployment must then be redone. Mainnet estimate: about 6 NEAR. See `docs/DEPLOY.md`. |
| F. Website | PASS, rerun by the main session: type-check and lint clean, 54 unit tests, build, 34 browser tests on desktop and phone. Screens reviewed by eye in both themes. Runs against a simulated engine behind the interface in `web/ENGINE_API.md`; the real engine replaces it with a one-line change in `web/src/engine/index.ts`. Not yet done: Content-Security-Policy header, Firefox and Safari. |
| S1. Security review 1 (escrow, Intents library) | Done. 2 bugs: F-01 medium (a third party can block a maker's swap through a fake token), F-02 low (quote freshness read from an unsigned field). 2 items need facts from outside the repo (escrow deployment without keys; USDC's resolve step). No way found to steal funds. Report: `docs/security/review-1.md`. |
| S1-fix. Fixes for review 1 | PASS, rerun by the main session. F-01, F-02 and NV-01 fixed, each with a regression test: escrow 50 unit and 6 sandbox tests, Intents library 24 unit and 21 recorded tests. The escrow's `new` is private, so deployment is: create the account, then one transaction signed by the escrow that deploys, calls `new` and deletes its key. NV-02 (USDC resolve step) was fixed later (row "NV-02 fix"). See `docs/security/fixes-1.md`. |
| G. Exit through Intents | PASS, rerun by the main session: 24 unit, 18 recorded-response and 2 sandbox tests against the live `intents.near` code; builds for wasm; clippy and fmt clean. No API key is needed for quote, generate or submit (tested live with an unfunded key). Execution with real funds is not verified. See `docs/INTENTS.md`. |
| A. Escrow contract | PASS, rerun by the main session: 48 unit tests, 5 sandbox tests against the live `intents.near` and `wrap.near` code, clippy and fmt clean. `claim` 16.8 Tgas, taker withdraw into Intents 42.4 Tgas. Findings: the stipend must be 0.03 NEAR; contracts need `--cfg near`; the maker prepays swap storage. See `contracts/near-escrow/README.md`. |
| B. Browser spike | PASS, rerun by the main session. In headless Chromium the wallet in a Web Worker receives a shielded payment, reads its memo, and builds, proves and broadcasts a fully shielded transaction with a 512-byte memo on Zebra regtest. Key build about 20 s, one send about 27 s. Run: `spike/browser-wallet/run.mjs`. Findings and the recommended engine structure: `docs/experiments/browser-wallet.md`. |
| Wave 0: repository setup | Cloakroom imported as commit `2f8e2f7`, tagged `pre-hackathon` in the build clone and pushed as the branch `pre-hackathon-baseline` (the git proxy refuses tags). `scripts/setup-env.sh` prepares the sandbox, the Intents contract code, the open-file shim and Docker. |

## Running

| Package | Started (UTC) | Where the work is |
|---|---|---|

## Running now (resumed 08:15 UTC)

The 5-hour usage limit was reached at about 07:12 UTC and reset at 08:10 UTC. Both agents below stopped at their first step and were resumed at 08:15 UTC. About one hour was lost. The next reset is 13:10 UTC; from now on at most two agents run at the same time.

- Taker-side fixes: F2-01, F2-02, F2-03, H1, H6, the refusal reply, published limits, the payout-pending state, testnet timing configs. In `crates/` and `harness/`. It must rerun all native scenarios and all five browser scenarios.
- Security review 3: the website, `zecret-wasm` and the maker's relay. Read-only, writes `docs/security/review-3.md`.

After these: fixes for review 3, a browser swap on public test networks with a local maker and relay, update the README status table and demo script, the final wording pass with `docs/WRITING_CHECKLIST.md`.

## Next

After E: a security review of the browser engine and the website, the taker-side fixes of review 2, and the final wording review (L). The swap on public test networks (T) and the README and documentation (K) are done.

Plan change: packages C and D are merged into one (CD), because the browser spike showed the right structure is one shared core crate with no I/O. The website (F) starts now against a documented engine interface with a simulated engine, so UX work runs in parallel.

## Next fix round (after package E finishes)

- Taker side of security review 2: F2-01, F2-02, F2-03, H1, H6, and compiling the `misbehave` branches out (`crates/zecret-core/src/taker.rs`; notes in `docs/security/fixes-2-core.md` section 6). Also the clippy lint at `taker.rs:576`.
- `web/public/maker-directory.json` (the bundled sample directory) shows a premium minimum of 0.0002 ZEC (20,000 zats). With the stipend of 0.03 NEAR and the default costs of 0.015 NEAR, that passes the maker's premium check only if 1 ZEC is worth at least 225 NEAR. `deploy/maker/README.md` now uses 450,000 zats.
- From the testnet swap: the maker sends no answer to a request above its limit, so the taker waits the full quote timeout (`maker.rs` around line 440); the maker's directory entry should publish its limits; a wrong "0 spendable zats" warning in the maker log.
- The taker can never pay for `withdraw` (200 Tgas needs about 0.2 NEAR). If the maker stops after the claim, the USDC waits in the escrow until any account sends `withdraw`. Decide: a larger stipend, or say it plainly in the app and docs.
- Zcash testnet blocks arrive every 23 s on average; the configs are sized for 75 s blocks.

## For the owner

- The repository is private. It must be public before 2026-11-14 23:59 UTC.
- The build environment cannot push git tags. The baseline commit `2f8e2f7` is pushed as branch `pre-hackathon-baseline`, and the README's diff link uses it. The tag `pre-hackathon` exists only in the build clone.
- The README has one sentence saying parts of the code and documentation were written with AI coding assistance (hackathon rule Q-005 asks for this). It names no product. Keep, reword or remove.
- `design-system/README.md` (baseline, from Cloakroom) names an AI product and links to it. The rules file of this repository is itself named after a tool (`CLAUDE.md`). Decide whether to remove or rename them before the repository goes public.
- Decide how the taker's `withdraw` should be paid if the maker stops after a claim (see "Next fix round").
- Public hosting of the website and the maker, and anything on mainnet, wait for approval.

## Blocked or skipped

- Public hosting of the website and the maker: not approved by the owner. Prepared only.
- E (browser engine), testnet: a full browser swap on Zcash testnet + NEAR testnet was not
  run. What ran (`harness/browser/testnet-smoke.mjs`, production CSP): engine load, maker
  directory check against NEAR testnet, key build, gRPC-web scan of Zcash testnet, a swap
  created up to the payment screen. Missing for a full run: testnet ZEC for the user and
  for the maker's quote fees, a maker running on testnet with `--relay` (and a public URL
  for the relay in `web/public/config/testnet.json` and the CSP `connect-src`), the
  published `web/public/config/testnet-directory.json`, and a NEAR testnet account
  registered with the test USDC token.

## Notes for whoever resumes

- Usage window resets at 03:10 UTC on 2026-11-13, then every 5 hours (08:10, 13:10, 18:10 UTC). A full window lasted about 4 hours with three agents.
- An hourly scheduled task wakes the main session (trigger `trig_01Bbv8VUrvwpGeEr3NWMoRAU`).
- Main session: `session_01UpiMzVjgad99bzMjoPrgGL`.
- Agents edit separate folders and do not commit. The main session reviews, runs the
  check and commits.
- Because this machine is temporary, the main session also pushes snapshots of
  work in progress. Their commit messages start with "WIP". A package counts as done
  only when it is listed under "Done" above.
