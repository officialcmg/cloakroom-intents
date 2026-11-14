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

## 06:39 to 07:27 UTC: the maker missed a refund on `escrow3` (serious; soak paused)

- What happened: the machine restart cut slot 15 off after the maker had opened its
  escrow (swap `c2b9fa50…`, 0.2877 tUSDC, opened 06:39). The taker never funded. The
  maker had to refund. It did not: from 06:53 at the latest, every NEAR RPC call of the
  hosted maker was answered `-429 Rate limits exceeded` by `rpc.testnet.fastnear.com`,
  including its `send_tx`. The punish deadline t2 (07:18:51) passed with the escrow
  still Open. The maker's log then repeats "past t2 with the escrow Open: only the
  taker can settle it now" about once per second.
- Effect: 0.2877 test USDC of the maker is locked in `escrow3` for a taker key that no
  longer exists. On mainnet the same fault would let a taker who never paid claim real
  USDC. The maker also cannot serve new swaps while it is rate-limited.
- The soak was stopped at 07:27 until this is fixed.

## 07:27 to 09:50 UTC: cause, fix and live check of the missed refund

- Cause (full note: `docs/incidents/2026-11-14-maker-missed-refund.md`): the maker read
  the escrow about once per second for a swap whose taker was gone (about 120 requests a
  minute). Its only NEAR endpoint began to refuse it at 06:43:47. The maker decided to
  refund at 06:43:55, which was correct, and tried 28 times; every send was refused. It
  had no second endpoint and no back-off.
- Fix (commit `213ad19`, released 09:29): an ordered list of three NEAR endpoints with
  failover and back-off; sends that protect funds go first and try every endpoint; the
  maker reads on a plan (2 to 4 requests a minute for the same state); a swap past its
  last deadline is watched every 200 s with one alert; `GET /health` on the relay port.
  New scenarios s26 (RPC rate-limited during the refund window) and s27.
- Checks by the main session before release: core and node tests, start-script tests,
  website unit tests, and scenarios s1, s3, s25, s26.
- Live check, 09:40 to 09:50: a swap was started through the site and the browser was
  closed when the escrow opened (09:45:09, escrow id `61bcde2a…`). At 09:50 `get_swap`
  showed state Refunded, payout paid. The maker's health route showed 59 NEAR requests
  in total since its start at 09:31 and no rate limiting.
- The first stuck swap (`c2b9fa50…`, 0.2877 tUSDC) stays locked in `escrow3`: its taker
  key is gone and the contract allows no refund after t2 for an Open swap.
- Slots 18 to 28 are recorded as `paused`. The soak runs again from slot 29 (10:00) to
  slot 41 (13:00).

## Slot 29 (10:00 UTC): lost, the build machine restarted again

- The cloud machine that runs the test script restarted during slot 29 and killed the
  script. The hourly check started it again at 10:21; slot 30 ran at once. The maker's
  health route stayed `ok` throughout. This is a fault of the test machine, not of the
  app. If slot 29's swap had opened an escrow, the maker refunds it by itself.

## Why the build machine "restarted": it stops when the session is idle

- The machine's start time (10:20:37) equals the time of the hourly check. The cloud
  machine is stopped when the build session has nothing running, and started again by
  the next event. A script started detached (`setsid nohup`) does not count as running.
  So the soak was killed at 06:35 and at 09:51, each time a few minutes after the main
  session went quiet. It survived 03:00 to 06:30 only because agents were working.
- Fix: from 10:22 the soak runs as a tracked background task of the session (2 hours
  at most per start; the hourly check starts it again).
