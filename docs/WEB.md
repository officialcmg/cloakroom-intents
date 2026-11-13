# The website

The Zecret website in `web/`: its pages, screens, states, and the decisions taken and
why. The interface to the swap engine is in `web/ENGINE_API.md`. How to build and test is
in `web/README.md`. The hosted copy is https://zecret-chrismgs-projects.vercel.app on
Zcash testnet and NEAR testnet (`docs/HOSTING.md`).

## Pages and routes

| URL | Page |
|---|---|
| `/` | The landing page: what Zecret does, how a swap works, who holds the money, what stays private and what is public, what happens on a failure, status, questions. It loads no engine, no maker directory and no network settings. |
| `/swap` | The swap app. The engine (worker and WebAssembly, about 9 MB) starts only here. |
| `/?demo=1`, `/?config=testnet`, `/?speed=20`, … | The swap app as well. A link to `/` that names an app setting (`demo`, `config`, `speed`, `fail`, `pause`, `cpu`, `directory`) opens the app and keeps its URL (`web/src/app/route.ts`). |

The two pages move with the History API, so going back to the landing page during a swap
does not stop the engine. While the engine reports a swap, the landing page shows "This
browser holds a swap" with a link to it.

On the hosted site, `/swap` runs the real engine on testnet, and `/?demo=1` runs the
simulated engine.

## What the website does

1. The user enters an amount of ZEC, a token, a chain and an address.
2. The page shows one price: "You pay X ZEC. You receive at least Y TOKEN on CHAIN."
3. The engine makes the swap's keys. The page offers a recovery file.
4. The user pays once, by QR code, from the user's own wallet.
5. The page shows the progress until the tokens arrive, or until the ZEC comes back.

The page holds no keys and moves no money. The engine does that, in a Web Worker. Two
engines implement the same interface:

- the real engine (`crates/zecret-wasm` in `src/engine/real/`), when the page has a
  network configuration (`?config=local`, `?config=testnet`, or `VITE_ZECRET_CONFIG` at
  build time; any other file or URL in `?config=` only in a development build, with a red
  warning that names it);
- the simulated engine (the demo), when it has none, or with `?demo=1`.

Each engine shows a one-line banner with a "Details" button:

- demo: "Demo. No real money moves." The details give the demo's speed and failure case,
  and "Demo controls";
- test network: "Testnet. Test money only." (or "Local test network."). The details give
  the network's name and the configuration's `honesty` lines: real transactions, test
  money, the stand-in token, and that the exit goes to a NEAR account because NEAR
  Intents has no testnet.

One tab of a browser runs the engine at a time. The page takes a Web Lock before it starts
the engine; a second tab shows "This swap is open in another tab", starts no engine, and
continues by itself when the first tab closes.

## Screens

Screenshots of each screen, light and dark, desktop and phone, are in `web/screenshots/`.

| Screen | Shown when | File |
|---|---|---|
| Swap form | No swap in this browser | `01-form-empty`, `02-form-price`, `03-token-picker` |
| Recovery file | Right after the user presses Continue | `04-recovery` |
| Payment | `awaiting_deposit` | `05-pay`, `06-pay-partial` |
| Progress | From `deposit_seen` to `exiting`, and the refund path | `07-progress-proving`, `08-progress-confirmations`, `09-progress-keep-open`, `11b-exit-delivering`, `15-maker-quiet-recovering` |
| Delivery paused | `exit_retry` | `11-exit-below-minimum`, `12-exit-failed` |
| Refund needs an address | `refunding` with no return address | `13-refund-needs-address` |
| Done | `done` | `10-done` |
| Refunded | `refunded` | `14-refunded` |
| Restore | The user chose "Restore a swap" | `16-restore` |
| Start failed | The network settings, the maker directory, the maker check or the engine failed | `17-directory-failed` |
| Another tab | Another tab of this browser runs the engine | no stored screenshot |

### Swap form

