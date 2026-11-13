# Screening dry run

Reviewed 2026-11-13, Africa/Nairobi (UTC+3), source commit `ac9c2ff79ae4a400c00312c637d6004c663ae78f`, on `codex/review`. The repository instructions, README, STATUS and the design's opening section were read first. No application source was changed.

This is a reproducibility and feature screening, not a certification. **Confirmed** below means the named observation or repository evidence supports the claim; it does not turn a stored log into a fresh successful run. **Not confirmed** means insufficient independent evidence; **contradicted** means contrary evidence. Every unexecuted check is marked **not checked**.

## Machine and execution boundaries

- Host: `Darwin arm64`; Node `v24.2.0`, npm `11.3.0`, Rust `1.94.0`. Only `aarch64-apple-darwin` was installed as a Rust target. Python, curl, gcc and the Docker CLI are present.
- The documented local chain environment is Linux x86-64, Rust 1.97.0 and Node 22 (`README.md:121`). `scripts/setup-env.sh:16` downloads a Linux-x86_64 sandbox unconditionally. That binary cannot run natively on this host. `docker info` was attempted but access to the daemon socket was denied by the execution sandbox; the daemon's health and architecture are **not checked**. No Linux VM was provisioned.
- Website commands ran in `/private/tmp/zecret-review/web`, a copy of `web/`, with the referenced `deploy/deployment.testnet.json` also copied. This explicit deviation protects the instruction not to edit `web/`: installs, builds, downloads and test outputs stay in temporary storage. Application source and browser configuration files were unchanged. `npm install` normalized one root dependency specifier in the temporary lockfile; it did not change the repository lockfile. The original repository remained clean until this report was added.
- The first network request failed DNS inside the execution sandbox. With network access enabled the hosted site returned HTTP 200. The first local preview server failed with `listen EPERM ::1:4173`; subsequent browser testing used localhost access. These were execution restrictions, not site failures.
- Hosted checks used headless Google Chrome, 1280 × 900, fresh browser contexts, the requested canonical domain `https://zecret.chrismg.dev`, and no API mocks. The primary hosted run had no speed override; a separately identified supplemental run used 20× speed. Local end-to-end tests use the repository's simulated engine and mocked 1Click responses.

## README Try it, step by step

### 1. Open the website

| README Do / See row | Actual observation | Match |
|---|---|---|
| `README.md:77`: open the landing page | HTTP 200. Saw How a swap works, Who holds the money, What stays private and what is public, If something goes wrong, Status and Questions. | Confirmed. Domain substituted as requested. Landing status still says two completed testnet swaps, whereas README lists three. |
| `README.md:78`: Try the simulated demo | Link opened the form and banner `Demo. No real money moves.` Token list request returned 200. | Confirmed. |
| `README.md:79`: enter 1, USDC on Base, the supplied Ethereum address | Entered exactly `1` and `0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed`. Quote endpoint returned 201. Saw `You pay 1.00 ZEC. You receive at least 916.5921 USDC on Base.` Expected output 935.2026. | Confirmed. Example price is correctly nonbinding. |
| `README.md:80`: Continue, Save recovery file, Continue to payment | Download event: `zecret-recovery-2b62f03c.json`. Saw `Send exactly 1.00 ZEC`, a rendered QR code and a `utest1…` address. | Confirmed. QR visually inspected; decoding its payload independently is **not checked**. |
| `README.md:81`: Simulate the payment | Clicked as written. The original real-time observation ended with `locator.waitFor: Timeout 600000ms exceeded` waiting for “You received”; completion in that run is **not checked** beyond that limit. A separately labeled 20× run showed all six groups and finished with `You received 935.2573 USDC on Base` and `What this swap revealed`. | UI sequence confirmed by supplemental accelerated run; default-speed completion **not confirmed**, not classified as a product failure |
| `README.md:82`: Details, Demo controls | A separate fresh demo context opened Details → Demo controls. Saw speed choices, maker quote/timeout/refund/disappearance, transient exit quote, below-minimum delivery, failed delivery, skip-wait and cleared-browser controls. | Confirmed controls are present; each hosted failure outcome is **not checked** |

