# Deploying Zecret

This file says how to put Zecret's NEAR contracts on testnet and on mainnet, what each
step costs, how to check a deployment, and what a taker checks before she sends ZEC. The
tool is `deploy/tool` (`zecret-deploy`); its commands are in
[`deploy/README.md`](../deploy/README.md).

## What is deployed

| Part | Testnet (demo, no real money) | Mainnet |
|---|---|---|
| Escrow | `escrow.<owner>`: the reviewed `contracts/near-escrow` wasm, no access keys | same |
| Token | `usdc.<owner>`: the stand-in `deploy/test-usdc` (NEP-141, 6 decimals, open `mint`) | Circle's USDC, `17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1` |
| Intents Verifier | `intents.<owner>`: the code of mainnet `intents.near`, initialised by us | `intents.near` |
| Maker | `maker.<owner>`: holds the token, registered on the token, escrow storage funds added | same, funded by the owner |

NEAR Intents has no testnet. On testnet, the stand-in Verifier runs the same code as
mainnet `intents.near` (code hash `BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk`, read from
mainnet on 2026-11-13). The escrow pays into it and a signed `ft_withdraw` works there.
1Click and the solvers do not exist on testnet, so the swap to another chain does not run
there. The code is from `github.com/near/intents`, MIT licence (Copyright 2025 NEAR
Foundation; `LICENSE` read on 2026-11-13).

## The current testnet deployment

Made on 2026-11-13. The escrow was replaced the same day by a build with the NV-02 fix
(two-step Intents withdraw and `sweep`, [`docs/security/fixes-2.md`](security/fixes-2.md)).
Full record: [`deploy/deployment.testnet.json`](../deploy/deployment.testnet.json).

| Part | Account | Code hash |
|---|---|---|
| Owner | `zecret-3f109eea1e.testnet` | none |
| Escrow | `escrow2.zecret-3f109eea1e.testnet` | `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` |
| Token (tUSDC) | `usdc.zecret-3f109eea1e.testnet` | `5znPiqS6ocnsGYgeByDsED99dmFS9kv2b7S8dEaL7bRE` |
| Verifier | `intents.zecret-3f109eea1e.testnet` | `BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk` |
| Maker | `maker.zecret-3f109eea1e.testnet` | none |
| Retired escrow (before the NV-02 fix; do not use) | `escrow.zecret-3f109eea1e.testnet` | `D9YwW6hJoKEzWkr9XW3fGT7ByCiEQZoWhfb45Q3jA7Lz` |

The escrow was deployed by transaction `Bwt8ARb8UJA3YPmhHfFAhHaWwAtrmorTdD99mK5Srgmk`
(block 277465825): deploy, `new`, delete key, in one transaction signed by the escrow
account. It has no access keys. `verify` passed at final block 277465868.

The escrow wasm is 525 881 bytes. It was built with rustc 1.97.0 and the escrow's
`Cargo.lock`. A second build from a clean copy of the sources in another folder gave the
same code hash.

The retired escrow (deployed by `EATa9Bwc3AcxEguREmV6H55MzhjnyDg4bQXwHxzYqapH`, block
277459936, escrow commit `e257d75`) has no keys and cannot be changed or removed. Its
tUSDC balance was 0 at a final block after the replacement (both of its swaps were
claimed and paid out). It is listed under `retired_escrows` in the
deployment file. A taker checks the code hash, so she does not fund a swap on it.

## Testnet

### Steps

```sh
cd /home/user/zecret && source .tools/env.sh
(cd contracts/near-escrow && cargo build --target wasm32-unknown-unknown --release)
cd deploy
cargo build -p test-usdc --target wasm32-unknown-unknown --release
cargo build -p zecret-deploy
./target/debug/zecret-deploy deploy --config configs/testnet.toml
./target/debug/zecret-deploy e2e    --config configs/testnet.toml
```

