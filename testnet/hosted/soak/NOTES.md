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

## 06:14 to 06:33 UTC: switch done

- 06:14 merge pushed (commit `7291d8e`). Railway rebuilt the maker; its start log shows
  `config.json updated (no open swap)`, escrow `escrow3`, the same wallet and NEAR key,
  `open swaps: 0`. Vercel served the `escrow3` config at 06:19.
- 06:25 site set online (commit `048cbd2`).
- 06:25 to 06:33 health swap through the site: done in 461 s, 0.2877 test USDC.
  Checked by RPC: `commit_claim` `2HySRVh8A9…`, `claim` `2x7YMYwqqF…` and `withdraw`
  `8kpizUwM21…` were all signed by the taker's key `4f7b851e1349…`; `get_swap` shows
  state Claimed, payout paid; the taker's account held 0.0571 NEAR afterwards.
- Slot 14 (06:15) is recorded as `switch`. Slots 15 and later run on `escrow3`. Two
  slots are added at the end (11:00 and 11:15) for slot 09 and slot 14.

## Slots 15, 16, 17 (06:30 to 07:00 UTC): missed, the build machine restarted

- Symptom: no result lines after slot 14; at 07:21 the soak script was not running.
- Cause: the build machine (the cloud container that runs the test script) restarted at
  about 06:35 and killed the script and the browser of slot 15. The site and the maker
  run elsewhere and were not affected.
- Fix: the hourly check started the script again at 07:21. Slots 15 to 17 are recorded
  as `missed`. Three slots are added at the end (11:30, 11:45, 12:00) in their place.
- Slot 15's swap was cut off in the test browser. Its recovery file is in
  `.tools/testnet/hosted/` on the build machine.