Supplemental deviation: the second demo used `/?demo=1&speed=20`; its six groups were payment, maker price, maker escrow, shared-address funding, USDC claim and Base delivery. After capturing its completion, an attempt to locate the banner by a test-only selector timed out; the independent controls check used visible button labels and succeeded. The original default-speed timeout is not proof of a stalled engine: its source schedule uses 75-second simulated blocks (`web/src/engine/timeline.ts:7`), and no final progress snapshot was captured at the timeout.

No real payment was sent to the simulated address. Individual hosted failure scenarios are **not checked** merely by seeing their controls; local suite results are separately identified below.

### 2. Swap on testnet through the website

| README Do / See row | Actual observation | Match |
|---|---|---|
| `README.md:109`: Start a swap | Opened `/swap`. Banner `Testnet. Test money only.` Real engine initialized and the form accepted input. | Confirmed for route/banner/initialization. Exact engine byte size and 20-second preparation benchmark are **not checked**; the scripted observation allowed 35 seconds before reading the form. |
| `README.md:110`: enter an amount, chain, token, NEAR account | Entered 0.01, USDC on Base and the documented recipient `zecret-3f109eea1e.testnet`. Displayed limits 0.001–0.10 ZEC and explicit mainnet-preview warning. Price line: minimum 0.2795 USDC on Base; expected 0.2852. Live quote HTTP 201. | Confirmed. The chosen destination chain is clearly not the testnet delivery chain. |
| `README.md:111`: save recovery and continue | Download event `zecret-recovery-ced458ba.json`; payment screen requested 0.01 ZEC, showed a QR and `utest1d5vxyh0ttfsk7nafc6sgny9rnrk5ukf3j7kn8n23hfmadcg2qjjzgnvhpth2h9uzg9nndkc75fzcf4njfu2whenz2ugptzps856nkd44`. Named test USDC and the NEAR recipient. | Confirmed for presentation. Recovery restore with this real unfunded wallet is **not checked**. |
| `README.md:112`: pay the exact amount | **Not checked.** No funded review wallet was available; no existing private wallets or keys were accessed. Native payer build and new faucet funding were not performed. | Not confirmed by a new swap. |
| `README.md:113`: wait for delivery | **Not checked** in a new funded swap. The three historical NEAR transactions were independently verified below. | Not confirmed by a new swap. |

Prerequisites at `README.md:93–103`: faucet quota, wallet-app Ironwood compatibility and a fresh token registration are **not checked**. The existing recipient was used only to inspect the form. No registration, faucet claim or transaction was submitted. The wallet creation instructions are incomplete for a fresh reader: `docs/TESTNET.md:86` assumes pre-existing folders and `./bin/zecret`, and its faucet step precedes wallet creation.

### 3. Local chains

| Exact documented command | Run / result |
|---|---|
| `./scripts/setup-env.sh` | **Not checked / not run.** It installs targets, downloads a Linux-only executable and pulls Docker images; host architecture and sandbox restrictions prevent the documented environment. The script has no platform guard and does not install the required compiler version. |
| `source .tools/env.sh` | **Not checked / not run.** No successful setup generated this file. |
| `python3 -I harness/run.py s1` | **Not checked / not run.** Requires the unavailable Linux sandbox and Docker access. It also rebuilds in restricted source directories and overwrites evidence (`README.md:189`). |
| `python3 -I harness/run.py` | **Not checked / not run.** Same blockers; the stored 21 PASS results are historical evidence only. |
| `node harness/browser/run.mjs` | **Not checked / not run.** Requires the same chains and a compiled browser engine. This is distinct from the simulated website suite. |

### 4. Read-only deployment checks

Executed the complete shell block at `README.md:199–206`, extracted verbatim, with `bash` and curl against `https://rpc.testnet.fastnear.com`. It completed successfully, including `base64 -w0` on this host. Output exactly matched:

