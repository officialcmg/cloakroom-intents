# The 1Click proxy and the partner key

The website calls the NEAR Intents 1Click API for the price preview and for the exit
(quote, generate-intent, submit-intent, status). A 1Click partner key (a JWT from
`partners.near-intents.org`) must not be in the browser: anything the browser has is
public. The proxy is a small Vercel function in the website project. It forwards the five
calls the site makes to `https://1click.chaindefuser.com` and adds the key from a server
environment variable. The browser never sees the key.

Status: built and tested on 2026-11-14 (branch `oneclick-proxy`). It has not run on
Vercel yet. The shipped configurations still point at 1Click itself, so the site works as
before until the owner turns the proxy on (below).

## What the key changes

From the 1Click docs, read 2026-11-14:

- <https://docs.near-intents.org/integration/distribution-channels/1click-api/authentication.md>:
  "Unauthenticated requests incur an additional 0.25% (25 basis points) platform fee on
  every non-`ANY_INPUT` quote. Authenticated requests use the partner fee schedule
  (typically 20 bps, or 1 bp on stablecoin and same-asset multichain routes)." The key is
  sent "as `X-API-Key` on API requests. `Authorization: Bearer` is also accepted for the
  same partner JWT."
- <https://docs.near-intents.org/resources/fees.md>: without a key, 25 bp on top of any
  `appFees`; with a key and no `appFees`, 20 bp (1 bp for USDC, USDT, DAI pairs and
  same-asset routes); with a key and `appFees`, a 50/50 split with at least 20 bp (or 1 bp)
  to 1Click.
- The OpenAPI file (`https://1click.chaindefuser.com/docs/v0/openapi.yaml`, version
  0.1.10) lists two schemes for `quote`, `status`, `generate-intent` and `submit-intent`:
  `X-API-Key` ("recommended") and a bearer JWT ("legacy, use X-API-Key instead"). The
  proxy sends `X-API-Key`. The docs keep `Authorization: Bearer` for user-session tokens
  of other endpoints.
- Rate limits: answers without a key carry `x-ratelimit-limit-unauth: 1200` and
  `x-ratelimit-reset-unauth: 60` (1200 requests per 60 s, measured 2026-11-14). The docs
  publish no number for keyed requests of the swap API; they name 429
  `rate-limit-exceeded`. The "1 request every 5 seconds per partner" on the docs is for
  the Explorer API, which the site does not use.
- What the quotes showed without a key on 2026-11-13 and 2026-11-14 (an `appFees` entry
  of 1 bp or 20 bp, no visible extra 25 bp) is in `docs/INTENTS.md` §4. What is charged
  can only be seen with funds.

## How it works

- Files: `web/api/oneclick/v0/{tokens,quote,generate-intent,submit-intent,status}.ts`, one
  Vercel function per call, and `web/server/oneclickProxy.ts`, the shared handler.
- URLs: `/api/oneclick/v0/tokens` (GET), `/api/oneclick/v0/quote` (POST),
  `/api/oneclick/v0/generate-intent` (POST), `/api/oneclick/v0/submit-intent` (POST),
  `/api/oneclick/v0/status?depositAddress=…` (GET). These are the only calls of
  `web/src/lib/oneclick.ts` and `crates/zecret-intents/src/oneclick.rs` that the site
  uses. Any other path under `/api/` has no function and answers 404.
- The configuration value `oneClickUrl` (top level of `web/public/config/*.json`) is the
  base URL for both the price preview and the engine's exit (`oneClickBase` in
  `web/src/lib/netconfig.ts`). It is `https://1click.chaindefuser.com` (the default when
  the file does not name one, and in the demo) or a path on the site, `/api/oneclick`.
  A path is taken on the page's own origin. On mainnet it must be https or a path on the
  site. Older files named it `exit.oneClickUrl`; that name still works. A page served
  without the functions (`vite`, `vite preview`, the e2e tests, another host) has no
  `/api/oneclick`; with the path there, the price preview shows an error. Such builds keep
  the 1Click URL.