The first `deploy` makes an owner account through the testnet faucet and records it in
`deployment.testnet.json`. Later runs use that owner. To start a separate deployment,
move `deployment.testnet.json` away. To deploy a new escrow build next to an old one,
change `escrow.sub_account` in the config: a deployed escrow cannot change. The tool then
moves the old escrow's record to `retired_escrows` (with `retired_at` and `replaced_by`).
`configs/testnet.toml` uses `escrow2` for the build with the NV-02 fix.

If the network drops a transaction (seen once, on a faucet account delete), the tool
says so after 40 s. Run the same command again. Each step checks the chain first.

### Costs (testnet NEAR, from the faucet)

The faucet gives 10 NEAR per account. The deployment needs about 24.2 NEAR, so the tool
used the owner account plus 2 faucet accounts (30 NEAR).

| Item | NEAR | Measured |
|---|---|---|
| Verifier account: 1 299 684 bytes of storage + 0.5 margin | 13.47 | yes |
| Escrow account: 474 069 bytes of storage + 0.2 margin (locked: the account has no keys), first escrow | 4.94 | yes |
| Second escrow (NV-02 fix), 525 881-byte wasm + 0.2 margin (locked) | 5.46 | yes (tool estimate; owner paid it from 6.30 NEAR) |
| Token account: 297 400 bytes + 0.3 margin | 3.27 | yes |
| Maker account | 2.00 | yes |
| Maker's escrow storage funds (`add_storage_funds`) | 0.50 | yes |
| Token registrations, 0.00125 each | 0.005 | yes |
| Gas for all deployment transactions (about 210 Tgas) | 0.02 | yes |
| Each end-to-end run: stipend 0.03, storage of one swap about 0.006, gas | about 0.04 | yes |

## Mainnet

Only the owner runs this. It spends real NEAR. The tool refuses to send any mainnet
transaction unless the command has `--i-am-the-owner-and-accept-spending-real-funds`.
This tool has not been run on mainnet.

### Before

1. Read [`docs/security/fixes-2.md`](security/fixes-2.md). The escrow build with the
   NV-02 fix pays a taker credit with `mt_transfer` from the escrow's own Intents
   account, and the USDC-specific cases (pause and blocklists between deposit and
   resolve) pass in the sandbox with the USDC code from mainnet. Its section 5 lists
   what is not covered, including the residual case (an Intents deposit failure and a
   USDC restriction in the same window). Deploy only a build whose code hash includes
   this fix (`DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` at the time of writing).
2. Pick an owner account that you control, for example `zecret.near`. The escrow will be
   `escrow.zecret.near`. Use a named account, not an implicit hex account (the commit
   message tells networks apart only by the escrow account id).
3. Put its full-access key in `deploy/.keys/mainnet/<owner>.json`, in the near-cli
   credentials format (`{"account_id": ..., "public_key": "ed25519:...",
   "private_key": "ed25519:..."}`), mode 600. The folder is ignored by git.
4. Set `owner_account` in `deploy/configs/mainnet.toml`. Check the rest of the file:
   token `17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`,
   Verifier `intents.near`, `maker.initial_balance_near`, `maker.storage_funds_near`.
5. Build the escrow from the reviewed commit. Write down its code hash:
   `./target/debug/zecret-deploy hash ../contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm`.

### Run

```sh
./target/debug/zecret-deploy deploy --config configs/mainnet.toml \
    --i-am-the-owner-and-accept-spending-real-funds
./target/debug/zecret-deploy verify --config configs/mainnet.toml
```

The tool creates `escrow.<owner>` and `maker.<owner>`, deploys the escrow in one
transaction that deletes its only key, registers the escrow and the maker on USDC if
needed (`intents.near` is already registered on USDC; read on 2026-11-13), and adds the
maker's storage funds. It does not move USDC. Send USDC to the maker yourself; `verify`
shows its balance.

Then publish the escrow account id and code hash where takers can see them (the maker
directory and the website), and keep `deployment.mainnet.json`.

### Costs (real NEAR)

Gas price on mainnet was 10^8 yoctoNEAR per gas on 2026-11-13 (0.0001 NEAR per Tgas).
Storage is 1 NEAR per 100 kB.