```text
DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1
[]
Claimed 3000000 usdc.zecret-3f109eea1e.testnet a published: True b: None
```

Thus the current escrow hash, empty access-key list and recorded native swap state are independently confirmed at finality. A fresh local wasm hash comparison is **not checked**.

The subsequent commands at `README.md:221–226`—escrow wasm build, test-USDC wasm build, deployment tool build and `zecret-deploy verify --config configs/testnet.toml`—are **not checked / not run**. No wasm target is installed, the installed compiler is below the locked NEAR dependencies' minimum, and no setup completed. This is not a claim that the deployment tool passed.

### 5. Other test suites

| Documented command | Actual execution and result |
|---|---|
| `cargo test --lib` in the escrow | Attempted first with `--offline --locked` and an external target directory: missing cached `near-sdk`. Retried online with `--locked`, temporary `CARGO_HOME` and `CARGO_TARGET_DIR`: stopped before compilation because Rust 1.94.0 is unsupported; near-sdk 5.30.1, near-workspaces 0.24.0 and other locked dependencies require Rust 1.95. No test executed. The unmodified command without these isolation flags is **not checked**. |
| `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox -- --test-threads 1` | **Not checked / not run:** compiler, wasm target and Linux sandbox blockers. |
| `./fetch-usdc-wasm.sh` | **Not checked / not run:** writes under `contracts/`, which this review must not edit. No USDC wasm was fetched for this run. |
| `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test usdc -- --test-threads 1` | **Not checked / not run:** same blockers and no fetched wasm. Do not infer eight passes from the source's early-return skip behavior. |
| Intents library `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test` | Attempted `cargo test --offline --locked` with external target storage; dependency resolution failed: locked Tokio 1.54.1 absent (cached candidate 1.50.0). No tests executed. Retried online with `--locked` and temporary Cargo directories: compiler gate failed because near-jsonrpc-client 0.23.0 and near-workspaces 0.24.0 require Rust 1.95. All unit/recorded/sandbox results are **not checked** successfully. |
| Website `npm install` | Ran on the temporary copy; installed 241 packages. npm reported 10 moderate dependency advisories; their applicability is **not checked**, and no dependency changes were made. |
| Website `npm test` | **88 passed in 9 files.** Initial temporary-copy run had 87 passes and one ENOENT because the sibling deployment JSON was missing; after copying that existing fixture, all passed. An accidental invocation from the repository root also failed because there is no root package.json; it is not a project test failure. |
| Website `npm run e2e` | **69 passed, 5 failed (3.7 minutes)** after installing pinned Chromium v1243 and granting localhost access. All five failures wait for `The maker could not be verified` but see `The swap engine did not load`: missing `/engine/zecret_wasm.js`. A clean source copy has no generated engine; Vite does not build it. |

### Chromium failures from the clean copy

All five hit `Test timeout of 60000ms exceeded` / `expect(locator).toBeVisible()` while expecting the maker-verification heading. The observed failure detail is `The WebAssembly engine did not load: Failed to fetch dynamically imported module: http://localhost:4173/engine/zecret_wasm.js`. These are a documented-setup gap, not evidence of a broken hosted engine or a Chromium-only assertion.

- Desktop: `web/e2e/landing.spec.ts:61`, testnet banner and failed maker check (assertion line 72).
- Desktop, 320 px: `web/e2e/responsive.spec.ts:81`, error states with long unbroken messages (assertion line 91).
- Desktop, 390 px: the same responsive test and assertion.
- Phone: `web/e2e/landing.spec.ts:61`, the same maker-check test.
- Screenshots: `web/e2e/screenshots.spec.ts:41`, maker check failed (assertion line 45).

`web/README.md:59–65` separately documents the missing wasm build. The root README's website instructions do not include it. No generated engine was borrowed from the hosted site to manufacture a clean-build pass.

## Status table verification

