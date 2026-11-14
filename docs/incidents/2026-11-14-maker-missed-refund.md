# Incident 2026-11-14: the hosted maker missed a refund under NEAR RPC rate limits

Network: Zcash testnet and NEAR testnet. Service: the hosted maker on Railway
(`deploy/maker/railway`), escrow `escrow3.zecret-3f109eea1e.testnet`, NEAR RPC
`https://rpc.testnet.fastnear.com` (the only endpoint in its settings).

## Summary

A taker accepted a quote and then disappeared before she funded (the build machine that
ran the test browser restarted). The maker had to refund its escrow: first before t0
(rule R3e), or else in [t1, t2 − B_R) (rule R3d). It decided the refund at the earliest
moment the rules allow, 4 s after the funding window closed. From the same minute, the
NEAR RPC endpoint answered most of the maker's requests with JSON-RPC error -429, "Rate
limits exceeded". The maker tried the refund 28 times; the endpoint refused every
`send_tx`. The punish deadline t2 passed with the escrow Open. Since then the escrow
accepts no refund for that swap, and the maker repeated an alert about once a second.

## Impact

- Swap `c2b9fa50` (escrow id `5e75b070…`): 287,700 base units of test USDC (0.2877
  tUSDC) of the maker are locked in `escrow3`. The escrow refuses `refund` after t2 for an
  Open swap. Only `commit_claim`, signed by the taker's key, can move the swap on. That
  key was in the test browser, which is gone; its recovery file was overwritten at 07:22
  by the next run of the test script (`.tools/testnet/hosted/recovery.json` now holds
  swap `d93f5090`). So nobody can lock the swap, and the tokens stay in the escrow. The
  maker kept the premium (1,000 zats).
- On mainnet the same fault would let a taker who never paid claim the maker's USDC after
  t2 (`commit_claim` on the punish path, then `claim`): at most one maximum swap
  (`max_zats`) per missed refund.
- From 06:43 the maker was rate-limited for all its NEAR work. It could not have opened a
  new escrow. The soak test was stopped at 07:27; no other swap was started on the maker.

## Timeline (UTC)

Maker times are the Unix times in its log lines (Railway project `7cef823e…`, service
`2708fc53…`). NEAR deadlines of the swap: t0 = 1794639231 (06:53:51), t1 = 1794639831
(07:03:51), t2 = 1794640731 (07:18:51); testnet timing D = 30 s, F = 10 s, R = 2, so
B_R = 110 s and the R3d window ends at 07:17:01.

| Time | Event |
|---|---|
| 06:14 | The maker and the site switch to `escrow3` (commit `7291d8e`). Same RPC endpoint as before. |
| 06:25 to 06:33 | A health swap through the site completes. |
| 06:35:27 | Test browser (soak slot 15): request for 959,000 zats sent. |
| 06:37:21 | Maker reads the request, quotes (Zcash tx `ee427b31…`). |
| 06:38:00 | Test browser: acceptance with the premium sent. |
| 06:38:41 | Maker reads the acceptance; stage Opening. |
| 06:39:00 to 06:39:03 | Maker: deposit `EmTAex8i…`, open `EJf8f5pP…`, stipend `AmNtL3sn…`; stage Opened. |
| 06:39:04 | Test browser: escrow verified, stipend arrived. Its last line: the machine restarted after this. |
| 06:39:03 to 06:43:47 | Maker logs nothing: its reads (one final block and one `get_swap` each second) work. |
| 06:43:47 | First `-429` in the maker log. |
| 06:43:51 | The funding window closes (t0 − ∆fund, ∆fund = 600 s): R3e may start. |
| 06:43:55 | Maker: "no funding before t0; refunding"; stage RefundSent. |
| 06:43:55 to 06:53:20 | Every step of the refund fails with -429: the `view_access_key` and `block` reads for signing, or the `get_swap` read. 25 to 31 warnings a minute. |
| 06:53:20 | First signed refund (`DgSSGEXF…`): `send_tx` answered -429. |
| 06:53:21 | R3e window ends (t0 − D). The contract accepts no refund until t1. |
| 07:03:51 | t1: R3d window opens. |
| 07:03:57 to 07:18:52 | 27 more signed refunds, one every 31 to 42 s; every `send_tx` answered -429. Many reads in this period were answered (alerts with the NEAR time every 2 to 3 s). |
| 07:17:01 | t2 − B_R: the R3d window ends. |
| 07:18:51 | t2. Escrow still Open. |
| from 07:18:51 | About once a second: "ALERT: past t2 … only the taker can settle it now" and "L_M violated …", mixed with -429 warnings. |
| 07:27 | Soak test stopped. |

