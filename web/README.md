# Cloakroom Intents website

The page where a user swaps shielded ZEC for a token on another chain. It is a static
site (Vite, React, TypeScript) with no backend. The swap engine runs in a Web Worker
behind the interface in [`ENGINE_API.md`](ENGINE_API.md): the real engine (Rust compiled
to WebAssembly, `crates/zecret-wasm`) when the page has a network configuration, and a
simulated engine (the demo, no money, no network) when it has none. Screens, states and
design decisions: [`../docs/WEB.md`](../docs/WEB.md).

## Pages and routes

| URL | Page |
|---|---|
| `/` | The landing page: what Cloakroom Intents does, who holds the money, what is private, failures, status, questions. It loads no engine, no maker directory and no network settings. |
| `/swap` | The swap app. The engine (worker and WebAssembly) starts only here. Vercel serves it with a rewrite to `index.html` (`vercel.json`; the headers of `/(.*)` apply to it as to every path). |
| `/?demo=1`, `/?config=testnet`, `/?speed=20`, … | The swap app as well: a link to `/` that names an app setting (`demo`, `config`, `speed`, `fail`, `pause`, `cpu`, `directory`) keeps working and keeps its URL (`src/app/route.ts`). |

The two pages move with the History API, so going back to the landing page in the middle
of a swap does not stop the engine. The app's code is a separate chunk that the landing
page loads only when the user opens the app. While the engine reports a swap, the app sets
`zecret.hasSwap` in `localStorage`; the landing page then shows "This browser holds a swap"
with a link to it.