Each row of `README.md:58–67` is accounted for. File references identify reviewed source and evidence at the source commit; historical results are distinguished from fresh execution.

| README row | Verdict | Evidence and limits |
|---|---|---|
| Escrow contract | **Confirmed** for recorded test inventory and current keyless deployment; test passes not freshly confirmed | Exactly 57 `#[test]` annotations in `contracts/near-escrow/src/tests.rs`, 6 sandbox tests and 8 USDC tests in their respective files. `docs/security/fixes-2.md:139–141` records those passes. `contracts/near-escrow/src/lib.rs:356–358` makes initialization private; `:619–672` separates terminal claim/refund credit paths. Current RPC matches `docs/DEPLOY.md:33–45`. Rebuild and tests **not checked** successfully. |
| Native maker and taker | **Confirmed** as repository evidence; fresh execution **not confirmed** | `harness/RESULTS.md:7–27` lists 21 PASS scenarios, including the explicit liveness-assumption violation at line 17. `harness/run.py:176–182` builds real executables with test misbehavior enabled. `taker.rs:606–635` builds an encrypted RFQ; `maker.rs:319–394` advances deadline work before quote work. Local chains **not checked**. |
| Browser taker | **Confirmed** for implementation, historical five scenarios and fresh initialization | `web/src/engine/index.ts:9–13` selects real/simulated workers; `web/src/engine/real/realEngine.ts:356–360` constructs `WasmTaker`. `harness/BROWSER_RESULTS.md:7–11` records all five passes; `:19–31` describes balance/chain checks. Current hosted payment-screen flow works; new funded browser swap **not checked**. |
| Three public swaps | **Confirmed** as three recorded happy-path completions, partially confirmed independently | `docs/TESTNET.md:20–25`, `:340–356`, `:391–410`; `testnet/EVIDENCE.json:27–36`; hosted result files `:6–31` record completion checks. Three transactions from the 0.05 run independently pass. Durations and Zcash transactions **not checked** afresh; the 0.01 post-fix transactions are explicitly unaudited in `docs/TESTNET.md:409`. |
| Website | **Contradicted** if read as 74 reproducible passes from the stated clean-clone commands; hosting and unit count confirmed | Fresh hosted observations above; `docs/WEB.md:261–273` names suite coverage and counts. Fresh Chromium outcome is in the execution table. Firefox/WebKit are outside this screening and **not checked** here. |
| Maker and relay hosted | **Not confirmed** for all operational details; reachable relay confirmed | Fresh `OPTIONS /execute-intents` on `https://zecret-maker-production.up.railway.app` returned 204, CORS `*`, and Railway response headers. `docs/HOSTING.md:52–73` and `docs/TESTNET.md:303–324` describe the service and persistent volume. Key provenance, volume persistence, current maker solvency and daemon health **not checked** through provider access. |
| Mainnet Intents exit not verified | **Confirmed** as a disclosed evidence limit | `docs/TESTNET.md:35–37` distinguishes the stand-in and direct exit. `docs/INTENTS.md:235–244` warns API acceptance is not settlement. Independent hosted exit is a testnet `ft_withdraw`, not cross-chain delivery. Mainnet exit and current unauthenticated generate/submit API behavior **not checked**. |
| Mainnet deployment not done | **Not confirmed** as a universal absence claim | No mainnet deployment evidence identified; `docs/DEPLOY.md:359` says the tool has not run there. `:145–151` supports the approximately 6.58 NEAR estimate. An exhaustive mainnet search and fresh costs are **not checked**. |
| Wallet apps not tested | **Confirmed** as the documented coverage limit | `docs/TESTNET.md:335–336` identifies the native payer; README explicitly disclaims wallet apps. No app payment was performed in this review. |
| Security reviews, every finding fixed | **Not confirmed** as an unqualified security conclusion | Three reviews and fix reports exist. `fixes-1.md:10–21`, `fixes-2.md:24–32`, `fixes-2-taker.md:11–34`, `fixes-3.md:10–24` map fixes and exceptions. Source spot checks find caller-bound storage, two-step payout, deadline recheck and relay restrictions. All fixes were not independently regression-tested here. Residual risks and untested hardening remain (`fixes-2.md:111`, `fixes-3.md:247`). |