- **You pay.** ZEC, with the maker's limits under the field, and the amount in US
  dollars at the 1Click market price.
- **You receive.** A searchable list of every token that 1Click offers on a chain the
  page can check addresses for (35 chains). Exact symbol matches come first, so "sol"
  finds SOL before USDC on Solana. The list uses coloured initials, not logos, so the
  page loads no images from third parties.
- **Address.** Checked as the user types, per chain family, with checksums where the
  format has one (EIP-55, Base58Check, bech32 and bech32m, CashAddr, TON and Stellar
  CRC16, Cardano and Aleo bech32). If 1Click still refuses the address, the field says so.
- **Zcash return address** (optional, behind a link). It must be a unified address with
  an Orchard receiver on the swap's network: the page decodes the bech32m string,
  reverses F4Jumble and reads the receivers. A transparent or Sapling address gets a
  plain explanation.
- **The price.** One sentence in large type, the expected amount under it, the time of
  the last quote and a refresh link. "How this price is made" lists every part: four
  Zcash network fees (the three sends from the temporary wallet and the spend from the
  shared address, as the engine reserves them), the maker's premium, the ZEC sold, the maker's
  rate against the market price, the USDC from the maker and the 1% floor below which the
  swap stops, the NEAR Intents fee from the quote (`appFees`), the cost of the exit
  together, and "Your minimum" with what the engine enforces: no maker price below the
  floor (the swap stops and the ZEC comes back), no signed delivery whose firm minimum is
  below this amount unless the user accepts a lower one; NEAR Intents then delivers at
  least the signed minimum or returns the USDC. Delivery itself is not guaranteed.
- **Errors from 1Click** have their own words: "no price for this pair at this moment"
  (retried after 20 s), "too small for this route, at least N USDC", "no liquidity",
  "cannot reach NEAR Intents".
- Below the form: four lines on how it works, and what stays private and what is public.

### Swap form on a network without 1Click

Local and testnet configurations say `exit.mode: "direct"`. The form then:

- still offers every 1Click token and chain, and shows the live mainnet price for it,
  under the words "Mainnet preview": the page says that on this network the swap pays the
  stand-in token and sends it to the user's NEAR account, and that no token is delivered on
  the chosen chain;
- asks for a NEAR account instead of an address on the destination chain. It must exist
  and be registered with the token contract; the engine checks both before it signs.

The maker's side shows the stand-in token's symbol (wNEAR locally, "test USDC" on
testnet) instead of USDC.

### Recovery file

A key icon, a title, one sentence: "This browser just made the keys for your swap. If it
loses its data, the recovery file lets you finish the swap or get your ZEC back." One
primary button saves the file. After saving, the button becomes "Continue to payment".
A quiet link allows "Continue without saving"; the payment and progress screens then
keep a reminder. A small line says the file is private and can be deleted after the
swap. The proving-key build runs underneath with its own progress bar, so its 20 s is
spent while the user reads.

### Payment

The QR code (ZIP 321 `zcash:` URI with the amount) on a white plate in both themes,
the exact amount and the full address with copy buttons, and "Open in a wallet app".
Wallets named: Zodl, Zingo, Zkool, Edge, Cake. A calm pulsing dot says "Waiting for your
payment" and that the tab may be closed. A partial payment shows "Received X of Y" and
the QR code asks for the rest. "Change the swap" (only before any ZEC arrives) asks for
confirmation. In the demo, a dashed box offers "Simulate the payment" and "Pay only 40%".

### Progress

- A summary line (`1.00 ZEC → at least … on Base`) and an estimate: "About 6 min left,
  done around 14:32". The estimate is the sum of typical durations of the steps left
  (`web/src/engine/timeline.ts`), divided by the simulator's speed. It is hidden while
  the user must act.
