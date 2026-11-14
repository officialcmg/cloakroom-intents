# Status

Updated: 2026-11-14 01:30 UTC. Deadline: 2026-11-14 23:59 UTC.

Any session can resume from this file. Read `docs/BUILD_PLAN.md` for the packages and
their checks.

## Not done

- Mainnet: nothing is deployed. The exit through NEAR Intents to another chain has not
  run with real funds.
- No wallet app was tested as the payer. Testnet payments came from the project's
  command-line wallet (`zecret pay`) and from the Fauzec faucet.
- Only Chromium was tested. Firefox and WebKit projects exist in
  `web/playwright.config.ts` (`ZECRET_BROWSERS=all`) and were not run: this machine has
  no Firefox or WebKit build (`docs/reviews/cross-browser.md`, follow-up).
- Open by decision: the taker trusts one NEAR RPC endpoint; the taker cannot pay for
  `withdraw` itself (the app shows the payout-pending state with the exact call). See
  `docs/security/`.

## Before submission (do not skip)

- [x] Screenshots in the README (`docs/img/`, taken after the redesign). Retake if the
  design changes again.
- [x] Docs name the site as `https://zecret.chrismg.dev`.
- [x] Screening dry run (`docs/reviews/screening-dry-run.md`, 72/100) worked through:
  pinned Rust (`rust-toolchain.toml`) and Node (`web/.nvmrc`), host and tool checks in
  `scripts/setup-env.sh`, clean-clone website test order in the README, missing engine
  and missing USDC wasm fail loudly, README claims tightened, the faucet route and a
  fresh-payer sequence documented, Firefox and WebKit projects added (not run).
- [ ] Push the current work to `main` with a message that does not start with "WIP", so
  Vercel deploys the leftover guard (website only; the engine crates did not change).
- [ ] After that deploy: open the hosted site once and check that the landing page and
  `/swap` load.
- [ ] Owner: NEAR mainnet payout account, repository public, demo video, post, form.

## Maker deploys (read before pushing changes under `crates/`)

Railway rebuilds the maker only when a file under `deploy/maker/` changes (set
2026-11-13, so that work-in-progress commits under `crates/` do not reach the live
maker). To release maker code: change `deploy/maker/RELEASE` (one line, the date and
what is released) in the same commit. Vercel still skips commits whose message starts
with "WIP".

## Overnight (2026-11-14, owner asleep)

- Soak test: `testnet/hosted/soak.sh` runs one real 0.01 TAZ swap through the live site
  every 15 minutes, 03:00 to 10:45 UTC (32 swaps). Results:
  `testnet/hosted/soak/results.jsonl`. The scheduled task "Zecret soak check" reads them
  every hour, fixes failures and notes them in `testnet/hosted/soak/NOTES.md`.
- Branch `escrow-light-payout` (worktree `/home/user/zecret-lw`): an escrow change so
  that the taker's key can finish the payout alone. Not merged. The live services stay
  on `escrow2` until it is proven; cut-off for a switch is 18:00 UTC.
- Branch `mainnet-ready` (worktree `/home/user/zecret-mainnet`): runbook, configs and
  no-funds checks for mainnet. Not merged. Nothing is done on mainnet without the owner.

## Latest (2026-11-14 01:30 UTC)

- Maker outage on 2026-11-13, 14:30 to 16:35 UTC: the maker's wallet no longer matched
  the chain and every quote was refused. Fixed in commit `cdaf614` (the wallet follows
  chain reorganisations and repairs its tree). Note:
  `docs/incidents/2026-11-13-maker-unknown-anchor.md`. Scenario s24 covers it.
- After the fix: two more swaps through the site completed (16:41 UTC, the demo video;
  00:57 UTC on 2026-11-14, a health check). Six swaps on public test networks in total.
- All 22 native scenarios pass on the final code (rerun by the main session, 01:24 UTC).
- Demo video: `testnet/hosted/demo/zecret-demo.mp4` on the build machine (not in git; the
  owner has a copy). Re-cut: `testnet/hosted/demo/edit_demo.py`.
- A scheduled task "Zecret maker health swap" runs one 0.01 TAZ swap through the site
  every 3 hours until the deadline and reports a failure to the owner.