| Item | NEAR | Note |
|---|---|---|
| Escrow code storage (about 527 000 bytes: the 525 881-byte wasm plus state) | 5.27 | Locked forever: the account has no keys. Estimate from the wasm size; the first escrow used 474 069 bytes for a 472 515-byte wasm. |
| Escrow margin (`escrow.margin_near`) | 0.20 | Locked forever. Covers the deploy transaction's prepaid gas. |
| Maker account (`maker.initial_balance_near`) | 1.00 | Stays with the maker. Pays its gas and the 0.03 NEAR stipends. |
| Maker storage funds (`maker.storage_funds_near`) | 0.10 | Pays the storage of about 15 swaps (about 0.006 NEAR each, measured on testnet). Can be withdrawn with `withdraw_storage_funds`. |
| USDC registration of the escrow and of the maker | 0.0025 | 0.00125 each (USDC `storage_balance_bounds`, read 2026-11-13). |
| Gas for the deployment (about 60 Tgas, from the testnet run) | 0.006 | |
| **Total from the owner** | **about 6.58** | of which about 5.47 is locked in the escrow |

Each swap then costs the maker: the 0.03 NEAR stipend to the taker key (most of it stays
on that key; commit and claim cost about 0.002 NEAR), about 0.006 NEAR of escrow storage,
and about 0.005 NEAR of gas for open, `set_ready` and `withdraw`. The maker attaches 200
Tgas to `withdraw` (the two-step Intents withdraw needs at least 110 Tgas); only the gas
burnt is paid.

## How to check a deployment

```sh
./target/debug/zecret-deploy verify --config configs/<network>.toml
```

It reads at a final block and spends nothing. It checks:

- the escrow's code hash equals the hash of the local wasm (and the recorded one);
- the escrow has no access keys;
- `get_intents_account()` returns the configured Verifier;
- the token reports the configured decimals; the escrow, the Verifier and the maker are
  registered on it; on testnet, the token's and the Verifier's code hashes equal the
  local wasm files;
- the Verifier answers `current_salt`;
- the maker exists, its token balance, its escrow storage funds.

Output on testnet, 2026-11-13 (the block and the balances change over time):

```text
verify testnet at final block 277471610 (4Fyruberb4M6eSxGe3RgWD2DJ9LKbzwucEqgXyPykDDq)
escrow escrow2.zecret-3f109eea1e.testnet (local wasm 525881 bytes, code hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1)
  [PASS] escrow code hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1 equals the local wasm
  [PASS] escrow code hash equals the recorded DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1
  [PASS] escrow has no access keys (found 0)
  [PASS] escrow get_intents_account = "intents.zecret-3f109eea1e.testnet" (expected intents.zecret-3f109eea1e.testnet)
  [PASS] token usdc.zecret-3f109eea1e.testnet has 6 decimals (symbol "tUSDC")
  [PASS] token code hash 5znPiqS6ocnsGYgeByDsED99dmFS9kv2b7S8dEaL7bRE equals local test-usdc wasm
  [PASS] escrow2.zecret-3f109eea1e.testnet is registered on the token
  [PASS] intents.zecret-3f109eea1e.testnet is registered on the token
  [PASS] maker.zecret-3f109eea1e.testnet is registered on the token
  [PASS] Verifier intents.zecret-3f109eea1e.testnet code hash BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk equals local intents.near wasm
  [PASS] Verifier current_salt = "1aa14f40"
  [info] maker maker.zecret-3f109eea1e.testnet holds 0.83401 NEAR
  [PASS] maker token balance 19996000000 (target 20000000000)
  [PASS] maker escrow storage funds 0.48552 NEAR
ALL CHECKS PASSED
```

The first `[PASS]` line shows that a fresh build of the escrow gives the code hash of the
deployed escrow. The build took about 2.5 min and `verify` 3 s on the build machine.

The same checks by hand, with any NEAR RPC (example for testnet):