## Independent verification of the three Hosted swap transactions

Used JSON-RPC method **`tx`**, not explorer pages or copied status text, against `https://rpc.testnet.fastnear.com` on 2026-11-13. Request shape:

```json
{"jsonrpc":"2.0","id":1,"method":"tx","params":{"tx_hash":"<hash>","sender_account_id":"<signer>","wait_until":"FINAL"}}
```

All three returned `final_execution_status: FINAL`, top-level `status: {"SuccessValue":""}`, and zero `Failure` entries across transaction and receipt outcomes.

| Source | Hash | Independent result |
|---|---|---|
| `docs/TESTNET.md:354` | `8uh57oHrWRxcLDRHHDGnzXs9ayPNuyoP3DfX1TqR4Rti` | Signer `aa30f22c8cb6c2361984ffb2d9094fcc6c967e2956a786f340ec27b1fb5a843e`; receiver `escrow2.zecret-3f109eea1e.testnet`; action `commit_claim`; locked event at 1794570130; 3 outcomes. |
| `docs/TESTNET.md:355` | `D4gdMsdEYku88YMyHa9j5DKTVT9wHGo77mnM1CEEswA6` | Same taker signer and escrow receiver; action `claim`; claimed event publishes `a`; 3 outcomes. Both escrow calls name swap `7e992f086d5d40d4e3f2fcea183f92ba7d779ae0190b955a24f92d7664a44531`. |
| `docs/TESTNET.md:356` | `DAKZAsvRKaN9hb6JJTUhLThtVgxpx1PbCccrLsDiXxvT` | Maker signer `maker.zecret-3f109eea1e.testnet`; receiver `intents.zecret-3f109eea1e.testnet`; action `execute_intents`. Inner signed `ft_withdraw` names that taker, token `usdc.zecret-3f109eea1e.testnet`, amount `1487700`, destination `zecret-3f109eea1e.testnet`. NEP-141 transfer event confirms that amount moved from the Verifier to the destination. 7 outcomes. |

These checks support successful NEAR-side claim and test-token delivery. They do not independently prove Zcash funding/sweep, elapsed browser time, mainnet behavior or the browser's private signing implementation. Those are **not checked** by a `tx` response.

## README statements to tighten

This is the complete list of issues identified in this screening's full README pass, including claims that remain dependent on historical or external evidence. It is not a claim to have disproved every unverified statement. Suggested sentences are proposed replacements only; README was not edited.