- The 5 browser scenarios were last run in full before the wallet fix; scenario a was
  rerun after it.

## Hand-over: where the build is (2026-11-13 14:30 UTC)

Read this section first if you continue the build. Then `CLAUDE.md`, `docs/BUILD_PLAN.md`.

Live and verified: the site https://zecret.chrismg.dev (landing at `/`, app at `/swap`,
demo at `/?demo=1`; also https://zecret-chrismgs-projects.vercel.app) and the maker with
relay on Railway. The redesign is live from commit `23b1b9a`. A push to `main` redeploys
both; Vercel skips commits whose message starts with "WIP".

Done since the last hand-over:

1. Four swaps on public test networks completed; the fourth was paid by the Fauzec faucet
   with no wallet (`testnet/hosted/result-faucet-0.05.json`, 314 s from payment to done).
2. The restore fix is proven: restoring the faucet-paid swap in a fresh browser sent the
   surplus to the return address (Zcash tx
   `33bb8defb4bbc41716cf978c6c2339dec76f1a579446b1561a27cefac1dca34f`, 94,990,000 zats;
   return wallet 83,899,000 → 178,889,000 zats). `docs/TESTNET.md`, "Restore that
   returned the surplus".
3. README screenshots and the custom domain in the docs (commit `61c9c7e`).
4. Leftover guard (not yet deployed): while a finished swap's leftover ZEC has no return
   address or its transaction is not mined, `forget` and `createSwap` refuse with
   `LEFTOVER_PENDING`, and the Finished screen disables "Start a new swap" and offers
   "Start over without the leftover" behind a confirm step that names the amount. A swap
   restored after its delivery (`exit.doneElsewhere`) says so and shows no amount.
   Unit tests in `web/tests/unit/engine.test.ts` and `payoutCheck.test.ts`; e2e in
   `web/e2e/journey.spec.ts`.
5. Screening follow-up (list under "Before submission").

Wallet research (summary): the page's address has one Orchard-protocol receiver, and
since NU6.3 a wallet can pay it only into the Ironwood pool. Zkool and Zingo (Android)
offer testnet wallets and support Ironwood per their release notes; not tested against
the site. Zodl, Cake and Edge support Ironwood but ship no testnet app. YWallet and
zcashd cannot pay it.

Only on the build machine, not in git: `deploy/.keys/testnet/` (owner and old maker
keys) and `.tools/testnet/` (funded test wallets, recovery files). A new machine does not
have them. The hosted maker does not need them: its keys are on its Railway volume. A
new machine can still run a hosted swap with `ZECRET_PAY=faucet`.

Owner only: NEAR mainnet payout account, make the repository public, demo video, post,
submission form (`docs/SUBMISSION.md`, `docs/DEMO_SCRIPT.md`), anything on mainnet, and
the Railway start command (`docs/HOSTING.md`).

## Done

