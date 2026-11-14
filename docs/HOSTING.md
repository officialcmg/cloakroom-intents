# Hosting on test networks

The website and one maker with its relay are hosted for public use on Zcash testnet and
NEAR testnet. Nothing here runs on mainnet or moves real funds.

| Part | Host | Address |
|---|---|---|
| Website | Vercel, team "chrismg's projects", project `zecret` (`prj_Mb3xhOSaYxnYcc9f5PxFSRfUKLIX`) | https://cloakroom.chrismg.dev (custom domain for the name Cloakroom Intents, from 2026-11-14); https://zecret.chrismg.dev (custom domain, added 2026-11-13, kept); also https://zecret-chrismgs-projects.vercel.app |
| Maker and relay | Railway, workspace "Chrismg's Projects", project `zecret` (`7cef823e-d834-4c56-9826-e072a4abaa6e`), environment `production` (`e4b3b306-c4a0-45a5-a822-ba036dce67ec`), service `zecret-maker` (`2708fc53-4643-46fd-bec6-38c532afcd06`) | https://zecret-maker-production.up.railway.app/execute-intents |

`zecret.vercel.app` belongs to someone else. It is not this site.

## How a push to `main` redeploys

Both hosts are connected to the GitHub repository `officialcmg/zecret`, branch `main`. The owner renames it to `officialcmg/cloakroom-intents`; GitHub redirects the old name, and both hosts follow the repository by its id. After the rename, check that each host still shows the repository and deploys on a push to `main`. The project and service names on Vercel and Railway keep "zecret".
Nobody needs a Vercel or Railway login to redeploy: push to `main`.

### Website (Vercel)

- Project settings: root directory `web`, files outside the root directory included,
  production branch `main`, framework Vite.
- `web/vercel.json` sets the commands:
  - install: `npm ci`;
  - build: `bash ../deploy/web/vercel-build.sh`;
  - ignored build step: `bash ../deploy/web/vercel-ignore.sh`.
- `deploy/web/vercel-ignore.sh` skips the build in two cases: the commit message starts
  with "WIP", or nothing changed since the last deployment of the branch under `web/`,
  `crates/zecret-wasm`, `crates/zecret-core`, `crates/zecret-intents`,
  `crates/Cargo.toml`, `crates/Cargo.lock`, `crates/.cargo` or `deploy/web/`. When it
  cannot find the previous deployment's commit, it builds.
- `deploy/web/vercel-build.sh` builds the browser engine on Vercel, then the site:
  1. installs `clang` and `llvm` with `dnf` (the engine has C code that is compiled for
     wasm32);
  2. installs Rust 1.97.0 with the `wasm32-unknown-unknown` target (rustup, minimal
     profile);
  3. downloads the wasm-bindgen 0.2.129 release binary and checks its SHA-256;
  4. runs `crates/zecret-wasm/build.sh` (output `web/public/engine/`);
  5. runs `npm run build` with `VITE_ZECRET_CONFIG=testnet`.

  So the production site loads the real engine on testnet by default. `?demo=1` still runs
  the simulated engine. The engine files are built from the commit that is deployed;
  they are not in git.

Why this way: Vercel's build image (Amazon Linux 2023) has `dnf` and runs the build as
root, and a build may take up to 45 minutes. Building the engine there keeps one source of
truth (the commit) and needs no extra secret. The other ways were not chosen: committed
engine files can drift from the source; a GitHub Actions workflow would need a Vercel
token as a repository secret.

- The 1Click proxy: the functions in `web/api/oneclick/v0/` are part of the same project.
  They forward the site's 1Click calls and add the partner key from the environment
  variable `ONECLICK_JWT` (Production only, Secret). Without the variable they forward
  with no key. The configurations point at the proxy only after it is checked with a key:
  `docs/ONECLICK_PROXY.md`.

### Maker and relay (Railway)

- Service settings: source `officialcmg/zecret`, branch `main`, Dockerfile
  `deploy/maker/railway/Dockerfile`, watch path `/deploy/maker/**` (since 2026-11-13;
  before that also `/crates/**`), restart policy "always", one replica.
- A push to `main` that changes a file under `deploy/maker/` builds the image and
  restarts the maker. Other pushes do not, also when they change `crates/`. The release
  rule: to ship maker code or the maker's settings, add one line to
  `deploy/maker/RELEASE` (the date and what is released) in the same commit
  (`STATUS.md`, "Maker deploys"). The image always builds the whole commit, so a release
  also ships every earlier change under `crates/`.
- The image builds `zecret` (`crates/zecret-node`) in release mode and copies the
  repository's testnet maker settings (`crates/zecret-node/config/testnet-maker.json`) with
  `chains.birthday` set to 4,614,922.
