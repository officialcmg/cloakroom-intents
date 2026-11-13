# The website

This document describes the Zecret website in `web/`: its screens, the states behind
them, and the decisions taken and why. The interface to the swap engine is in
`web/ENGINE_API.md`. How to build and test is in `web/README.md`.

## What the website does

1. The user enters an amount of ZEC, a token, a chain and an address.
2. The page shows one price: "You pay X ZEC. You receive at least Y TOKEN on CHAIN."
3. The engine makes the swap's keys. The page offers a recovery file.
4. The user pays once, by QR code, from the user's own wallet.
5. The page shows the progress until the tokens arrive, or until the ZEC comes back.

The page holds no keys and moves no money. The engine does that, in a Web Worker. Until
the WebAssembly engine is ready, a simulated engine runs behind the same interface. The
page then shows a banner: "Demo with a simulated engine: no real funds move."

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
| Start failed | The maker directory or the engine did not load | `17-directory-failed` |

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
  the last quote and a refresh link. "How this price is made" lists every part: Zcash
  network fees for three transactions, the maker's premium, the ZEC sold, the maker's
  rate against the market price, the USDC from the maker and the 1% floor below which the
  swap stops, the NEAR Intents fee from the quote (`appFees`), the cost of the exit
  together, and the guaranteed minimum.
- **Errors from 1Click** have their own words: "no price for this pair at this moment"
  (retried after 20 s), "too small for this route, at least N USDC", "no liquidity",
  "cannot reach NEAR Intents".
- Below the form: four lines on how it works, and what stays private and what is public.

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
| Maker directory or engine fails | What failed and a likely reason. | "Try again". |

### Done and refunded

Done: a check seal, "You received 933.623 USDC on Base", the address, the destination
transaction, what was paid, what the maker paid, the guaranteed minimum, any leftover
ZEC, and "What this swap revealed": what stayed private (the Zcash wallet, the link
between that wallet and the destination, the memos) and what is public (the escrow
steps and USDC amount on NEAR, the amount and address on the destination chain, what
the maker knows, and the IP address seen by the services unless Tor or a VPN is used).
Refunded: the amount on its way back, the address, the reason, the transaction.

"Start a new swap" deletes the finished swap and its keys from the browser.

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
| The guaranteed minimum is 1Click's `minAmountOut` (1% slippage), scaled down to the lowest USDC the engine accepts (1% below the indicative rate). | `minAmountOut` is exactly `amountOut × 0.99`. Measured drift over 10 minutes was at most 0.32% (docs/INTENTS.md §5), so the minimum is reachable. The engine takes a fresh firm quote at exit and does not sign below the minimum. |
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
- End to end (`web/e2e/journey.spec.ts`), on a desktop and a phone viewport: the full
  swap; each failure case; reload mid-swap; restore after a cleared browser and in a
  fresh browser; form checks with keyboard-only token selection; 1Click errors; the
  token list down; the tab-close guard; page responsiveness while the worker busy-loops;
  the theme toggle.

## Not done here

- The real engine. The page is ready for it through `web/ENGINE_API.md`.
- A Content-Security-Policy header. Its `connect-src` must list the light-wallet server
  and NEAR RPC of the real engine; it is better added with the engine.
- Firefox and Safari were not tested; only Chromium.
- Destination tags (XRP) and memos are not supported; the page says so for XRP.