| Package | Result |
|---|---|
| H. Testnet hosting and a swap through the hosted site | Done (owner approved). Website on Vercel: https://zecret-chrismgs-projects.vercel.app (landing page at `/`, app at `/swap`, simulated demo at `/?demo=1`). Maker with its relay on Railway: https://zecret-maker-production.up.railway.app. Both redeploy on a push to `main` (seen for commit `908f763`); Vercel skips commits whose message starts with "WIP" and commits that change neither the site nor the engine crates. The hosted maker made its own wallet and NEAR key on its Railway volume; the key was added to `maker.zecret-3f109eea1e.testnet` (tx `BTpC5EAxUnFxUcMHSfvWJBFqXpky8nkHECgZK6v939fK`). Swap through the hosted site: 0.05 TAZ for 1.4877 test USDC, 493 s from payment to done (`docs/TESTNET.md`, "Hosted swap"; `testnet/hosted/result-0.05.json`). A first run with 0.01 TAZ refunded: the site reserved three Zcash fees and the engine four; fixed in `web/src/lib/price.ts` (commit `5d35000`). Rerun on the fixed site (12:07 to 12:14 UTC): 0.01 TAZ for 0.2877 test USDC, 392 s from payment to done, no CSP violation (`docs/TESTNET.md`, "Hosted swap after the price fix"; `testnet/hosted/result-0.01-fixed.json`; transactions taken from the result file, not audited separately). Pending: clear the custom start command on the Railway service (`docs/HOSTING.md`). |
| Web after the landing page (commit `5d35000`) | Landing page, responsive layout, one-line banners with details, start failures that name their cause. 88 unit tests and 74 Playwright tests pass (main session). |
| S2-fix (taker side) | PASS. The main session reran the core tests (65), clippy, and scenarios s1, s22 and s23; the agent reports all 21 native scenarios, all 5 browser scenarios and 50 Playwright tests passing on the same code. Fixed: F2-01, F2-02, F2-03, H1, H5, H6 (commit `6ec3ad7`). Added: a signed refusal memo, signed maker limits in the directory, a payout-pending stage, testnet timing sized from measured block intervals. The hosted swap (row H) ran with this code; the refusal and payout-pending paths were not run on testnet. See `docs/security/fixes-2-taker.md`. |
| S3. Security review 3 (website, browser engine, relay) | Done. 2 bugs, both in the maker's relay: F3-02 medium (one slow client stops the relay and can grow the maker's memory without limit), F3-01 low (anyone who sent one request can make the relay pay for transactions without limit). No way found for a third party to take a user's funds through the website or the engine. 2 open questions: a link can replace the whole network configuration (`?config=`), blocked on Vercel by the CSP but not on other hosts; two tabs on one swap have no lock. Report: `docs/security/review-3.md`. All fixed (row S3-fix). |
| S3-fix. Fixes for review 3 | Partly rerun by the main session: relay tests (11 unit, 9 relay), web type-check, lint and 78 unit tests pass. The agent also ran the build and 50 Playwright tests; the main session did not rerun those, to avoid disturbing the running browser scenarios. Relay: request deadline, size caps, 4 workers, one `ft_withdraw` per payload for a swap with a payout, at most 3 per swap, refused on mainnet. Website: production builds accept only known config names; a second tab shows "open in another tab"; the engine stops after a failed save; restored addresses are recomputed from the wallet key. At that time the relay was not run end to end with the browser engine; later the 5 browser scenarios passed through it (`docs/security/fixes-2-taker.md` §12), and so did the hosted swap (row H). See `docs/security/fixes-3.md`. |
| E. Taker engine in the browser, wired into the website | PASS. The agent ran five browser scenarios with the taker only in headless Chromium through the real page and the maker as the native program, on Zebra regtest and a NEAR sandbox with the real escrow: (a) full swap with exit, (b) maker goes quiet and the browser recovers the ZEC, (c) two reloads in the middle of a swap, (d) restore from the recovery file in a fresh browser, (e) quote below the minimum. The main session reran (a): PASS in 181 s, every Zcash transaction fully shielded. Browser timings: key build 20 to 26 s, one proof 27 to 33 s. On local and test networks the maker relays the taker's signed exit (`zecret maker --relay`), because the leftover stipend cannot pay for it. A browser swap on public test networks ran later through the hosted site (row H). Not verified: the 1Click exit with real funds, Firefox and Safari. See `harness/BROWSER_RESULTS.md`, `web/README.md`. |
| K. README, submission notes, demo script | Written. The agent ran every "do X, see Y" step in the README (escrow tests incl. real USDC, one scenario and the full 19, testnet `verify`, the simulated website, the Intents tests). The main session read the opening sections but did not rerun the steps. Updated on 2026-11-13 for the hosted site, 21 scenarios and the review-3 and taker-side fixes. |
| S2-fix (maker side) | PASS, rerun by the main session: core 26 unit tests, 11 bounds tests, the review's proof of concept, node tests, and scenarios s1 and s3 with the new escrow. F2-04 fixed: with 3,000 stuck requests a maker tick takes about 20 ms and 3 to 6 RPC calls, and the refund still goes out. Also: a Zcash sync failure no longer stops the maker's NEAR duties; the file store writes owner-only files atomically; the test-only `misbehave` setting is refused without a cargo feature; a `premium_check` setting refuses a premium worth less than the stipend and costs; `withdraw` uses 200 Tgas and the maker calls `sweep` before a retry. One clippy lint in `taker.rs:576` is open. See `docs/security/fixes-2-core.md`. |
| T. A swap on public test networks | Done. The native maker and taker swapped 0.1 testnet ZEC for 3 test USDC on Zcash testnet and NEAR testnet through `escrow2`, then the taker's signed withdrawal moved the tokens out of the stand-in verifier. Taker finished in 222 s, 322 s with the exit. The main session checked on NEAR by RPC: the claim was sent by the taker's own key and succeeded, and the escrow shows the swap as Claimed for 3,000,000 units. The Zcash side was checked by the agent only (6 transactions, all fully shielded). Testnet ZEC came from the Zcash Foundation faucet. Evidence: `docs/TESTNET.md`, `testnet/EVIDENCE.json`. The two wallets in `.tools/testnet/` (untracked) still hold funds. |
| NV-02 fix. Two-step payout | PASS, rerun by the main session: 57 unit tests, 6 sandbox tests, and 8 tests against the USDC code from mainnet (pause, blocklist and races between delivery and resolve: the taker is paid exactly once). The old escrow fails the same tests. New testnet escrow: `escrow2.zecret-3f109eea1e.testnet`, code hash `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`, no access keys (checked by RPC). `withdraw` now needs 200 Tgas. See `docs/security/fixes-2.md`. |
| S2. Security review 2 (core, native client) | Done. 4 bugs: F2-01 medium (taker erases its key half after a refund while its funding is still at the shared address), F2-02 medium (first claim sent without re-checking the reveal deadline), F2-04 medium (unpaid requests slow the maker without limit), F2-03 low (an oversized reveal window freezes both sides). 2 open questions: one NEAR RPC endpoint is trusted (still open); the premium is not tied to the stipend (addressed by `premium_check` in S2-fix). Report and proofs: `docs/security/review-2.md`, `poc2/`. The taker-side fixes are in row "S2-fix (taker side)". |
| CD. Shared core, NEAR adapter, native maker and taker | PASS. The agent ran 19 two-party scenarios (maker and taker as separate programs, Zebra regtest plus NEAR sandbox with the real escrow and the live `intents.near` code); the main session reran s1 (happy path), s3 (maker refund) and s4 (punish path) and the 25 core unit tests, all passing. `crates/zecret-core` has no I/O and builds for WebAssembly. The binary is `zecret` in `crates/zecret-node`. Reorganization scenarios s13 and s14 were dropped: NEAR final blocks cannot be reorganized. Later s22 and s23 were added (21 scenarios). See `harness/RESULTS.md`, `docs/ENGINE.md`. |
| NV-02. Real USDC check | Done: the risk is real. USDC's resolve step fails when USDC is paused or an account is blocklisted between delivery and resolve. The escrow then restores a credit that was already paid, and a second withdraw pays twice. Reproduced in the sandbox with the USDC code from mainnet. Only Circle can trigger it. Fixed by the two-step payout (row "NV-02 fix" above). See `docs/security/nv02-usdc.md`. `docs/INTENTS.md` is in line with the security fixes. |
| I1. Deployment tooling, live on NEAR testnet | Done. Escrow `escrow.zecret-3f109eea1e.testnet` (now retired; replaced by `escrow2` in the NV-02 fix), test USDC, stand-in verifier and a funded maker are deployed (`deploy/deployment.testnet.json`). The main session checked by RPC: the escrow has 0 access keys, its code hash equals the local build, and its Intents account is the stand-in verifier. Two full contract-level swaps ran on testnet (hashes in `deploy/e2e.testnet.json`). The tool's own tests were not rerun by the main session. Keys are in `deploy/.keys/` (untracked); losing this machine loses them, and the deployment must then be redone. Mainnet estimate: about 6.6 NEAR. See `docs/DEPLOY.md`. |
| F. Website | PASS, rerun by the main session: type-check and lint clean, 54 unit tests, build, 34 browser tests on desktop and phone. Screens reviewed by eye in both themes. Runs against a simulated engine behind the interface in `web/ENGINE_API.md`; the real engine replaces it with a one-line change in `web/src/engine/index.ts`. The Content-Security-Policy header was added later (`web/vercel.json`). Not done: Firefox and Safari. |
| S1. Security review 1 (escrow, Intents library) | Done. 2 bugs: F-01 medium (a third party can block a maker's swap through a fake token), F-02 low (quote freshness read from an unsigned field). 2 items need facts from outside the repo (escrow deployment without keys; USDC's resolve step). No way found to steal funds. Report: `docs/security/review-1.md`. |
| S1-fix. Fixes for review 1 | PASS, rerun by the main session. F-01, F-02 and NV-01 fixed, each with a regression test: escrow 50 unit and 6 sandbox tests, Intents library 24 unit and 21 recorded tests. The escrow's `new` is private, so deployment is: create the account, then one transaction signed by the escrow that deploys, calls `new` and deletes its key. NV-02 (USDC resolve step) was fixed later (row "NV-02 fix"). See `docs/security/fixes-1.md`. |
| G. Exit through Intents | PASS, rerun by the main session: 24 unit, 18 recorded-response and 2 sandbox tests against the live `intents.near` code; builds for wasm; clippy and fmt clean. No API key is needed for quote, generate or submit (tested live with an unfunded key). Execution with real funds is not verified. See `docs/INTENTS.md`. |
| A. Escrow contract | PASS, rerun by the main session: 48 unit tests, 5 sandbox tests against the live `intents.near` and `wrap.near` code, clippy and fmt clean. `claim` 16.8 Tgas, taker withdraw into Intents 42.4 Tgas. Findings: the stipend must be 0.03 NEAR; contracts need `--cfg near`; the maker prepays swap storage. See `contracts/near-escrow/README.md`. |
| B. Browser spike | PASS, rerun by the main session. In headless Chromium the wallet in a Web Worker receives a shielded payment, reads its memo, and builds, proves and broadcasts a fully shielded transaction with a 512-byte memo on Zebra regtest. Key build about 20 s, one send about 27 s. Run: `spike/browser-wallet/run.mjs`. Findings and the recommended engine structure: `docs/experiments/browser-wallet.md`. |
| Wave 0: repository setup | Cloakroom imported as commit `2f8e2f7`, tagged `pre-hackathon` in the build clone and pushed as the branch `pre-hackathon-baseline` (the git proxy refuses tags). `scripts/setup-env.sh` prepares the sandbox, the Intents contract code, the open-file shim and Docker. |