- The start script `deploy/maker/railway/start.sh` prepares the volume and then runs
  `zecret maker --dir /data --relay 0.0.0.0:$PORT` as the unprivileged user `zecret`
  (mainnet: `--health 0.0.0.0:$PORT` instead of the relay).
- The volume `zecret-maker-data` (1 GB) is mounted at `/data`. It holds the wallet key,
  the NEAR key, the request key, the swap records, the refusal records and the relay
  counters. A redeploy keeps it.
- The start script never overwrites the wallet, the request key, the swap records or the
  relay counters. It replaces `config.json` with the image's settings only when no swap is
  open; with an open swap it keeps the old settings, because the maker uses them for that
  swap's deadlines. One exception (since 2026-11-14): when only `chains.near_rpc` or
  `polling` differ, it applies the new settings also with a swap open, because they do
  not change a swap's terms and an open swap needs working endpoints most. An open swap is a file `swap/*.json` whose `stage` is a string other
  than `Swept` or `Refunded` (`Aborted` is stored as an object and does not count; a
  request in `Quoting` or `Quoted` counts until the maker drops it, at most
  `quote_ttl_secs`, 1 hour by default). The old settings are kept as
  `config.json.prev-<time>`. The wallet, `near_key.json`, `rfq_key.json`, `seen.json`,
  the swap records and the relay counters stay as they are, so the maker's Zcash address,
  NEAR key and signed directory entry do not change.
- Since the switch to the light-payout escrow (`docs/LIGHT_PAYOUT_SWITCH.md`) the maker
  keeps its test USDC in its own account in the stand-in Verifier
  (`intents.zecret-3f109eea1e.testnet`) and deposits the difference from its plain
  balance before each open.
- NEAR RPC endpoints (since 2026-11-14, incident
  `docs/incidents/2026-11-14-maker-missed-refund.md`): `chains.near_rpc` lists
  `https://rpc.testnet.fastnear.com`, `https://near-testnet.drpc.org` and
  `https://testnet-rpc.intea.rs`, in that order; the website's `near.rpc` lists the same
  three. Each answered `block`, a `get_swap` view and `tx` from the build machine on
  2026-11-14, and each checked a signed `send_tx` from a key without an account up to
  `SignerDoesNotExist` (so a real transaction would be processed). drpc refuses the
  user agent `Python-urllib` (HTTP 403); the maker's client sends none.
  `rpc.testnet.near.org` and `test.rpc.fastnear.com` are run by fastnear too (same
  addresses), so they are not a second operator; `api.zan.top/near-testnet` answered a
  `tx` read with "cu limit exceeded" at once; the lava endpoints are discontinued (HTTP
  410). None of the free endpoints publishes its rate limit.
