# Cross-browser run

Date: 2026-11-13. Source commit: `ac9c2ff79ae4a400c00312c637d6004c663ae78f`. Branch: `codex/review`.

**Result: both requested commands were attempted, but neither browser ran a test.** Playwright refuses `--browser` when the configuration defines projects. This is the configuration limitation anticipated in the request, not a Firefox or WebKit site failure.

## Environment and unchanged inputs

Host: macOS, arm64; Node `v24.2.0`, npm `11.3.0`; repository-pinned Playwright `1.63.0`.

To obey the restriction against editing `web/`, installation and commands ran from `/private/tmp/zecret-review/web`, a temporary copy of that directory. The sibling deployment JSON used by the suite was copied too. Dependencies, browser downloads and generated outputs stayed outside the repository. No application or Playwright configuration file was edited, including in the copy.

The earlier screening's `npm install` had normalized the temporary lockfile's root `@playwright/test` specifier from `^1.63.0` to `1.63.0`. Before the browser commands, the repository lockfile was restored and `npm ci` was repeated successfully. The restored lockfile SHA-256 matched the repository exactly:

```text
06207a789a10b032c1d71c178cd4d4ca22d059b24b7795a775608edc3ee3e156
```

`package.json` and `playwright.config.ts` also matched their repository copies by SHA-256. No dependency versions were deliberately upgraded.

## What ran

Commands were executed in the temporary `web` directory. The cache flag and browser-path environment variable only redirect downloaded files to temporary storage.

```sh
npm ci --cache /private/tmp/zecret-review/npm-cache
PLAYWRIGHT_BROWSERS_PATH=/private/tmp/zecret-review/browsers npx playwright install firefox webkit
# After restoring the original lockfile, repeat the clean installation:
npm ci --cache /private/tmp/zecret-review/npm-cache
PLAYWRIGHT_BROWSERS_PATH=/private/tmp/zecret-review/browsers npx playwright test --browser=firefox --trace off
PLAYWRIGHT_BROWSERS_PATH=/private/tmp/zecret-review/browsers npx playwright test --browser=webkit --trace off
```

| Step | Actual result |
|---|---|
| `npm ci`, repeated from the original lockfile | Exit 0; 241 packages installed, 242 audited. npm reported 10 moderate advisories. Applicability of those advisories is **not checked**; no audit fix was applied. |
| Firefox installation | Succeeded: Firefox 155.0, Playwright build v1543, macOS arm64. |
| WebKit installation | Succeeded: WebKit 26.6, Playwright build v2359, macOS 26 arm64 package. |
| Firefox test command | Exit 1 during configuration loading; zero tests executed. |
| WebKit test command, run after Firefox | Exit 1 during configuration loading; zero tests executed. |

Both commands produced the same error:

```text
Error: Cannot use --browser option when configuration file defines projects. Specify browserName in the projects instead.
    at new FullConfigInternal (.../node_modules/playwright/lib/common/index.js:486:13)
    at loadConfig (.../node_modules/playwright/lib/common/index.js:1277:22)
    at Object.loadConfigFromFile (.../node_modules/playwright/lib/common/index.js:1490:10)
    at runTests (.../node_modules/playwright/lib/cli/testActions.js:46:18)
    at _Command.<anonymous> (.../node_modules/playwright/lib/program.js:55:7)
```

The installed runner's `lib/common/index.js:485–486` explicitly throws when both a CLI browser override and configured projects are present. This is an actual observed rejection, not an assumption about command-line support.

## Classification and test-level failures

There are **no individual Firefox/WebKit test failures to list**, because configuration loading failed before test execution. Browser rendering, downloads, recovery, tab coordination, responsiveness and engine compatibility are all **not checked** in these browsers.

`web/playwright.config.ts:25–42` defines three projects:

| Project | Browser assumption in source |
|---|---|
| `desktop` | `devices['Desktop Chrome']`, line 29 |
| `phone` | `devices['Pixel 7']`, line 35 |
| `screenshots` | `devices['Desktop Chrome']`, line 40 |

Classification: **test configuration assumes Chromium**. The result is not evidence of a real site bug or a browser-specific assertion failure. Selecting `--project` would select these existing projects, not turn them into Firefox/WebKit projects. No replacement config or edited project definition was used.

The earlier Chromium run is recorded separately in [screening-dry-run.md](screening-dry-run.md): 69 passes and five failures caused by a missing generated WebAssembly module. Those failures were not reproduced in Firefox/WebKit and must not be attributed to either browser. Although most tests use the simulated engine and need no chains, the suite also contains real-engine maker-verification cases (`web/e2e/landing.spec.ts:61`, `web/e2e/responsive.spec.ts:81`, `web/e2e/screenshots.spec.ts:41`); a complete future run needs the build prerequisite in `web/README.md:59–65`.

## Not run

- No Firefox or WebKit test body, browser launch, screenshot or browser trace: **not checked**, because the runner stopped at configuration loading.
- No alternate configuration, source modification or manual browser substitute: **not run** under the requested unchanged-config restriction.
- No chains, funded swaps or hosted-site security probes were needed or run for this task.

To enable a future run, add explicit Firefox/WebKit projects with suitable device options, then run those projects and classify their actual failures. That is a proposed follow-up; this review made no such change.