- **Now.** What happens now in plain words, a progress bar for the current work (a
  proof, a block, a signature) drawn from the page's own clock, and a safety line:
  - green "You can close this tab. The swap waits and continues when you come back."
  - blue, during funding confirmations: "come back before 14:32 so the app can claim."
  - amber, from Ready until the claim is final: "Keep this tab open for about a
    minute." The page also asks before unloading in these stages.
- **Steps.** Six groups instead of sixteen stages: your payment arrives; the maker
  confirms the price; the maker locks the USDC; your ZEC goes to the shared address; you
  claim the USDC; TOKEN arrives on CHAIN. Done groups list their stages with times. On
  the refund path the group where the swap stopped gets a cross, and "Your ZEC comes
  back" follows.
- **Delivery track.** During `exiting`, four segments: firm price, check and sign,
  submitted, delivering.
- **Transactions.** Each with its chain, a label, the shortened hash with a copy button
  and an explorer link. Simulated hashes have no link, and the page says why.
- The browser tab title shows the step: "(5/6) Claiming your USDC · Zecret".

### Failure screens

| Case | What the page says | What the user can do |
|---|---|---|
| Firm quote below the minimum | The maker offered X, your minimum was Y; the swap stopped before the premium was paid; the ZEC comes back less two network fees. | Nothing, or enter a return address if none was given. |
| The maker does not answer | No price came back in time; same refund. | Same. |
| The maker refunds instead of Ready | The maker took its USDC back, which published its key half; the ZEC comes back; the premium stays with the maker. | Nothing. |
| The maker vanishes | After the deadline, the app claims the USDC anyway (punish path). The done screen says so. | Nothing. |
| Delivery price below the minimum | The new price, the minimum, "Nothing was signed", and that the USDC is under the user's key. | "Get a new price" or "Accept at least N". |
| Delivery failed or refunded | NEAR Intents returned the USDC; nobody else can move it. | "Try the delivery again". |
| No exit quote for a moment | The engine asks again in 20 s. | Nothing. |
| Page closed and reopened | The swap continues; the engine catches up with the networks first. | Nothing. |
| Browser data lost | The form shows; "Have a recovery file? Restore a swap". | Choose or drop the file. |
| The app cannot start | A title that names the cause (below), what it means for the user's money, the next step, and the engine's message as "Technical detail" with a copy button. | "Try again", where a retry can help. |
| The browser refuses to store the swap | A red notice: the engine has stopped and does nothing until the browser stores the swap again; it retries every few seconds. | Keep the tab open, free disk space, save the recovery file. |
| The WebAssembly engine crashes | The page starts a new engine from the stored swap and says so. After three crashes in ten minutes: the start-failed screen. | Nothing, or "Try again". |
| The swap is open in another tab | "This swap is open in another tab". | Use the other tab, or close it. |

### Done and refunded

Done: a check seal, "You received 933.623 USDC on Base", the address, the destination
transaction, what was paid, what the maker paid, the user's minimum, any leftover
ZEC, and "What this swap revealed": what stayed private (the Zcash wallet, the link
between that wallet and the destination, the memos) and what is public (the escrow
steps and USDC amount on NEAR, the amount and address on the destination chain, what
the maker knows, and the IP address seen by the services unless Tor or a VPN is used).
Refunded: the amount on its way back, the address, the reason, the transaction.

"Start a new swap" deletes the finished swap and its keys from the browser.

### Start failures

`web/src/app/failureCopy.ts` picks the title from the cause, in this order:

| Cause | Title |
|---|---|
| A link names network settings the site does not use | "This link asks for network settings this site does not use" |
| The network settings did not load | "The network settings did not load" |
| The maker directory did not load | "The maker’s details did not load" |
| NEAR did not answer during the maker check | "The maker could not be checked" |
| The maker's signed entry failed the check | "The maker could not be verified" |
| The engine download failed | "The swap engine did not load" |
| The engine stopped several times in ten minutes | "The swap engine stopped" |
| Any other start failure | "The swap engine could not start" (the browser may block workers, WebAssembly or storage) |