| README line(s) | Issue | Suggested replacement sentence |
|---|---|---|
| 3–10, 299–314, 574 | Present-tense cross-chain delivery overstates the deployed testnet product. | “The deployed testnet app swaps shielded testnet ZEC for test USDC on NEAR; cross-chain delivery through 1Click is implemented and sandbox-tested but has not been completed with real funds.” |
| 13, 77, 109 | Hosted URL differs from the requested canonical domain. The older URL itself is **not checked** here. | “Open https://zecret.chrismg.dev for the hosted testnet app.” |
| 17–19 | Universal claim about every route from shielded Zcash is attributed to the paper, not independently substantiated. Paper quotation and ecosystem completeness **not checked**. | “The cited paper motivates this design by describing transparent intermediaries in existing exit routes; this repository does not establish an exhaustive comparison of current routes.” |
| 20–23 | Keeping funds shielded does not establish unlinkability or unconditional atomicity. Token identifier history is **not checked** against the dated API snapshot. | “The recorded swaps use shielded transactions; safety also depends on timely participation, the configured chain services and the exit provider.” |
| 29, 513, 520–524, 564 | Event dates, rules, provenance date, remote branch/tag policy and absence of a template are historical/external claims **not checked** independently. | “The project records its event dates, prior-work provenance and submission-rule interpretation in BASELINE.md and the submission notes; eligibility and external provenance have not been independently verified here.” |
| 40, 418 | “Two exits” can be mistaken for direct token delivery; claim/refund only create credits. | “Claim and refund publish a key half and create a credit; a separate withdraw call delivers the tokens.” |
| 43, 419 | Verifier named as `intents.near` without testnet qualification. | “Mainnet uses intents.near; this testnet deployment uses intents.zecret-3f109eea1e.testnet as a stand-in.” |
| 46 | “Any browser” is broader than the tested support and ignores deadlines. | “A recovery file can resume a swap in a compatible browser, subject to chain availability and the remaining protocol deadlines.” |
| 58–63, 178–181, 268, 281–285 | Historical PASS counts can be read as guarantees for a fresh checkout; measured run times are machine-specific. | “These are the recorded results for the linked runs; a fresh run requires the documented platform, compiler, wasm artifacts and external services.” |
| 61, 498–499 | “Happy path only” omits the documented hosted pre-funding refund; it is fair only when restricted to completed swaps or escrow-refund/punish paths. | “Three swaps completed on public testnets; a separate below-minimum quote run returned the deposit before shared-address funding, while escrow-refund and punish paths were tested only locally.” |
| 67, 460–466 | “Every finding is fixed” conflates fix reports with independent re-review and can hide remaining validation gaps. | “Fix reports address the confirmed findings from three reviews; accepted risks, untested changes and independent revalidation gaps remain listed below.” |
| 93–100 | Faucet quota and working wallet compatibility are time-dependent; fresh payer setup is not self-contained. | “Use an Ironwood-capable testnet payer; the linked run used the project's CLI, and the fresh-wallet build, initialization, faucet and payment commands should be supplied as one runnable sequence.” |
| 109, 116–117, 363 | Download, preparation, proof and online-duration estimates are not bounds. | “Recorded download and compute times are examples; keep the tab open throughout the claim and follow its current deadline warning.” |
| 121–134 | Setup installs wasm targets but not a sufficiently new compiler, and downloads a Linux binary without checking the host. | “Use Linux x86-64 with Docker and Rust 1.97.0 already installed; the setup script adds wasm targets but does not install or select that Rust version.” |
| 146 | “Only through encrypted memos and the escrow” excludes the NEAR stipend and testnet exit relay. | “Swap negotiation uses encrypted memos and escrow state; the maker also sends the NEAR stipend and relays the direct testnet exit.” |
| 229 | “It ends with ALL CHECKS PASSED” is conditional. | “A successful verification prints ALL CHECKS PASSED; build and RPC failures must be reported separately.” |
| 269–271 | USDC tests return early when the wasm is absent, so a green test count can include unexecuted cases. | “Only count the USDC suite as executed after the pinned wasm download succeeds and the run contains no SKIPPED messages.” |
| 289–294 | The suite is not entirely simulated: five cases load the real engine, which is absent from a clean checkout. | “Before the full browser suite, build the WebAssembly engine as described in web/README.md; the maker-verification tests need it even though they mock NEAR.” |
| 291 | `npm install` is less reproducible than the lockfile installation used by deployment. | “Run npm ci from web/ using the documented Node version, then npm test and npm run e2e.” |
| 316–318 | Maker disappearance does not itself publish `b`; punish-path proceeds still need withdrawal gas. | “After a maker refund publishes b, the taker can recover ZEC; otherwise, after the punish deadline it can claim a USDC credit, whose delivery still requires a funded NEAR caller.” |
| 328–332, 345–348, 365–369 | Gas figures and timings are dated measurements; fresh benchmarks **not checked**. | “These gas and timing figures were measured in the linked runs and are not guaranteed maxima.” |
| 343–344, 575 | No gas trail is narrower than no linkage; stipend, amount, timing, RPC and destination remain observable. | “The stipend avoids funding the swap key from the user's existing NEAR account; it does not prevent timing, network or destination-address correlation.” |
| 374–377 | At-most-once payout is supported for tested interleavings, not an unconditional theorem about changing external contracts. | “The two-step payout passed the linked pause, blocklist and callback-race tests with the recorded Verifier and USDC builds; residual stuck-token risks remain.” |
| 383–385 | 230 tokens, 39 chains and 35 supported chain validators are dated inventory claims; exact current counts **not checked**. | “The app filters the current 1Click token list to chains with supported address validation; counts and route availability can change.” |
| 400–401, 415 | Worker separation is not a security boundary against a compromised same-origin page; the page receives exported recovery text (`web/src/screens/Pay.tsx:12–15`). | “The worker checks generated intents against the approved parameters; the page still handles approvals and recovery downloads, so same-origin script compromise remains a key risk.” |
| 531 | Protocol-rule equivalence is **not checked** formally, and the removed claim relayer should not be confused with the existing testnet exit relay. | “The baseline and new components are mapped here; use the linked comparison for exact changes, and distinguish removed claim relaying from the testnet exit relay.” |
| 463–466 | A negative finding from prior reviews is not proof of safety. | “The prior reports did not confirm third-party theft within their stated scope and execution limits.” |
| 497, 502–503 | Mainnet absence and absence of third-party testnet services are not exhaustively established by this repository. | “No mainnet deployment or completed funded 1Click exit is recorded here; this deployment uses its own testnet token and Verifier.” |
| 505 | “One swap per browser” suggests a global limit across profiles/devices. | “The app supports one active swap per browser storage context and coordinates tabs with a Web Lock.” |
| 506–509, 544–559 | Measurements outside the tree and external dependency/source-version attribution cannot be reproduced solely from the README; external provenance **not checked**. | “The linked experiments and lockfiles identify recorded versions and measurements; unavailable experiment code and external-source correspondence are not independently reproducible from this checkout.” |

