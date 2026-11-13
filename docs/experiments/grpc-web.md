# gRPC-web: can a browser read Zcash blocks?

**Question.** Light-wallet servers (lightwalletd) speak plain gRPC, which browsers cannot.
Can a web page still read Zcash blocks and server info?

**Result: yes, through a gRPC-web endpoint.** Tested 2026-11-13.

| Server | Plain gRPC | gRPC-web with CORS |
|---|---|---|
| `zcash-testnet.chainsafe.dev` | | Yes |
| `zcash-mainnet.chainsafe.dev` | | Yes |
| `zec.rocks`, `na.zec.rocks`, `testnet.zec.rocks` | Yes | No (answers as `application/grpc`) |

From a page in headless Chromium, with `fetch` and hand-built frames, against the
ChainSafe testnet endpoint:

| Call | Result | Time |
|---|---|---|
| `GetLightdInfo` | 1 message, `grpc-status: 0` | 852 ms |
| `GetLatestBlock` | height 4,614,381 | |
| `GetBlockRange` (10 blocks, server streaming) | 10 messages, 14.5 kB, `grpc-status: 0` | 386 ms |

Not tested: `SendTransaction`, and how long ChainSafe will keep these endpoints public.
For a local regtest node there is no lightwalletd; the page talks to Zebra's JSON-RPC
through a small proxy that adds CORS headers.

## Run

```sh
node test.js    # needs Playwright; uses HTTPS_PROXY if set
```