- Health: `GET https://zecret-maker-production.up.railway.app/health` (the relay's port)
  answers the maker's report as JSON: when NEAR and Zcash last answered, the NEAR
  endpoints and their back-off, open swaps by stage, the nearest deadline and the active
  alerts (`docs/ENGINE.md`, section 4). HTTP 200 with `"ok": true` when all is well, 503
  otherwise. Swap `c2b9fa50` will stay in the list as `past_t2` (it does not make `ok`
  false). It shows no key, address or amount.
- The relay answers on Railway's HTTPS domain. Its CORS header is
  `Access-Control-Allow-Origin: *`. It refuses to start on mainnet
  (`crates/zecret-node/src/main.rs`, `run_maker`), and the start script refuses any
  `chains.network` other than `test`.

## Secrets

No secret is in git or in a log.

| Secret | Where it is | Who can read it |
|---|---|---|
| Maker wallet key (`wallet.json`) | Railway volume `zecret-maker-data`, `/data/wallet.json` | Railway project members (shell into the service) |
| Maker NEAR key (`near_key.json`) | Railway volume, `/data/near_key.json` | same |
| Maker request key (`rfq_key.json`) | Railway volume, `/data/rfq_key.json` | same |
| NEAR keys of the testnet deployment | `deploy/.keys/testnet/` on the build machine (not in git) | the build machine |
| 1Click partner key (`ONECLICK_JWT`), when the owner sets it | Vercel project → Settings → Environment Variables, Production, Secret | Vercel project members can replace it, not read it; the functions in `web/api/oneclick/v0/` (`docs/ONECLICK_PROXY.md`) |

The hosted maker made its own wallet and its own NEAR key on the volume at its first
start. Only the public values were printed: the wallet's address, the NEAR public key and
the signed directory entry. The NEAR public key,
`ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH`, is a full-access key of
`maker.zecret-3f109eea1e.testnet`. It was added on 2026-11-13 in NEAR testnet transaction
`BTpC5EAxUnFxUcMHSfvWJBFqXpky8nkHECgZK6v939fK`, with these commands on the build machine
(they sign with the existing maker key in `deploy/.keys/testnet/` and do not print it):

```sh
cd /home/user/zecret && source .tools/env.sh
(cd testnet/tools && CARGO_TARGET_DIR=../../crates/target cargo build --release --bin zecret-testnet-addkey)
crates/target/release/zecret-testnet-addkey \
  deploy/.keys/testnet/maker.zecret-3f109eea1e.testnet.json \
  maker.zecret-3f109eea1e.testnet ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH
```

### Start command

In the final state the service has no custom start command. The image's entrypoint runs
`deploy/maker/railway/start.sh`. With `ZECRET_NEW_NEAR_KEY=0` that script refuses to make
a new NEAR key on an empty volume.

The first deployments ran from commit `d6bac89`, whose start script cannot make a NEAR
key. For them, a custom start command was set on the Railway service. It made the key,
printed the public values and then ran the image's entrypoint.

That custom start command was cleared on 2026-11-13. Since then the service runs the
image's entrypoint (`deploy/maker/railway/start.sh`).

Railway variables of the service:

| Variable | Value | Purpose |
|---|---|---|
| `PORT` | `8080` | The relay port; the Railway domain routes to it. |
| `ZECRET_NEW_WALLET` | `0` | `1` lets the start script make a new wallet when the volume has none. Keep `0`: a lost volume then stops the service instead of starting with an empty wallet. |
| `ZECRET_NEW_NEAR_KEY` | `0` | The same for the NEAR key. |
| `RUST_BACKTRACE` | `1` | Backtraces in the log. |
| `ZECRET_NEAR_KEY` | not set | Optional. A NEAR key `ed25519:...` for the maker account. When set and different from the volume's key, the start script writes it and keeps the old file as `near_key.json.prev-<time>` (key rotation). |
| `ZECRET_MAKER_STATE` | not set | Optional. Base64 of a `tar.gz` of a maker folder (`wallet.json`, `rfq_key.json`, `seen.json`, `swap/`, `relay/`). Used only when the volume has no wallet. |

### Rotation

- NEAR key: make a new key, add it to `maker.zecret-3f109eea1e.testnet` as a full-access
  key, set `ZECRET_NEAR_KEY` on the service in the Railway dashboard (Variables), let it
  redeploy, check the log line `NEAR public key: ...`, then delete the old key from the
  account. Publish a new directory entry (below): the entry names the NEAR key.
- Wallet or request key: they cannot be rotated in place while a swap is open. Stop the
  service when no swap is open (log line `open swaps: 0` at start), move the funds out with
  `zecret pay`, replace the volume's files, start it, and publish a new directory entry.

### The directory entry

The website reads the maker's signed entry from `web/public/config/testnet-directory.json`.
To make it again after a key or limit change, copy the line `directory entry: {...}` from
the service log and run:

```sh
node deploy/web/directory-from-entry.mjs entry.json 30 > web/public/config/testnet-directory.json
```

## How to stop the services

- Maker: remove the service's active deployment, or remove the service `zecret-maker`.
  Either stops the maker and its relay. The volume `zecret-maker-data` holds the wallet
  key and the NEAR key. It stays until it is deleted. Deleting the volume loses the
  wallet and its test funds.
- With an open swap, a stopped maker lets the deadlines run out. The taker takes its ZEC
  back after `t1`. The maker must run again before `t0 + t1 + t2` to take back its
  tokens.
- Website: pause or delete the Vercel project `zecret`.

## What the owner pays for

- Railway: the service runs all the time, and it has a 1 GB volume. Both are billed
  against the workspace plan's usage. CPU and memory use were not measured.
- Vercel: one static site on the team's plan. Each build that compiles the engine takes
  several minutes of build time.
- No domain was bought. No paid add-on was turned on.
- Testnet NEAR and testnet ZEC have no value.

## Moving to mainnet later (not done)

The runbook is [`docs/MAINNET.md`](MAINNET.md). It adds a second Railway service
(`zecret-maker-mainnet`, mainnet mode of the start script) and a second Vercel project
(`zecret-mainnet`, built with `VITE_ZECRET_ALLOW_MAINNET=1`); the testnet services stay
as they are. The list below is the short form.

This is a list, not a plan that was run.

1. Deploy the escrow on mainnet (`docs/DEPLOY.md`, about 6.6 NEAR).
2. Build the site with `VITE_MAINNET_ESCROW` and `VITE_MAINNET_ESCROW_CODE_HASH`, and add a
   mainnet configuration with the 1Click exit; the relay is not used on mainnet.
3. Run the maker without `--relay`, with `chains.network` `main`, `premium_check` set, and
   a mainnet NEAR account funded with USDC and NEAR. The Railway start script refuses
   mainnet; a mainnet service needs its own start script and its own volume.
4. Add the mainnet RPC and light-wallet hosts to the CSP in `web/vercel.json`.
5. Owner approval is required for each of these steps.

## Setup record

See "Hosted setup" in `docs/TESTNET.md` for what was run, the transaction ids and what was
checked.
