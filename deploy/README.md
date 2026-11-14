# deploy

Tools to put Zecret's NEAR contracts on a network and check them.

| Path | What it is |
|---|---|
| `tool/` | `zecret-deploy`, the deployment and check tool (Rust, near-workspaces 0.24 for signing, plain JSON-RPC for reads). |
| `test-usdc/` | The stand-in USDC for testnet: a standard NEP-141 token with 6 decimals and an open `mint`. It has no value. |
| `configs/` | `testnet.toml`, `mainnet.toml`, `sandbox.toml`. |
| `deployment.testnet.json` | The testnet deployment: account ids, code hashes, transaction hashes and block heights. No keys. Earlier escrows of the deployment are under `retired_escrows`. |
| `e2e.testnet.json` | The end-to-end runs on testnet: every transaction hash and its timings. |
| `maker/` | Dockerfile and notes for running the maker daemon as a service. |
| `maker/railway/` | The hosted testnet maker on Railway: Dockerfile, start script, its signed directory entry (`docs/HOSTING.md`). |
| `web/` | The Vercel build of the website with the browser engine, its ignored-build-step script, and the directory converter (`docs/HOSTING.md`). |
| `.keys/` | Private keys the tool makes or uses. Not in git (see `.gitignore`), mode 0700, files mode 0600. |

Steps, costs and checks for testnet and mainnet are in [`docs/DEPLOY.md`](../docs/DEPLOY.md).

## Build

```sh
cd /home/user/zecret && source .tools/env.sh
(cd contracts/near-escrow && cargo build --target wasm32-unknown-unknown --release)
cd deploy
cargo build -p test-usdc --target wasm32-unknown-unknown --release
cargo build -p zecret-deploy
```

## Commands

```sh
T=./target/debug/zecret-deploy
$T rehearse --config configs/sandbox.toml   # local sandbox: deploy, deploy again (must send nothing), e2e
$T deploy   --config configs/testnet.toml   # create or complete the deployment; safe to run again
$T verify   --config configs/testnet.toml   # read-only checks at a final block; exit 1 if one fails
$T plan     --config configs/mainnet.toml --owner ACCOUNT [--maker-config F] [--expect-escrow-hash H]
                                            # the dry run: read-only checks, the plan and its cost (docs/MAINNET.md)
$T e2e      --config configs/testnet.toml   # one swap end to end with real transactions
$T maker-key --config configs/testnet.toml --out DIR   # DIR/near_key.json for `zecret maker --dir DIR`
$T hash FILE.wasm                           # the NEAR code hash of a wasm file
$T fund --config configs/testnet.toml ACCOUNT_ID NEAR   # testnet: top up any account from the faucet
```

The sandbox needs `LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD`.

`--escrow-wasm FILE` replaces the escrow wasm path of the config.

On mainnet, `deploy` refuses to run without
`--i-am-the-owner-and-accept-spending-real-funds`. `plan` and `verify` only read and run
on mainnet without it; `plan` has no signer at all. `e2e` and `rehearse` never run on
mainnet. `e2e` attaches 200 Tgas to `withdraw` (the escrow's two-step Intents withdraw
needs at least 110 Tgas) and checks that the escrow's own Verifier balance is 0 after it. `verify` runs on mainnet; it only reads.

## What `fund` does

`fund` sends at least `NEAR` to the existing testnet account `ACCOUNT_ID`. Use it to
top up the maker or any other testnet account. It sends nothing from the owner.

1. It refuses to run if the config is not for testnet, if `ACCOUNT_ID` is not a valid
   account id or ends in `.near`, or if the amount is 0 or needs more faucet accounts
   than `max_faucet_accounts`.
2. It reads the balance of `ACCOUNT_ID` at a final block. If the account does not
   exist, it stops. (A delete with a beneficiary that does not exist burns the funds.)
3. It creates a faucet account (10 NEAR) and deletes it with `ACCOUNT_ID` as
   beneficiary. Before each delete it checks again that `ACCOUNT_ID` exists. A faucet
   account left by an earlier run (its key is still in `.keys/testnet/`) is used first.
   If the network drops the delete, the tool sends it again, up to 4 times.
