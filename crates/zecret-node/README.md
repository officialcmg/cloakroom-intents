# zecret-node

The native programs on top of `zecret-core`: the maker daemon (it runs in our backend),
a reference taker (for tests, and the reference for the browser taker), and a tool that
sets up a local NEAR sandbox. One binary: `zecret`. It replaces the Cloakroom client in
`node/` (kept at the `pre-hackathon` tag).

`src/io.rs` holds the native implementations of the core's interfaces: `Http` (blocking
`reqwest`) and `FileStore` (one JSON file per record). Native builds switch on the
`multicore` features of `orchard` and `zcash_primitives`.

## Commands

Each party keeps its keys and records in its own directory, with its settings in
`<dir>/config.json` (`MakerConfig` or `TakerConfig`, see `docs/ENGINE.md`).

```sh
zecret init      --dir D                    # create the wallet key, print the unified address
zecret balance   --dir D                    # shielded balance (JSON)
zecret pay       --dir D --to UA --zats N   # fully shielded payment
zecret near-key  --dir D                    # create D/near_key.json, print the public key
zecret directory --dir D                    # maker: print the signed directory entry (JSON)
zecret maker     --dir D [--exit-after N] [--vanish-at STAGE]
zecret taker     --dir D --zats N [--vanish-at STAGE]
zecret devnet    --rpc URL --root-key validator_key.json --wasm-dir DIR \
                 --escrow-wasm near_escrow.wasm --maker-dir D [--maker-wnear N]
```

- `maker` runs until stopped (or until `N` swaps are terminal with `--exit-after`).
- `taker` starts a swap of `N` zatoshis, or resumes the one in its directory, and exits when
  it is terminal.
- `--vanish-at STAGE` exits when a swap reaches `STAGE`, with its state stored; running the
  same command again resumes. The scenario runner uses it to make a party disappear.
- `devnet` creates `wrap`, `intents`, `escrow` and `maker` under the sandbox's root account,
  deploys the live `wrap.near` and `intents.near` code and the escrow, initialises them,
  registers token storage for the escrow and Intents, and gives the maker wNEAR. It writes
  the maker's NEAR key to `<maker-dir>/near_key.json` and prints the account ids.

## Tests

```sh
source .tools/env.sh
cd crates
cargo test -p zecret-node --lib                                            # file store
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p zecret-node --test sandbox -- --nocapture
```

The sandbox test starts its own NEAR 2.14.1 sandbox and runs the escrow through the core's
NEAR client: open, stipend, set_ready, commit_claim (a wrong signature is refused),
claim, `a` read from contract state, withdraw into `intents.near`, a refund refused before
t1 and accepted after it, and `b` read from contract state. It also checks that the swap
id, commitment and commit message the client computes equal the contract's views. The
escrow wasm comes from `ESCROW_WASM` or is built in `contracts/near-escrow`.

The two-party scenarios (Zebra regtest and the NEAR sandbox) are in `harness/run.py`.