- Runtime: Node.js (Vercel's default for files in `api/`, Web `fetch` handler). Vercel
  recommends Node.js over Edge
  (<https://vercel.com/docs/functions/runtimes/edge>). `web/vercel.json` sets
  `maxDuration` 20 s for these functions.

### What the function does

| Rule | How |
|---|---|
| Allowlist | One file per call; each file passes its fixed name to the handler, which knows the one method and upstream path of that name. Another method answers 405. |
| No open relay, no SSRF | The upstream origin is the constant `https://1click.chaindefuser.com`. No part of the request picks a host or a path. `status` forwards only `depositAddress` and `depositMemo` (each at most 256 characters) as query values; other calls forward no query. Redirects are not followed (502). |
| The key | `ONECLICK_JWT` from the environment, trimmed, sent as `X-API-Key`. Unset or empty: forwarded with no key, as the browser does today. |
| Which mode ran | Every answer has `x-oneclick-auth`: `key`, `none`, or `key-rejected` (1Click answered 401 to the key; the proxy asked once more without it, so a wrong or expired key never stops the site). |
| Request headers | None of the browser's. The proxy sets `accept` and, for POST, `content-type: application/json`. Cookies, `Authorization`, the browser's `X-API-Key`, `X-Forwarded-For` and the user agent are not forwarded. |
| Response headers | Only `retry-after` of 1Click, and its `content-type` when that is JSON (any other type is replaced by `application/json`, with `nosniff`, so nothing renders on the site's origin). `set-cookie`, `access-control-allow-origin` and every other upstream header are dropped. The body passes byte for byte. |
| Same origin only | A browser request from another site is refused (403) before anything is sent: `Sec-Fetch-Site` must be `same-origin` when present, and `Origin` must name this host when present. No answer carries `access-control-allow-origin`, so no other site can read an answer. POST must be `application/json`, which a page on another site cannot send without a CORS preflight, and the preflight (`OPTIONS`) answers 405. |
| Body cap | 64 KiB per request (by `content-length` and while reading). An answer above 4 MiB is a 502. Vercel's own limit is 4.5 MB (<https://vercel.com/docs/functions/limitations>). |
| Timeouts | 15 s per upstream call (504), 20 s per function run. |
| Caching | `quote`, `generate-intent`, `submit-intent`, `status` and every error: `cache-control: no-store`. `tokens` when it succeeds: `public, max-age=60, s-maxage=60`. |
| Logs | One line per failure with the call name and the error name, or "1Click refused the key". No body, no query, no key. |
| Security headers | The `/(.*)` headers of `web/vercel.json` (CSP, `X-Frame-Options`, `nosniff`, …) apply to function answers too ("custom response headers for static files, Vercel functions", <https://vercel.com/docs/project-configuration/vercel-json#headers>). |

`deploy/web/vercel-build.sh` stops the deployment if the value of `ONECLICK_JWT` is found
in `web/dist`. Vite puts only `VITE_*` variables into the bundle, so the key should never
be there; the check makes sure. Do not name the key `VITE_…`.

### What a hostile proxy can and cannot do

The engine treats the proxy as untrusted transport, the same as the network. The checks
in `crates/zecret-intents/src/check.rs` are unchanged: 1Click's ed25519 signature over
the quote, the approved amount, asset, recipient and minimum, and the payload to sign
(one `transfer` of the approved amount of the approved token to the quote's deposit
address, on `intents.near`, with a near deadline). `crates/zecret-wasm/tests/oneclick_proxy.rs`
runs the engine's exit through a proxy that edits recorded 1Click answers: an edited
price, minimum or deposit address, a missing signature, a payload that pays another
account, a larger amount, another token, another Verifier, a second intent, a far
deadline or another signing standard. Each is refused, and no signed intent leaves the
browser. A forged status is not believed. A hostile proxy can still refuse to answer, or
answer with errors, which stops the exit until it answers (the swap's USDC stays under
the swap key; the rescue in `docs/MAINNET.md` §6a works without 1Click). The price
preview is display only; it shows what the proxy answers.

### Privacy

The proxy sees what 1Click would see anyway: the amount, the destination token and
address, the swap key's NEAR Intents account (`refundTo`, the signer), the deposit
address, and the signed intent at `submit-intent`. The signed intent is public on NEAR
once 1Click submits it. The proxy does not see the swap key, the recovery file, Zcash
data, or any signature other than that signed intent. It runs on the site's host
(Vercel), so Vercel sees the user's IP address with these calls, as it does for every
page load. 1Click then sees Vercel's address instead of the user's.

## The owner's steps

### Turn it on

1. Get the key from <https://partners.near-intents.org/> (Partner Dashboard).
2. Vercel: project `zecret` (and `zecret-mainnet` when it exists) → Settings →
   Environment Variables → Add. Key `ONECLICK_JWT`, value the JWT, environment
   **Production** only, type Secret (Sensitive). Do not add it to Preview or Development:
   a preview deployment of any branch would use the key.
3. Redeploy production (Deployments → the latest production deployment → Redeploy).
   Environment variable changes apply only to new deployments
   (<https://vercel.com/docs/environment-variables>).
4. Check the key is in effect. A quote is the check that counts: 1Click answers 200 to
   `tokens` even with a wrong key.

   ```sh
   curl -sS -D - -o /dev/null https://<site>/api/oneclick/v0/quote \
     -H 'content-type: application/json' \
     -d '{"dry":true,"swapType":"EXACT_INPUT","slippageTolerance":100,"originAsset":"nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1","depositType":"INTENTS","destinationAsset":"nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near","amount":"5000000","refundTo":"0000000000000000000000000000000000000000000000000000000000000000","refundType":"INTENTS","recipient":"0x1111111111111111111111111111111111111111","recipientType":"DESTINATION_CHAIN","deadline":"2030-01-01T00:00:00.000Z"}' \
     | grep -i -E '^HTTP|x-oneclick-auth'
   ```

   Expect `HTTP/2 201` and `x-oneclick-auth: key`. `none`: the variable is not set for
   this deployment. `key-rejected`: 1Click refused the key (check its value).
   The engine's own check through the proxy, read-only (a dry quote whose signature the
   engine verifies, and a firm quote for a throwaway key that holds nothing):

   ```sh
   cd crates && cargo run -p zecret-wasm --example oneclick_proxy_check -- https://<site>/api/oneclick --firm
   ```

5. Point the site at the proxy: set `"oneClickUrl": "/api/oneclick"` in
   `web/public/config/testnet.json` (the price preview) and
   `web/public/config/mainnet.json` (preview and exit), commit, push. Do this only after
   step 4 shows `key`: without a key, the proxy sends every user's calls from Vercel's
   addresses, which share 1Click's 1200-per-minute limit for calls without a key.
6. Open `/swap`, enter an amount, and check in the browser's network tab that
   `/api/oneclick/v0/quote` answers with `x-oneclick-auth: key`.

### Turn it off

- Fast, no deployment of code: delete `ONECLICK_JWT` (Settings → Environment Variables)
  and redeploy. The proxy then forwards without a key (`x-oneclick-auth: none`).
- Fully: set `oneClickUrl` back to `https://1click.chaindefuser.com` in the configurations
  and push. The site then calls 1Click directly, as before. The configurations are served
  with `cache-control: no-cache`, so pages load the change at the next page load.
- If the key leaks: revoke it in the Partner Dashboard, then do the first step.

### Limit abuse (optional)

Anyone can call the proxy with curl and so spend the key's quota; CORS stops only other
sites' pages. What Vercel offers without paid add-ons:

- Vercel WAF rate limiting is on all plans. Hobby: one rate-limit rule per project,
  counted by IP or JA4 digest, fixed window of 10 s to 10 min, 1,000,000 allowed requests
  included (<https://vercel.com/docs/vercel-firewall/vercel-waf/rate-limiting>). A rule
  for the path prefix `/api/oneclick/`, for example 60 requests per 60 s per IP, keeps the
  page working (it asks for a quote every 30 s while the form is open) and stops a loop.
  It is set in the dashboard: Project → Firewall → Configure → New rule. Check the price
  dialog the dashboard shows before saving.
- The function's own limits: the allowlist, 64 KiB bodies, 15 s upstream timeout, and
  20 s `maxDuration`.

No per-user limit is built in. A per-instance counter in the function would not be one:
Vercel runs several instances.

## Not verified

- The function has not run on Vercel. The checks here ran the handler in vitest with a
  mocked upstream, and the compiled functions under a local Node server against the live
  1Click API (tokens, a dry quote, a firm quote for a throwaway key checked by the engine,
  a status; without a key, and with a fake key, which showed `key-rejected`).
- Vercel's handling, read in its docs and not tried: functions in `api/` of the Root
  Directory `web` are built next to the Vite output (`outputDirectory: dist`); the
  filesystem, including functions, is matched before rewrites, so the `/swap` rewrite does
  not touch `/api/*`; the `/(.*)` headers apply to function answers.
- Whether 1Click charges less with the key can only be seen in a quote's `appFees` and
  amounts, and in a funded swap.