4. It counts each faucet account as its balance less 0.01 NEAR for the gas of the
   delete, and repeats step 3 until the count reaches `NEAR`. One faucet account covers
   at most 9.99 NEAR, so `fund ... 10` uses two.
5. It reads the balance again at a final block and prints both balances and the
   transaction hashes. The difference also includes what the account itself spent in
   between.

`fund` does not write the deployment file. The real delete cost is about 0.0003 NEAR
(measured on 2026-11-14: 10 NEAR faucet account, `maker.zecret-3f109eea1e.testnet`
went from 0.62605 to 10.62577 NEAR, transaction
`H3Ks4ysGqTAomQeyv9nMcMnsFoGarQLYRv3tsJhvChaS`).

## What `deploy` does

Each step reads the chain first and does nothing if its result is already there. A
second run sends no transactions (`rehearse` checks this).

1. Owner. Testnet: if no owner is recorded, the tool picks `zecret-<random>.testnet` and
   creates it through the faucet. Mainnet: `owner_account` from the config, with its key
   in `.keys/mainnet/<owner>.json`.
2. Funds. The tool adds up what the missing accounts need (code storage plus margins).
   On testnet, if the owner holds less, it creates faucet accounts (10 NEAR each) and
   deletes each one with the owner as beneficiary. On mainnet it stops and says how much
   is missing.
3. Token (testnet): create `usdc.<owner>`, then one transaction from that account:
   deploy `test-usdc`, call `new` (private).
4. Verifier (testnet): create `intents.<owner>`, then one transaction: deploy the code
   of mainnet `intents.near`, call `new` with `wnear_id = wrap.testnet`, fee 0, fee
   collector itself (as the sandbox tests do).
5. Escrow: create `<escrow.sub_account>.<owner>` with one temporary full-access key, then
   one transaction from the escrow account to itself: `DeployContract`,
   `new(intents_account)`, `DeleteKey` of the signing key. The tool then deletes its copy
   of that key. A deployed escrow cannot change: for a new escrow build, set a new
   `escrow.sub_account` (testnet uses `escrow2` for the build with the NV-02 fix). The
   record of the old escrow moves to `retired_escrows`; the old account stays as it is.
6. Token storage for the escrow and the Verifier (`storage_deposit`, paid by the owner).
7. Maker: create `maker.<owner>`, register it on the token, mint up to `token_target`
   (stand-in token only), and add escrow storage funds (`add_storage_funds`) up to
   `storage_funds_near`.
8. `verify`.

Every transaction is written to the deployment file as it completes, with its hash,
block height and timings.

## Tests

```sh
cargo test -p zecret-deploy                                   # config, keys, mainnet and fund guards
cargo test -p test-usdc --lib                                 # token unit tests
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD cargo test -p test-usdc --test sandbox   # token in nearcore 2.14.1
LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD ./target/debug/zecret-deploy rehearse --config configs/sandbox.toml
cargo clippy --all-targets -- -D warnings
```

## The stand-in token

`test-usdc` is the standard `near-contract-standards` 5.30 NEP-141 implementation
(transfer, `ft_transfer_call`, `ft_resolve_transfer`, NEP-145 storage, NEP-148 metadata)
plus one method:

`mint(account_id?, amount)`, payable. Anyone can call it. It creates `amount` (at most
10 000 tokens, `10000000000` units) for `account_id` (default: the caller). If the
account is not registered, attach at least `storage_balance_bounds().min` (0.00125 NEAR)
and the call registers it; the rest is sent back.

Symbol `tUSDC`, name "Test USDC (testnet, no value)", 6 decimals.

Why not Circle's testnet USDC (`3e2210e1...b8af` on NEAR testnet, it exists and reports 6
decimals): its faucet is a web page with a per-address limit (reported as 20 USDC every
2 hours; not checked here), so a script cannot fund a maker with it. The stand-in token
uses the standard resolve code, so testnet shows the normal path. The USDC-specific cases
(pause and blocklist between deposit and resolve, review-1 NV-02) are tested in the
sandbox with the mainnet USDC code: `contracts/near-escrow/tests/usdc.rs` and
`docs/security/fixes-2.md`.
