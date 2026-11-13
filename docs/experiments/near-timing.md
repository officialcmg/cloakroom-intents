# NEAR timing: inclusion and finality

**Question.** What are `D` (time to inclusion) and `F` (inclusion to finality) for a
NEAR transaction? The swap's safety windows are built from these.

**Result** on testnet, 2026-11-12, 20 one-yocto transfers through
`rpc.testnet.fastnear.com`:

| Step | Min | Median | Max |
|---|---|---|---|
| Submitted to included | 7.2 s | 7.6 s | 7.9 s |
| Submitted to included in a final block | 8.4 s | 8.7 s | 9.2 s |
| Submitted to fully final | 9.6 s | 10.0 s | 10.4 s |

The clock starts before the client builds and sends the transaction, so the figures
include client and RPC latency. This is a small sample from one machine and one
provider, on testnet. Measure mainnet before fixing real parameters.

## Run

```sh
SAMPLES=20 cargo test --test timing -- --nocapture
```

The test creates two throwaway testnet accounts through the faucet.
