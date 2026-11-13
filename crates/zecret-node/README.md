# zecret-node

The native programs on top of `zecret-core`: the maker daemon (it runs in our backend),
a reference taker (for tests, and the reference for the browser taker), and a tool that
sets up a local NEAR sandbox. One binary: `zecret`. It replaces the Cloakroom client in
`node/` (kept at the `pre-hackathon` tag).

`src/io.rs` holds the native implementations of the core's interfaces: `Http` (blocking
`reqwest` with rustls; it trusts the bundled web PKI roots and the system store, which
also reads `SSL_CERT_FILE`; it uses `HTTPS_PROXY` if set) and `FileStore` (one file per
record). Native builds switch on the
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
zecret check     --dir D                    # read both chains with D's settings (see below)
zecret maker     --dir D [--exit-after N] [--vanish-at STAGE] [--relay HOST:PORT]
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

- `check` reads the NEAR final block, checks the escrow (with `escrow_code_hash` in a
  taker config: the code hash, no access keys, the Intents account; else it prints them),
  prints the maker's balances (maker config), and reads the Zcash tip. Use it to see that
  every endpoint answers.

## NEAR testnet and Zcash testnet

`config/testnet-maker.json` and `config/testnet-taker.json` use the deployment in
`deploy/deployment.testnet.json`: escrow `escrow2.zecret-3f109eea1e.testnet` (code hash
`DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`, no access keys; it replaces
`escrow.zecret-3f109eea1e.testnet`, see `docs/security/fixes-2.md`), token
`usdc.zecret-3f109eea1e.testnet` (6 decimals), Verifier `intents.zecret-3f109eea1e.testnet`,
maker `maker.zecret-3f109eea1e.testnet`, NEAR RPC `https://rpc.testnet.fastnear.com` and
the gRPC-web light-wallet server `https://zcash-testnet.chainsafe.dev`. Timing: D = 30 s,
F = 10 s, R = 2 (B_R = 110 s; measured on testnet: included in at most 2.0 s, final in at
most 6.9 s), k_Z = 3 and ∆fund = 900 s for 75 s Zcash blocks.

Steps (not yet run end to end on testnet):

1. `cargo build -p zecret-node --release`.
2. Maker: a folder `M` with `config.json` from `testnet-maker.json`, `chains.birthday` set
   to the current Zcash tip (`zecret check` prints it), and `near_key.json` from
   `zecret-deploy maker-key` (see `deploy/maker/README.md`). `zecret init --dir M` prints
   the maker's Zcash address; send it testnet ZEC for quote fees (10,000 zats each).
   `zecret check --dir M`, then `zecret maker --dir M` (keep it running).
3. `zecret directory --dir M` prints the maker's directory entry.
4. Taker: a folder `T` with `config.json` from `testnet-taker.json`, `maker` replaced by
   that entry, `chains.birthday` the current tip, `min_price` as wanted.
   `zecret init --dir T` prints the taker wallet's address; send it
   `zats + 40,000 + max_premium` zatoshis in one payment, wait for it to be mined, then
   `zecret taker --dir T --zats <zats>`.

A swap on testnet takes about as long as five to ten Zcash blocks plus the cover delays
(off in these configs).

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