```sh
RPC=https://rpc.testnet.fastnear.com
ESCROW=escrow2.zecret-3f109eea1e.testnet
# 1. Code hash: compare with the hash of the reviewed wasm (zecret-deploy hash FILE).
curl -s $RPC -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"query",
 "params":{"request_type":"view_account","finality":"final","account_id":"'$ESCROW'"}}'
# 2. Access keys: the list must be empty.
curl -s $RPC -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"query",
 "params":{"request_type":"view_access_key_list","finality":"final","account_id":"'$ESCROW'"}}'
# 3. Intents account: args_base64 of "{}" is e30=; the result is the account id as JSON bytes.
curl -s $RPC -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"query",
 "params":{"request_type":"call_function","finality":"final","account_id":"'$ESCROW'",
 "method_name":"get_intents_account","args_base64":"e30="}}'
```

## What a taker checks before she sends ZEC

The taker software makes these checks at a final block. If one fails, she does not fund.

1. The escrow account's code hash equals the published hash of the reviewed build.
2. The escrow account has no access keys. With a key, its holder could replace the code
   after `claim` publishes `a`, and keep both the ZEC and the tokens.
3. `get_intents_account()` returns `intents.near` on mainnet (the stand-in Verifier on
   testnet).
4. The swap at its id (`get_swap`) has the agreed maker, token, amount, taker key, `A`,
   `B`, times and reveal window.
5. The token is the expected one (USDC on mainnet).

`zecret-deploy e2e` makes checks 1 to 3 (and the token checks) before it opens its swap.
The browser taker must make them too; `TakerConfig.escrow_code_hash` in
`crates/zecret-core` holds the hash it compares with.

## Measured on testnet

Runs 1 and 2 used the retired escrow `escrow.zecret-3f109eea1e.testnet`. Run 3 uses
`escrow2.zecret-3f109eea1e.testnet` (NV-02 fix) and is listed after them.

`zecret-deploy e2e`, two runs on 2026-11-13 at 04:16 and 04:17 UTC, through
`rpc.testnet.fastnear.com`, from this build machine. Each run: maker sends the 0.03 NEAR
stipend to a new taker key; maker opens a swap of 1 tUSDC (`ft_transfer_call`);
`set_ready`; the taker key sends `commit_claim` (10 Tgas attached) and `claim` (25 Tgas);
the maker sends `withdraw`, which pays the taker inside the Verifier; the taker key signs
one `ft_withdraw` (built with `crates/zecret-intents`, `raw_ed25519`); the maker submits
it with `execute_intents`; the tokens reach `zecret-3f109eea1e.testnet`. All hashes are
in [`deploy/e2e.testnet.json`](../deploy/e2e.testnet.json).

Run 1 (swap `0188df35...a187`, taker `dd704294...1279`):

| Step | Transaction | Block | Tgas | Included | Final |
|---|---|---|---|---|---|
| stipend 0.03 NEAR | `14FxQQp46HjtttRwmyhcV9TW89cUybhRFqztXD2FWJN3` | 277460057 | 8.57 | 1.6 s | 4.3 s |
| open (`ft_transfer_call`) | `BP2fYuuvmgFcvVERLLydQGEP5LMQ2AZPCnrbq21UZt7y` | 277460065 | 12.18 | 2.0 s | 5.6 s |
| `set_ready` | `2F7Km5cS5vtzEdqLNUuDS1kmzRt8SzB2YNBgM1mTm5Be` | 277460074 | 2.53 | 1.2 s | 3.9 s |
| `commit_claim` (taker) | `FH7cLU2Td8eoQAbVYHxKrv75fkCZquKhUowJHy4eY6EL` | 277460082 | 10.14 | 1.6 s | 3.9 s |
| `claim` (taker) | `9a5FM8R7L5Zi7qakxHf7TiAg9hR5mpbuFpWrfDkzWicK` | 277460089 | 24.37 | 1.7 s | 5.1 s |
| `withdraw` into the Verifier | `sj3HAE4ZB3zBn4NzqWaMbndGSd7UR19reddH7D3YBvW` | 277460098 | 13.98 | 2.0 s | 6.9 s |
| `execute_intents` (`ft_withdraw`) | `Hodyh7KyA8GtKn84KqvZVKdAsujPyf8bWDcE5aVi3oxS` | 277460117 | 11.35 | 0.9 s | 4.8 s |