Each says "Nothing was sent and no money moved", or, when this browser holds a swap,
that the swap is stored and continues when the page starts again.

## States

The engine is the only source of truth. The page keeps its own copy by folding engine
events with the same reducer the simulator uses (`web/src/engine/reducer.ts`), and keeps
only two things of its own: the screen between "Continue" and "Continue to payment", and
preferences (theme, last token) in `localStorage`.

Which screen shows is a function of the swap's stage (`web/src/App.tsx`,
`screenFor`). Focus moves to the new screen's heading when the screen changes.

## Decisions

| Decision | Why |
|---|---|
| The engine runs the exit itself; the page passes only the approved asset, address and minimum. | The safety check before signing lives in the engine (`crates/zecret-intents`). The page cannot be tricked into signing. |
| The page makes dry quotes only. | Price display needs no deposit address. Firm quotes belong to the engine. |
| The user's minimum is 1Click's `minAmountOut` (1% slippage), scaled down to the lowest USDC the engine accepts (1% below the indicative rate). It is called "Your minimum", not a guarantee. | `minAmountOut` is exactly `amountOut × 0.99`. Measured drift over 10 minutes was at most 0.32% (docs/INTENTS.md §5), so the minimum is reachable. The engine takes a fresh firm quote at exit and does not sign below the minimum; delivery can still fail and refund (security review 3, H9). |
| `?config=` takes only the shipped names in production builds; mainnet settings must match constants in the bundle. | The configuration chooses the escrow, the servers and the maker. A link must not choose them (security review 3, NV3-01). |
| One tab runs the engine (Web Lock), and the real engine's store refuses a write from a tab with an older copy (`web/version`). | Two engines on one swap overwrite each other's records (security review 3, NV3-02). |
| After a failed write the engine calls nothing in the core until the write succeeds. | Nothing that reveals a secret or broadcasts may run on state that storage does not hold (security review 3, H1). |
| Links from 1Click, configuration explorer templates and recovery files must be https. | They become links on the page (security review 3, H3). |
| Fees come from the quote, not from constants. | Without a key, 1Click adds 1 to 20 bp as `appFees`; the amount varies by route. |
| Amounts are integers (bigint) and display rounds down. | A shown minimum is never above the real one. |
| One swap per browser. | The temporary wallet, its keys and the recovery file stay simple. A second swap waits for the first. |
| The recovery file is offered before payment, with one sentence of why, and can be skipped. | DESIGN.md: "Recovery file, shown before any money moves." Forcing it would push people to click through without reading. |
| Six step groups, not sixteen stages. | Users need to know where they are, not the protocol. The stage names stay visible inside each group. |
| Progress bars from the page's own clock. | The real worker blocks during a proof and cannot answer. The engine sends an estimate before it starts. |
| "Keep this tab open" only from Ready to the claim. | That is the only time closing the tab can cost the swap (DESIGN.md §6). Everywhere else the page says the tab may close. |
| No token logos. | They would load images from third parties and tell them the user's IP address and choice. |
| `Referrer-Policy: no-referrer`. | Explorer links do not reveal that the user came from Zecret. |
| Testnet only for the simulator. | Its deposit address is random. A testnet address cannot receive mainnet ZEC by mistake. |
| Fraunces for headings and amounts, Archivo for text, IBM Plex Mono for addresses; warm paper and ink colours with a gold accent. | A calm, editorial look of its own, not Cloakroom's black outlines. Archivo and Plex Mono come from `design-system/`; all fonts are served by the site. |
| Theme follows the system, with a toggle that is remembered. | Both themes are designed; the user decides. |

## Accessibility

- Keyboard: every control is reachable; the token list is a combobox with arrow keys and
  Enter; dialogs are native `<dialog>` elements (focus trap, Escape).
- A skip link, landmarks, labelled fields with descriptions, `aria-invalid` on errors,
  `aria-live` for the payment wait, the "now" panel and copy confirmations, and
  `role="progressbar"` with values.