## Root cause

**1. The refund needed one RPC endpoint, and that endpoint refused it (certain).** The
maker had one NEAR RPC URL. All 28 `send_tx` of the refund got the JSON-RPC error -429
from `rpc.testnet.fastnear.com`, and the maker had nowhere else to send them. Reads were
answered part of the time (the alerts in [t1, t2) show fresh NEAR times every 2 to 3 s);
the `send_tx` calls never were. Why fastnear refused `send_tx` more than reads is not
known (fastnear publishes no limits).

**2. The maker's own traffic was high and it made it worse under the limit (certain).**
Measured with a counting proxy in front of the local NEAR sandbox (scenario
`s27_rpc_budget`, the binary before the fix and after):

| State | Before the fix | After the fix |
|---|---|---|
| Idle (no open swap) | 0 | 0 |
| One swap Opened, taker gone | 118 a minute (59 final blocks, 59 `get_swap`) | 4 a minute |
| Past t2 with the escrow Open | 121 a minute, and 242 alert lines in 2 minutes | 8 a minute (sandbox W = 45 s; with the testnet W = 600 s: under 1 a minute); 1 alert line |

Before the fix the maker read every open swap at every tick: the main loop slept 1 s
(`POLL_SECS`), and each read was a final block and a `get_swap`. A refund that had been
sent and was still "on its way" returned `Next::At(0)` (`maker.rs`, `refund`): the loop
then ran again at once, so for up to D = 30 s after each signed refund the maker read the
chain as fast as the RPC answered. Each failed tick logged a warning; past t2 each tick
logged an alert and an "L_M violated" line (they alternated, so the "same as the last
line" check never held them back), and appended both to the swap record.

**3. Why it began at 06:43 and not before (partly known).** The maker sent the same
traffic before the switch: 12 swaps completed on `escrow2` with the same endpoint and the
same 1 s loop. The new escrow did not change how often the maker reads. What was new is
that a swap stayed in Opened for a long time: in a normal swap the taker funds within
minutes and the swap moves on; here the swap waited 5 minutes with the maker reading at
the full rate before the first -429. From this build machine, fastnear answered 150
sequential `block` requests (3.5 a second for 43 s) and then 200 requests from 8 parallel
clients (28 a second for 7 s) without one 429 (2026-11-14 07:3x). So the maker's 2
requests a second alone do not explain the limit. Railway's outgoing traffic leaves from
addresses that are shared with other services unless static IPs are bought; if the limit
is per address, other users on the same address count too. This is likely and not proven:
the maker's outgoing address and fastnear's rule are not known. The website and the test
scripts call fastnear from the users' and the build machine's addresses, not from
Railway's, so they did not count against the maker. The test browser stopped at 06:39.

**4. The rules did not cause the miss (checked).** The maker took the R3e decision 4 s
after the earliest moment (06:43:51 window, 06:43:55 decision). The light-payout escrow did
not change when a refund may be sent (`contracts/near-escrow/src/lib.rs`, `refund`: Open
before t0, Open or Ready in [t1, t2), Locked from lockedAt + W).

**5. After t2 nothing can work (contract rule).** `refund` of an Open or Ready swap is
refused from t2 (`ERR_TOO_EARLY`, "no refund window is open"). From t2 only
`commit_claim` with the taker key's signature changes the state (punish path, to Locked).
If that happens and the taker does not claim within W, the maker can refund (R3l); if she
claims, the maker can sweep the joint note (if there is one). The old maker had no idea of
this state: it read it every second forever.

## Fix

- NEAR RPC client (`crates/zecret-core/src/near/rpc.rs`): an ordered list of endpoints
  (`chains.near_rpc` is one URL or a list; old configs still parse). On HTTP 429, the
  JSON-RPC -429, HTTP 5xx, no response or a node that is behind, the same request goes to
  the next endpoint at once; the failed one backs off 2 s doubling to 60 s with ±20 %
  jitter, longer when it sends `Retry-After`. A request tries each endpoint once and the
  client never sleeps. When all endpoints back off, a read fails at once without a
  request. In critical mode (refund, set_ready of a funded swap, the taker's commit,
  claim and withdraw) the endpoints that back off are tried too, after the others. One
  final block read serves a tick. Native transport: 10 s connect timeout.
- Read plan (`maker.rs`, `MakerConfig::polling`): a swap is read at its stage's interval
  (30 s while it waits for funding, 10 s while the taker acts, 5 s after a refund) and
  just after each deadline that matters, never more often than every 3 s. A failed read
  backs off; a failed deadline-critical step is retried every 3 s across all endpoints.
  While a critical step fails, the tick does no other NEAR work. The tight loop after a
  sent refund is gone.
- Past t2 with the escrow Open or Ready: one read every `min(300 s, W / 3)`, one alert,
  repeated every 30 minutes. If the taker's key locks, the maker refunds at lockedAt + W
  (R3l) or sweeps after a claim, as before.
- Alerts and logs: each kind of alert per swap at most once per `quiet_secs` (600 s); a
  warning that repeats (same text up to numbers and hashes) at most once a minute, with a
  count. An alert `near_failing` after a NEAR step of an open swap has failed for 60 s.
- Health: `GET /health` on the relay port, and `--health HOST:PORT` (mainnet start
  script). JSON: last NEAR and Zcash success, endpoints, open swaps by stage, nearest
  deadline, alerts. No keys, addresses or amounts.
- Settings: testnet `rpc.testnet.fastnear.com`, `near-testnet.drpc.org`,
  `testnet-rpc.intea.rs`; mainnet templates `rpc.mainnet.fastnear.com`, `near.drpc.org`,
  `rpc.intea.rs`. The website's `near.rpc` takes the same list; the CSP lists the new
  hosts.
- Start script: with a swap open it now applies a config change that touches only
  `chains.near_rpc` or `polling`, so the stuck swap's maker gets the new endpoints.
- Tests: unit tests of the client (429 with and without `Retry-After`, the JSON -429,
  failover order, back-off bounds, critical mode, no request while backing off) and of
  the maker (requests per minute in Opened, Ready and past t2; the refund through the
  second endpoint while the first answers 429; both endpoints down then one back; the
  stuck swap refunded after a lock). Native scenarios `s26` (a proxy that answers 429 in
  front of the sandbox) and `s27` (the request budget).

## What happens to swap c2b9fa50

The new maker starts with the swap in RefundSent, reads it every 200 s (W / 3), logs one
alert ("past t2 … the escrow allows no refund now") and repeats it every 30 minutes. It
sends nothing. If someone ever sends a `commit_claim` signed by the swap's taker key, the
maker refunds after the reveal window. Without that key, 0.2877 tUSDC stay in `escrow3`.
The swap stays an open swap in the start script's count.

## Not known

- fastnear's rate-limit rule (per address, per method, per time window) and whether
  other services behind Railway's shared address were counted. fastnear publishes no
  limit for its free endpoints ("Not published" in the NEAR docs' provider list). An
  unofficial measurement on `free.rpc.fastnear.com` (2026-11-04) reported a burst of 50 to
  100 requests and a refill of about 1.5 to 2 a second, with no rate-limit headers; this
  build machine could not reproduce a limit at 28 requests a second.
- Why reads were often answered while every `send_tx` was refused.
- Whether the limit has ended: the hosted maker still got -429 at 07:29.
- The other endpoints' limits. drpc publishes "~2,100 CU/s" for its free tier; intea
  slows requests without a key by one second; neither was tested under load.
