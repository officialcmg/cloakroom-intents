# Gas spike: key-share checks on NEAR

A feasibility test, not a product. It measures whether a NEAR contract can
do the three checks the escrow needs: that a revealed key share `s` matches a
stored public share `S = s·G` on Pallas (Zcash Orchard-protocol keys) or on
ed25519 (Monero keys), and that an ed25519 signature is valid.

Result on a local sandbox, whole transaction, against a 300 Tgas
per-transaction limit:

| Check | Method | Gas (nearcore 2.14.1) | Gas (nearcore 2.9.0, lowered wasm) |
|---|---|---|---|
| ed25519 share | `check_share` | 5.1 Tgas | 8.9 Tgas |
| Pallas share | `check_share_pallas` | 16.2 Tgas | 16.5 Tgas |
| ed25519 signature (native host function) | `check_signature` | 1.8 Tgas | 2.0 Tgas |

The experiment used a throwaway contract with the three check methods and sandbox tests
that called them.

## Code

The experiment's code ran in a separate scratch repository and is not in this
repository. The escrow's sandbox tests supersede it for the two checks the escrow uses
(the Pallas key-half check in `claim` and `refund`, and the ed25519 signature check in
`commit_claim`). They print the gas of each call as `GAS ...` lines:

```sh
source .tools/env.sh
cd contracts/near-escrow
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test --test sandbox -- --test-threads 1 --nocapture
```

The ed25519 key-share check (for Monero keys) has no equivalent in this repository.
Tests must use nearcore 2.14.1 or newer, so the `wasm-opt` lowering for 2.9.0 is not
needed.