- Colours meet WCAG AA in both themes: every text and status colour pair used is at
  least 4.94:1 (light) and 5.26:1 (dark), computed from the tokens; field and button
  borders are at least 3:1. Status is never shown by colour alone.
- `prefers-reduced-motion` stops the pulses, the spinner and the sliding bar.

## Tests

- Unit (`web/tests/unit/`): price math, amounts, the maker directory, ZIP 321, 1Click
  parsing and error classes, address checks for every chain family, Zcash unified
  addresses against vectors from `zcash_address`, the reducer, and the simulated engine
  through the real transport on every path.
- Unit, security review 3: `config_override.test.ts` (what `?config=` and a configuration
  may choose), `realEngine.test.ts` (with a scripted WebAssembly module: a failed write
  stops the core calls, a trap stops the engine, the version guard, the payment address of
  a restored file).
- End to end, `web/e2e/landing.spec.ts`: the landing page loads no engine, its links open
  the app at `/swap` and Back returns; old links with app settings open the app; the
  one-line banners and their details; a failed maker check on testnet.
- End to end, `web/e2e/responsive.spec.ts`: every screen at 320 and 390 px wide.
- End to end, `web/e2e/tabs.spec.ts`: two tabs of one browser on one swap; a production
  build refuses settings from a link.
- End to end (`web/e2e/journey.spec.ts`), on a desktop and a phone viewport: the full
  swap; each failure case; reload mid-swap; restore after a cleared browser and in a
  fresh browser; form checks with keyboard-only token selection; 1Click errors; the
  token list down; the tab-close guard; page responsiveness while the worker busy-loops;
  the theme toggle.
- Counts on 2026-11-13: 88 unit tests (`npm test`) and 74 Playwright tests (`npm run
  e2e`: desktop and phone, the 320 and 390 px runs, and 14 screenshot tests).

## Headers

`vercel.json` sets, for every path:

| Header | Value and why |
|---|---|
| `Content-Security-Policy` | `default-src 'self'`; `script-src 'self' 'wasm-unsafe-eval'` plus the SHA-256 of the one inline script (the theme, before first paint); `worker-src 'self'`; `connect-src 'self'` plus exactly the hosts the engine and the page call on testnet: `https://1click.chaindefuser.com`, `https://zcash-testnet.chainsafe.dev`, `https://rpc.testnet.fastnear.com`, and the maker's relay `https://zecret-maker-production.up.railway.app`; `img-src 'self' data: blob:`; `style-src 'self'`; `font-src 'self'`; `object-src 'none'`; `base-uri 'none'`; `form-action 'none'`; `frame-ancestors 'none'`. WebAssembly needs `wasm-unsafe-eval`. The synchronous XMLHttpRequests of the worker are governed by `connect-src`. `tests/unit/csp.test.ts` checks the hash and the hosts against the https endpoints of `public/config/testnet.json`, including `exit.relayUrl`, and refuses any `http:` host. The relay URL is in the configuration, so a new relay URL makes the test fail until `connect-src` lists it. |
| `Referrer-Policy` | `no-referrer` |
| `X-Content-Type-Options`, `X-Frame-Options` | `nosniff`, `DENY` |
| `Cross-Origin-Opener-Policy` | `same-origin` |
| `Permissions-Policy` | camera, microphone, geolocation, payment and USB off |

The browser scenarios (`harness/browser/run.mjs`) serve the site with these headers and add
only the local endpoints to `connect-src`; a CSP violation would show in their console log.
They must build the site with `VITE_ZECRET_ALLOW_CONFIG_URL=1`, because they load their
configuration from a path.

`vercel.json` also sets `"installCommand": "npm ci"`, so the deployment installs the locked
versions.

## Not done here

- Firefox and Safari were not tested; only Chromium.
- No wallet app was tested as the payer. The testnet payments came from the project's
  command-line wallet (`zecret pay`).
- Destination tags (XRP) and memos are not supported; the page says so for XRP.