## Running

Nothing.

## Fix rounds

All fix rounds are done. Still open by decision, not by omission: one NEAR RPC endpoint is trusted; the taker cannot pay for `withdraw` itself (the app shows a payout-pending state with the exact call); the bundled sample directory's premium minimum.

## For the owner

- Make the repository public before 2026-11-14 23:59 UTC.
- The build environment cannot push git tags. The baseline commit `2f8e2f7` is pushed as branch `pre-hackathon-baseline`, and the README's diff link uses it. The tag `pre-hackathon` exists only in the build clone.
- The README has one sentence saying parts of the code and documentation were written with AI coding assistance (hackathon rule Q-005 asks for this). It names no product. Keep, reword or remove.
- `design-system/README.md` (baseline, from Cloakroom) names an AI product and links to it. The rules file of this repository is itself named after a tool (`CLAUDE.md`). Decide whether to remove or rename them before the repository goes public.
- Clear the custom start command on the Railway service (`docs/HOSTING.md`, "Start command").
- Anything on mainnet waits for approval.
- Record the demo video, post, and submit (`docs/DEMO_SCRIPT.md`, `docs/SUBMISSION.md`).

## Notes for whoever resumes

- Usage window resets at 03:10 UTC on 2026-11-13, then every 5 hours (08:10, 13:10, 18:10 UTC). A full window lasted about 4 hours with three agents.
- An hourly scheduled task wakes the main session (trigger `trig_01Bbv8VUrvwpGeEr3NWMoRAU`).
- Main session: `session_01UpiMzVjgad99bzMjoPrgGL`.
- Agents edit separate folders and do not commit. The main session reviews, runs the
  check and commits.
- Because this machine is temporary, the main session also pushes snapshots of
  work in progress. Their commit messages start with "WIP". A package counts as done
  only when it is listed under "Done" above.