Run 2 (swap `a62ea4a6...5ae8`): stipend `7EGbtSHYv4mbSLaDeUHMk4yTT2PaYacobNCRB85RFDJn`, open
`7XRLkV59cco3bbkHzkwm7XqnMYYR1AjvHzuf3D28xLfk`, `set_ready`
`4uRH7YUNoF3X1dcNt9ga9UMoioFnKWutYkVBsZTEmUg8`, `commit_claim`
`BgfVF67SuuF6CU4M7h5n1q42jC2VP5tDKYi59gJkRNeU`, `claim`
`43w5G3mZoiMYwCGFg5ZTJK1BcpFcMnLHneWDM7oJ1EeK`, `withdraw`
`6ftHVzRZkW8TwZPdmiYpJuM3cH8zazv8DEJDZmmggdeA`, `execute_intents`
`AuSJ2j4jfKjB4jjzxEU5NxfzDC4NivwRAKePG693xmsE`.

Run 3 (2026-11-13 05:14 UTC, escrow `escrow2.zecret-3f109eea1e.testnet`, swap
`5d9f0899...0706`, taker `4c98a781...d770`), all receipts successful:

| Step | Transaction | Block | Tgas | Included | Final |
|---|---|---|---|---|---|
| stipend 0.03 NEAR | `DCgWZNgQXvnH5KLfiKQ8jHeiQXkSwP1UHvEwuK72KdMa` | 277465882 | 8.57 | 1.6 s | 4.1 s |
| open (`ft_transfer_call`) | `EUHDYSjCXfjWLteoM4FrCBRm1SDvwk1HdLQVi6YEi1Q7` | 277465889 | 12.45 | 1.2 s | 5.3 s |
| `set_ready` | `ErgGucmjf7Sh5hkahBMZvaWQ8AyHowzWHJyx47YyNCb2` | 277465899 | 2.59 | 1.6 s | 4.5 s |
| `commit_claim` (taker) | `GAjCc38PL8w93pRh6nZdMxpDRo87uLTEcpwgPU6UuZfX` | 277465907 | 10.20 | 1.6 s | 4.2 s |
| `claim` (taker) | `3WEk6ARNuYFxQ9NK38ECWvoe4ghAtrcRPaakLuKqKET4` | 277465914 | 24.44 | 1.2 s | 3.8 s |
| `withdraw` (two steps, 200 Tgas attached) | `E1FSqhzLbWbBpitqXaCnFNCnN8VQZYQ7FEaKsDs64vqb` | 277465921 | 21.65 | 1.6 s | 8.0 s |
| `execute_intents` (`ft_withdraw`) | `5wiuNp2LzCheAL2ipNnBD1fvk3wRwk26sNhKZb2hNYEA` | 277465936 | 11.35 | 1.6 s | 5.3 s |

After the withdraw, the escrow's own Verifier balance was 0 (the tool checks it). The
two-step withdraw takes more receipts: 8.0 s to all receipts final, against 6.9 s in run
1. One sample.

Timings over the 14 transactions of runs 1 and 2. The clock starts before the tool reads the nonce and
signs; the status is polled every 0.2 s with `tx` and `wait_until: NONE`.

| From submit to | Min | Median | Max |
|---|---|---|---|
| included in a block (status `INCLUDED` or later) | 0.9 s | 1.6 s | 2.0 s |
| the transaction's block final (`INCLUDED_FINAL` or later) | 2.4 s | 2.9 s | 3.3 s |
| all receipts executed in final blocks (`FINAL`) | 3.9 s | 4.5 s | 6.9 s |

These figures are lower than the 7.6 s / 10.0 s in `docs/DESIGN.md` §7 (2026-11-12). That
measurement used a different method (`docs/experiments/near-timing.md`); the cause of the
difference was not investigated. Both are small samples from one machine and one RPC
provider.

Other measurements from these runs:

- The taker key held 0.02805 NEAR of its 0.03 NEAR stipend after `commit_claim` and
  `claim`.
- `withdraw` into the Verifier burnt 14.0 Tgas with the stand-in token on the retired
  escrow (42.4 Tgas in the sandbox tests with `wrap.near`), and 21.65 Tgas on the escrow
  with the NV-02 fix (64.3 Tgas with `wrap.near` and 21.0 Tgas with the USDC code in
  the sandbox).
- Two swaps reduced the maker's escrow storage funds from 0.49874 to 0.48643 NEAR,
  about 0.0062 NEAR per swap.

## Running the maker as a service

See [`deploy/maker/README.md`](../deploy/maker/README.md). In short: build the image from
`deploy/maker/Dockerfile`, export the maker key with `zecret-deploy maker-key`, write
`config.json`, and run the container with that folder as `/data`.

The testnet maker with its relay is hosted on Railway from
`deploy/maker/railway/Dockerfile`; see [`docs/HOSTING.md`](HOSTING.md).

## Hosting the website

The website is static (`web/`, Vite). `web/vercel.json` sets the build
(`deploy/web/vercel-build.sh`: the browser engine, then `npm run build`, output `dist`)
and security headers. It is hosted on Vercel; see [`docs/HOSTING.md`](HOSTING.md).

Build-time values (`web/src/vite-env.d.ts`):

| Value | Where | Default | Testnet value |
|---|---|---|---|
| Maker directory URL | `VITE_DIRECTORY_URL` at build time, or `?directory=` in the page URL | `/maker-directory.json` (a bundled sample) | the URL where the maker's directory JSON is served |
| 1Click API | `VITE_ONECLICK_URL` | `https://1click.chaindefuser.com` | the same (quotes only; there is no execution on testnet) |

The bundled sample `web/public/maker-directory.json` names
`maker.zecret-3f109eea1e.testnet` and `escrow2.zecret-3f109eea1e.testnet`; its Zcash
address and request key are sample values. The hosted site reads the hosted maker's
signed entry from `web/public/config/testnet-directory.json` (`docs/HOSTING.md`).

The browser engine reads these values from the network configuration
(`web/public/config/testnet.json`). They are the fields of `TakerConfig` in
`crates/zecret-core/src/config.rs`.

| Value | Testnet | Mainnet |
|---|---|---|
| Zcash network (`chains.network`) | `test` | `main` |
| Light-wallet server, gRPC-web (`chains.lightwalletd`) | `https://zcash-testnet.chainsafe.dev` (read from the browser in `docs/experiments/browser-wallet.md`; used by the native maker and taker in `docs/TESTNET.md`) | not chosen |
| NEAR RPC (`chains.near_rpc`) | `https://rpc.testnet.fastnear.com` | `https://rpc.mainnet.fastnear.com` |
| Escrow (`chains.escrow`) | `escrow2.zecret-3f109eea1e.testnet` | `escrow.<owner>.near` |
| Escrow code hash (`escrow_code_hash`) | `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` | the hash of the reviewed build |
| Token (`chains.token`) | `usdc.zecret-3f109eea1e.testnet` | `17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1` |
| Intents Verifier (`chains.intents`) | `intents.zecret-3f109eea1e.testnet` | `intents.near` |

The page connects to the 1Click API, the NEAR RPC, the light-wallet server and the
maker's relay. The Content-Security-Policy in `web/vercel.json` allows exactly these in
`connect-src`, and `wasm-unsafe-eval` in `script-src` for the WebAssembly engine
(`docs/WEB.md`, "Headers").

## Not verified

- The tool has not run on mainnet. The mainnet guard is tested (unit tests and a run
  without the flag that stops before any network access).
- The image of `deploy/maker/Dockerfile` itself was not rebuilt with the TLS fix. The
  Railway image (`deploy/maker/railway/Dockerfile`) was built and runs the daemon on
  testnet (`docs/HOSTING.md`).
- The website on mainnet. On testnet, one swap ran through the hosted website with these
  contracts (`docs/TESTNET.md`, "Hosted swap").