A local `git diff --name-only 2f8e2f7 HEAD` restricted to `node/`, `contracts/src`, `contracts/test`, `contracts/audit`, `formal/`, `app/` and `design-system/` returned no changes, confirming the listed baseline directories are unchanged. The remote-tracking baseline branch exists locally; the live remote was **not checked** for that branch.

The visible landing page also still says **two** completed testnet swaps. Update it to the three recorded completions when application changes are next allowed; this review did not modify it.

## Preliminary score and highest-impact changes

**Preliminary score: 72/100** for a hackathon screening, not a security score or an official judging rubric: hosted usability 18/20, independent feature evidence 22/30, reproducibility 14/25, claim precision and risk disclosure 18/25. The strongest evidence is the independently successful NEAR settlement plus the working hosted form. The largest gaps are clean-clone test preparation, a newly funded swap from this machine, and unverified cross-chain delivery.

The five changes most likely to raise this score:

1. **Make the clean-clone path self-contained.** Pin/select Rust and Node, reject unsupported hosts early, install Playwright browsers, build the wasm prerequisite before the full website suite, and make missing USDC wasm fail or explicitly skip rather than look like a pass.
2. **Publish one reproducible public-testnet walkthrough from an empty wallet.** Include wallet folders, binary placement, faucet funding, token registration and exact payment commands; independently verify all transactions of the 0.01 fixed-price run and exercise real escrow refund/punish paths.
3. **Align the product claims and landing status with measured behavior.** Lead with shielded testnet ZEC to test USDC on NEAR; label cross-chain delivery as unverified, replace unconditional recovery/browser/privacy claims, and use the canonical domain.
4. **Prove the post-claim exit when the maker stops.** Demonstrate a funded independent caller performing the advertised withdrawal, with the UI retaining recovery information until delivery; document the residual Verifier/USDC failure case prominently.
5. **Add independently reproducible browser and payer coverage.** Provide Firefox/WebKit projects and at least one actual Ironwood-capable wallet-app payment, with exact versions and raw run results tied to a source commit.
