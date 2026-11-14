# Merging `mainnet-ready`

How the main session merges the branch `mainnet-ready` into `main`. The branch prepares
mainnet (`docs/MAINNET.md`). It does not deploy anything and does not change what the
testnet services do. Its base is `main` at `1ab066a` (rebased on 2026-11-14 03:40 UTC).

## Files

| Area | Files | Kind |
|---|---|---|
| Zcash wallet on mainnet | `crates/zecret-core/src/wallet.rs` (enum `Net` gets `Main`; `MAINNET_NU7_HEIGHT`; one test), `crates/zecret-core/src/engine.rs` (`wallet_for`: `network` `main` with lightwalletd) | code |
| Mainnet templates and their tests | `crates/zecret-node/config/mainnet-maker.json`, `mainnet-taker.json`, `crates/zecret-node/src/mainnet_config.rs`, `crates/zecret-node/src/lib.rs` (one `mod` line) | new |
| Dry run | `deploy/tool/src/plan.rs` (new), `deploy/tool/src/main.rs` (command `plan`, options `--owner`, `--maker-config`, `--expect-escrow-hash`, guard, tests) | code |
| Hosted maker | `deploy/maker/railway/start.sh` (mainnet mode), `deploy/maker/railway/test-start.sh` (new), `deploy/maker/railway/Dockerfile` (comment only) | code |
| Website | `web/src/lib/netconfig.ts`, `web/src/screens/Shell.tsx`, `Landing.tsx`, `Pay.tsx`, `web/src/App.tsx`, `web/src/app/failureCopy.ts`, `web/src/engine/real/realEngine.ts`, `web/src/styles/app.css`, `web/src/vite-env.d.ts`, `web/public/config/mainnet.json` (new), `web/vercel.json` (two CSP hosts) | code |
| Website tests | `web/tests/unit/mainnet_switch.test.ts` (new), `csp.test.ts`, `config_override.test.ts`, `netconfig.test.ts`, `web/e2e/mainnet-off.spec.ts` (new), `web/e2e/mainnet/mainnet-on.spec.ts` (new), `web/e2e/fixtures/mainnet-directory.json` (new), `web/playwright.mainnet.config.ts` (new), `web/playwright.config.ts` (ignores `e2e/mainnet/`), `web/tsconfig.node.json`, `web/.gitignore` | tests |
| Build and publish scripts | `deploy/web/vercel-build.sh` (a mainnet build stops early without its variables), `deploy/web/directory-from-entry.mjs` (mainnet entries) | code |
| Scripts | `scripts/mainnet-preflight.sh`, `scripts/mainnet-browser-check.mjs`, `scripts/zcash-block-intervals.py` (new) | new |
| Docs | `docs/MAINNET.md`, `docs/MAINNET_MERGE.md` (new); short pointers in `docs/DEPLOY.md`, `docs/HOSTING.md`, `deploy/README.md`, `deploy/maker/README.md`; four decision rows in `docs/DESIGN.md` | docs |

## Conflicts to expect

With `main` after `1ab066a`: none known. The branch was rebased on it; the only conflict
was `deploy/tool/src/main.rs` against the `fund` command, resolved by keeping both (23
tests pass).

With `escrow-light-payout` (uncommitted work in `/home/user/zecret-lw` at 03:00 UTC,
touching `contracts/near-escrow`):

- **No file of the escrow is touched here.** Neither is `crates/zecret-core/src/near/`,
  `taker.rs`, `maker.rs`, `web/src/engine/real/payoutCheck.ts` or `deploy/tool/src/e2e.rs`.
- Likely text conflicts, all in docs: `docs/DESIGN.md` (both add rows at the end of the
  "Decided" table: keep both, in date order) and `docs/DEPLOY.md` (this branch adds one
  paragraph at the top of "## Mainnet": keep it, then update the code hash quoted below it
  if the escrow changes).
- `deploy/tool/src/main.rs` if that branch adds a command: keep both; the guard must
  still list `"verify" | "plan"` as read-only.