**Banners.** The test-network banner and the demo banner are one line ("Testnet. Test
money only.", "Demo. No real money moves.") with a "Details" button (`aria-expanded`). The
details hold the network label, every `honesty` line of the network configuration, the
demo's speed and failure, and "Demo controls".

**Start failures.** `src/app/failureCopy.ts` picks the title and the next step from the
cause: refused or missing network settings, maker directory not loaded, maker not verified
(its signed entry failed the check), maker not checked (NEAR did not answer), engine
download, repeated engine crashes, and only then the generic "could not start" with the
hint that the browser may block workers, WebAssembly or storage. The engine's message is
shown below as "Technical detail", wrapped, with a copy button.

## Commands

Node 22 (`.nvmrc`). From a clean clone, in this order:

```sh
npm ci                                  # the locked dependencies
npx playwright install chromium         # skip if Chromium for Playwright 1.63.0 is installed
../crates/zecret-wasm/build.sh          # the real engine, into public/engine/ (see "The real engine")
npm test                                # unit tests (vitest)
npm run e2e                             # end-to-end tests, desktop and phone (Playwright, --trace off)
```

Seven end-to-end tests (the failed maker check on testnet, on the landing, responsive and
screenshot specs) load the real engine. Without `public/engine/` they fail at once with a
message that names `crates/zecret-wasm/build.sh`. All other tests use the simulated engine.

Other commands:

```sh
npm run dev                 # http://localhost:5173
npm run typecheck           # tsc -b
npm run lint                # eslint
npm run build               # static site in dist/
npm run screenshots         # screenshots of the key screens into screenshots/
npm run check               # type-check, lint, unit tests, build and end-to-end tests
```

`npm run e2e` builds the site and serves it with `vite preview` on port 4173. If
Playwright cannot find Chromium, set `PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`.
The tests mock the 1Click API (`e2e/oneclick-mock.ts`, built from the real token list of
2026-11-13), so they need no network. `ZECRET_BROWSERS=all npm run e2e` adds the desktop
tests in Firefox and WebKit (projects `firefox` and `webkit`; install them with
`npx playwright install firefox webkit`). They have not been run: the build machine has
only Chromium.

## The real engine

Build its WebAssembly into `public/engine/` (not in git) once, and after every change in
`crates/`:

```sh
cd ..                                   # repository root
crates/zecret-wasm/build.sh             # needs wasm-bindgen-cli 0.2.129 and clang
```

`rust-toolchain.toml` selects Rust 1.97.0 with the wasm32 target. Install wasm-bindgen
with `cargo install wasm-bindgen-cli --version =0.2.129 --locked`. The C parts of the
build need `clang` and `llvm-ar`; where `cc` is not clang, set
`CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar`
(`deploy/web/vercel-build.sh` does this).

Then the page runs it with `?config=local` or `?config=testnet`
(`public/config/<name>.json`; `src/lib/netconfig.ts` describes the format), or with
`VITE_ZECRET_CONFIG=<name or URL>` at build time. `?demo=1` always runs the demo.

A link cannot load other network settings (security review 3, NV3-01): a production build
takes only those two names from `?config=`. In development (`npm run dev`), or in a build
made with `VITE_ZECRET_ALLOW_CONFIG_URL=1`, `?config=` may name any file or URL, and the
page shows a red warning that names it. A mainnet configuration must use https endpoints
and the escrow, code hash, token and Verifier built into the bundle (`MAINNET_ANCHORS`;
the escrow comes from `VITE_MAINNET_ESCROW` and `VITE_MAINNET_ESCROW_CODE_HASH` at build
time, and without them the page refuses mainnet).

One tab runs the engine at a time (a Web Lock). A second tab shows "This swap is open in
another tab" and continues by itself when the first one closes.

### Against the local network (Zebra regtest and a NEAR sandbox)

```sh
cd ..                                   # repository root
source .tools/env.sh
export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers
node harness/browser/run.mjs --serve
```

This builds the engine, the site and the escrow, starts Zebra regtest (Docker, port 18332),
a CORS proxy for it (18330), a NEAR sandbox with the escrow, wrap.near and intents.near
(3130), a maker with its relay (3410) and the site with the headers of `vercel.json`
(18380). It prints the page URL (`http://127.0.0.1:18380/?config=/run/local/config.json`,
also served as `?config=local`), a NEAR account registered for wNEAR to receive the tokens,
a funded native wallet and the command that pays the QR address from it
(`zecret pay --dir … --to <address> --zats <amount>`). Zcash blocks are mined every few
seconds. Stop with Ctrl-C.

The runner must build the site with `VITE_ZECRET_ALLOW_CONFIG_URL=1`, because it opens
`?config=/run/<scenario>/config.json`. With `SKIP_BUILD=1`, `dist/` must come from such a
build (`VITE_ZECRET_ALLOW_CONFIG_URL=1 npx vite build`); `npm run build` and `npm run e2e`
write a `dist/` without it.

The automatic check runs five scenarios through the page the same way:
`node harness/browser/run.mjs` (or `a b c d e`); results in `../harness/BROWSER_RESULTS.md`.

### Against Zcash testnet and NEAR testnet

`public/config/testnet.json` uses `https://zcash-testnet.chainsafe.dev` (gRPC-web),
`https://rpc.testnet.fastnear.com`, the escrow, test USDC and Verifier of
`../deploy/deployment.testnet.json`, and the relay of the hosted testnet maker
(`https://zecret-maker-production.up.railway.app/execute-intents`).
`public/config/testnet-directory.json` holds that maker's signed entry. The hosted site
(https://cloakroom.chrismg.dev/swap, also https://zecret-chrismgs-projects.vercel.app/swap) uses these files; see
[`../docs/HOSTING.md`](../docs/HOSTING.md).

A swap needs testnet ZEC in a wallet that can pay a unified address, and a NEAR testnet
account registered with the test USDC token (`storage_deposit`).

To run your own maker instead:

1. start a maker on testnet with testnet ZEC for its quote fees:
   `zecret maker --dir M --relay 127.0.0.1:3410` (`../crates/zecret-node/README.md`, "NEAR
   testnet and Zcash testnet");
2. write its directory: `node ../harness/browser/make-directory.mjs M <USDC per ZEC> testnet >
   public/config/testnet-directory.json`;
3. set `exit.relayUrl` in `public/config/testnet.json` to `http://127.0.0.1:3410/execute-intents`;
4. serve the site: `npm run build && npx vite preview --port 4173`, then open
   `http://localhost:4173/?config=testnet`.

A relay at an `http:` address works only from a site served without the headers of
`vercel.json` (as in step 4): the deployed CSP allows no `http:` host. A deployed site
needs the relay at a public https URL, in `exit.relayUrl` and in `connect-src` of
`vercel.json`; `tests/unit/csp.test.ts` fails until both agree.

The relay pays `execute_intents` only for one `ft_withdraw` of the maker's token, from the
taker account of a swap that paid out, at most 3 times per swap and 6 requests a minute per
address. It refuses to start on mainnet (`crates/zecret-node/src/relay.rs`).

## Trying it by hand

Open `http://localhost:5173/?speed=20` (or `/swap?speed=20`). Enter an amount, keep USDC
on Base or pick a token, paste an address (for example
`0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed`), continue, save the recovery file, and press
"Simulate the payment". "Details", then "Demo controls" in the banner switch the speed and
the failure cases, skip a wait, and simulate a cleared browser. All URL parameters are in `ENGINE_API.md`, "The simulated engine".

Prices on the form are live: the page calls `https://1click.chaindefuser.com` from the
browser (no key; CORS is open).

## Configuration

| Setting | Default | How to change |
|---|---|---|
| Maker directory | `/maker-directory.json` (a sample for Zcash testnet, in `public/`) | `VITE_DIRECTORY_URL` at build time, or `?directory=<url>` on the page (demo only; with a network configuration the page uses the configuration's directory) |
| 1Click API | `https://1click.chaindefuser.com` | `oneClickUrl` in the network configuration (`public/config/*.json`): an http(s) URL, or a path on this site for the proxy (`../docs/ONECLICK_PROXY.md`) |
| Engine | simulated (demo) | a network configuration: `?config=local`, `?config=testnet`, or `VITE_ZECRET_CONFIG` at build time (any other `?config=` only in development, see above); `?demo=1` forces the demo |
| Mainnet escrow | none (mainnet refused) | `VITE_MAINNET_ESCROW`, `VITE_MAINNET_ESCROW_CODE_HASH` at build time |

The directory format is `MakerDirectory` in `src/lib/directory.ts`: maker name, Zcash
unified address, request key, NEAR account, escrow contract, indicative rate in USDC per
ZEC, limits, premium (bps and minimum) and the Zcash fee per send. The sample describes
the testnet maker of `../deploy/maker/README.md` (940 USDC per ZEC, 0.01 to 2 ZEC, a flat
premium of 450,000 zats) with the accounts of `../deploy/deployment.testnet.json`; its
Zcash address and request key are placeholders. The page rejects a
file that does not match, with the reason.

## Deploying

The site is hosted on Vercel with the project root `web/` ([`../docs/HOSTING.md`](../docs/HOSTING.md)).
`vercel.json` sets:

- the install: `npm ci` (the locked versions);
- the build: `bash ../deploy/web/vercel-build.sh`. It builds the WebAssembly engine, then
  runs `npm run build` with `VITE_ZECRET_CONFIG=testnet` (output `dist/`);
- the ignored build step: `bash ../deploy/web/vercel-ignore.sh`;
- the headers: a Content-Security-Policy that allows exactly the hosts of the testnet
  configuration and 1Click (`../docs/WEB.md`, "Headers"), and `Referrer-Policy:
  no-referrer`, so explorer links do not tell explorers where the user came from.

`npm run build` alone does not compile the engine. Run `crates/zecret-wasm/build.sh`
first (it needs Rust), or the site runs only the demo.

## Layout

| Path | What |
|---|---|
| `src/engine/` | The engine interface (`api.ts`), transport (`rpc.ts`), reducer, timeline, and the simulated engine (`sim/`) |
| `src/lib/` | Pure code: amounts (`units.ts`), price math (`price.ts`), maker directory, 1Click client, ZIP 321 URIs, address checks for 35 chains and Zcash unified addresses (`address/`) |
| `src/app/` | Page state: boot and engine events (`useZecret.ts`), live price, exit control, stage copy, URL settings |
| `src/screens/`, `src/ui/` | Screens and components |
| `src/styles/` | Design tokens (light and dark) and styles. The look follows the style reference measured in `design-system/vaynex-notes.md` |
| `src/assets/fonts/` | Hanken Grotesk, Inter, Instrument Serif italic and Geist Mono: Latin woff2 subsets with their SIL OFL licences |
| `design-system/vaynex-notes.md` | Measured type, colour, shape, depth, layout and motion of the style reference |
| `tests/unit/` | Unit tests |
| `e2e/` | Playwright tests, the 1Click mock and its fixture |
| `screenshots/` | Key screens, light and dark, desktop and phone (`npm run screenshots`); `*@2x.png`: landing page and swap form, light and dark, 390 and 1440 px wide, full page |

## Test vectors

The unified-address tests in `tests/unit/address.test.ts` use addresses made with
`zcash_address` 0.14.0-pre.1 (the crate the engine uses), with receivers filled with
constant bytes (Orchard `0x07`, Sapling `0x09`, P2PKH `0x03`). The page's encoder
produces the same strings byte for byte.
