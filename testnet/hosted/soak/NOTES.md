# Soak test notes, 2026-11-14

## Slot 09 (05:00 UTC): FAILED before the swap started

- Symptom: the run ended after 2 s with `Error: GetLatestBlock: HTTP 502` from
  `zecret balance --dir .tools/testnet/taker`.
- Cause: the test script reads the payer wallet's balance with the command-line wallet
  before it opens the site. The public light-wallet server
  (`zcash-testnet.chainsafe.dev`) answered 502 once. The command-line wallet is a
  one-shot program and does not try again. The site was not opened, no swap was started,
  and nothing was paid. The app was not involved.
- Fix: `testnet/hosted/run.mjs` now runs the payer wallet's read steps again (up to 5
  times, 20 s apart). It runs a send again only if the send failed while it read the
  chain, before anything was broadcast.
- Not changed: the app. The maker and the browser engine already treat a failed Zcash
  read as a warning and read again on the next tick.
- Open point for mainnet: the app depends on one public light-wallet server per network.

## 06:09 UTC: soak paused for the switch to the light-payout escrow

Slots 01 to 13 ran on `escrow2` (12 done, slot 09 failed in the test script, see above).
The script was stopped between swaps. The site was paused, then the maker and the site
were moved to `escrow3`. The slots that fall in this window are recorded as `switch`.