- Escrow-specific values are inputs, not constants: the escrow wasm and its code hash
  (`--escrow-wasm`, `--expect-escrow-hash`, `ESCROW_HASH`), `VITE_MAINNET_ESCROW` and
  `VITE_MAINNET_ESCROW_CODE_HASH`, the taker template's `escrow_code_hash` placeholder,
  and `"@build"` in `web/public/config/mainnet.json`. Nothing here needs a change when
  the escrow changes.
- Two constants depend on the taker's NEAR calls: `TGAS_CLAIM_PREPAID` (25) and
  `COMMIT_CLAIM_BURNT_YOCTO` in `deploy/tool/src/plan.rs` (the stipend check, gap G6 in
  `docs/MAINNET.md`). If the light payout changes what the taker sends with its stipend
  (for example the taker sends `withdraw`), update them and G6. The dry run's cost table
  reads the wasm size at run time.

## What a merge does to the live testnet services

- **Railway** rebuilds the testnet maker when a file under `deploy/maker/` or `crates/`
  changes. This branch changes both (`start.sh`, `wallet.rs`, `engine.rs`). So the merge
  restarts the hosted testnet maker. Merge between two soak swaps, or when the log shows
  `open swaps: 0`; the start script keeps all state either way. Also change
  `deploy/maker/RELEASE` in the merge commit if that is the rule then (`STATUS.md`).
- **Vercel** rebuilds the testnet site (files under `web/` and `crates/zecret-core`
  change). The site stays on testnet: its build has no `VITE_ZECRET_ALLOW_MAINNET`.

## Proof that testnet behaviour does not change

Each item names the test that shows it; all passed on this branch (outputs in the final
report of the work and in `docs/MAINNET.md` section 10).

| Part | Unchanged because | Test |
|---|---|---|
| Wallet on testnet | `Net::Test` keeps `TEST_NETWORK`'s heights; `wallet_for` maps `test` as before | `wallet::tests::mainnet_parameters_have_nu7_at_the_height_the_chain_shows` (asserts the testnet heights too); all 44 `zecret-core` unit tests |
| Testnet configs | not edited | `testnet_config::*` (4 tests) |
| Hosted testnet maker | the default mode (no `ZECRET_NETWORK`) runs the image's testnet settings with `--relay 0.0.0.0:8080`, as before | `test-start.sh` cases `testnet-default`, `testnet-refuses-a-mainnet-image` |
| Deploy tool on testnet | only the read-only `plan` is new; `deploy`, `verify`, `e2e`, `fund` unchanged | 23 `zecret-deploy` tests |
| Website default build | `KNOWN_CONFIGS` is still `local`, `testnet`; mainnet needs the build switch; testnet copy unchanged (each mainnet string is behind `MAINNET_BUILD` or `network === 'mainnet'`); the testnet engine still polls every 25 s | 120 unit tests; the full default Playwright run, 96 passed (the 92 existing tests plus the 4 new refusal and landing tests) |
| CSP | two hosts added to `connect-src` (`https://zcash-mainnet.chainsafe.dev`, `https://rpc.mainnet.fastnear.com`); nothing removed | `csp.test.ts` derives the list from both shipped configs |
| Directory converter | testnet output identical apart from `updatedAt` | compared with `web/public/config/testnet-directory.json` |

## Steps

```sh
cd /home/user/zecret && git fetch origin mainnet-ready
git merge --no-ff origin/mainnet-ready      # resolve docs conflicts as above
cd web && npm ci && npm run typecheck && npm run lint && npm test && npm run build
npx playwright test --trace off e2e/mainnet-off.spec.ts && npx playwright test -c playwright.mainnet.config.ts
cd ../deploy && cargo test -p zecret-deploy
cd ../crates && cargo test -p zecret-core --lib && cargo test -p zecret-node --lib
sh ../deploy/maker/railway/test-start.sh
```

Then push `main` (it restarts the testnet maker; see above). Nothing in the merge touches
mainnet.
