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

## Code

The experiment's code ran in a separate scratch repository and is not in this
repository. A later measurement with a different method is in this repository:
`zecret-deploy e2e` (`deploy/tool/src/e2e.rs`) records, for each transaction of a full
swap on testnet, the time to inclusion and to finality, polling the status every 0.2 s.
Its results (median 1.6 s to inclusion, 4.5 s to all receipts final, over 14
transactions) are in `docs/DEPLOY.md`, "Measured on testnet", and in
`deploy/e2e.testnet.json`. The cause of the difference from the figures above was not
investigated. `zecret-deploy e2e` spends testnet funds of the deployment in
`deploy/deployment.testnet.json`; see `docs/DEPLOY.md` for how to run it.
