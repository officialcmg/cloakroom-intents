# Going to mainnet

This is the runbook for the owner. Follow it from top to bottom. Each step says what to
run, what you should see, and what to check on chain. Nothing in this file was run on
mainnet with funds. Everything that could be checked without funds was checked on
2026-11-14 between 02:50 and 03:25 UTC; the outputs below are from those runs.

Read the gap list first. It says what is still untested at the moment of the first swap.

## 0. Gap list: untested at the first mainnet swap

Each item says what is not tested, why, and how to reduce the risk before or during the
first swap.

| # | Not tested | How to reduce it |
|---|---|---|
| G1 | **The 1Click delivery leg with funds.** Quote, generate and submit answered without a key, and the quote signature and payload checks pass on live answers (`docs/INTENTS.md`). No signed exit was ever executed with a funded key. The real fees, the status sequence of a funded exit, the refund path and the time to deliver are not known. Today's dry quote: 5 USDC on NEAR to USDC on Base gives 4.997080 USDC, minimum 4.947109, estimate 12 s. | First swap: the smallest amount (0.002 ZEC, about 1.8 USDC) to USDC on Base, to an address you control. If the exit fails, the USDC stays in `intents.near` under the swap's key; the recovery file holds that key. Keep the recovery file until the USDC arrives. |
| G2 | **Zcash mainnet transactions from this code.** The wallet scanned Zcash mainnet (native: 262 blocks to the tip 3,570,962 in 0.9 s; browser engine: read the tip 3,570,968 through gRPC-web). It never built, proved or broadcast a mainnet transaction. The consensus parameters for mainnet are new in this branch: `zcash_protocol` 0.11.0-pre.0 has no mainnet NU7 height, so `crates/zecret-core/src/wallet.rs` sets 3,543,405. The chain agrees: the coinbase of block 3,543,404 has branch id `0x37a5165b` (NU6.3), that of 3,543,405 has `0x77190ad9` (NU7), and the light-wallet server reports `77190ad9` at the tip. | The first Zcash send of the maker (its first quote) shows it: a node refusal says why. Before that, send 0.001 ZEC from the maker's wallet to your own wallet with `zecret pay` (step 6.5). That proves a mainnet send without a swap. |
| G3 | **The escrow with Circle's USDC and the real `intents.near`.** Tested only in the NEAR sandbox with the code of both contracts copied from mainnet (`contracts/near-escrow/tests/usdc.rs`, `docs/security/fixes-2.md`). | The first swap is that test. The escrow's `withdraw` pays the taker inside `intents.near`; check the balance there (step 9). |
| G4 | **Light-wallet server under load.** One public server, `zcash-mainnet.chainsafe.dev` (Zebra 7.0.0, lightwalletd 0.6.1). Its CORS answer allows any origin (it echoed `https://zecret.chrismg.dev`, a made-up `https://mainnet.zecret.chrismg.dev` and `http://127.0.0.1:<port>`). How long ChainSafe keeps it public, and how it behaves with many users, is not known. | Small swaps, few at a time (`max_pending_quotes` 3). If it fails, every party's deadlines still hold: the taker's ZEC comes back through the refund path. |
| G5 | **Wallet apps as payers on mainnet.** The deposit address has one Orchard-protocol receiver, and since NU6.3 a wallet pays it only into the Ironwood pool. Zodl, Zingo, Zkool, Cake and Edge support Ironwood per their release notes (`docs/WEB.md`; not re-checked for this file). None was tested against this site. | Pay the first swap from the wallet you plan to recommend. If it cannot pay the address, use the command-line payer (step 8, "If the wallet app refuses"). |
| G6 | **Mainnet gas price against the stipend.** Read from mainnet RPC: block gas price `100000000` yoctoNEAR per gas (10^8). The protocol config reports `min_gas_price` `1000000000` (10^9), which equals the "pessimistic" price measured in the 2.14.1 sandbox; mainnet also runs nearcore 2.14.1. With NEAR's check at 10 times the gas price, `claim` (25 Tgas attached) needs 0.025 NEAR, plus about 0.0019 NEAR already burnt: 0.0269 NEAR of the 0.03 NEAR stipend. The stipend stays enough only up to a gas price of 1.124 × 10^8, **1.12 times today's price**. If NEAR's check uses a fixed 10^9, the gas price does not matter. Which one is true on mainnet was not tested: it needs a transaction. | The dry run prints this check with today's gas price (step 3). If the gas price rises above 1.12 × 10^8 before a swap, pause the maker. If a taker cannot claim, the maker refunds at its deadline and the taker takes the ZEC back (less fees and premium). |
| G7 | **The maker's fixed price.** `price_per_zec` is one number in `config.json`. The code has no price feed. A price above the market loses USDC on every swap. See section 3, "Price". | Use the price rule. Run the dry run with `--maker-config` every few hours: it fails when the price is less than 2 % below the market. Keep the float small. |
| G8 | **The hosted maker on mainnet.** The mainnet mode of `deploy/maker/railway/start.sh` is tested with a stand-in binary (18 cases, `deploy/maker/railway/test-start.sh`). The image with this code was not rebuilt or run on Railway. | Watch the first start's log lines (step 6.4). |
| G9 | **The website on mainnet.** The switched build passed its Playwright tests with all outside requests blocked and a stand-in escrow; it never ran a swap. The engine's maker check ran on mainnet only as far as a refused stand-in entry. | The first swap (step 8). |
| G10 | **The deployment tool on mainnet.** `zecret-deploy plan` ran read-only on mainnet. `deploy` never ran on mainnet; it ran on testnet and in the sandbox. | Step 4 runs it once; each step reads the chain first and can run again. |
| G11 | **One trusted NEAR RPC endpoint** (`rpc.mainnet.fastnear.com`) for the taker and the maker. A false endpoint that works with the maker can take the taker's ZEC (`docs/DESIGN.md` section 10). | Small swaps. For your own first swap the risk is yours. |
| G12 | **Legal and compliance questions.** Not assessed. They are for you to judge. Questions that apply: whether running a maker that buys ZEC for USDC needs a registration or licence where you live or where the service runs; sanctions screening of counterparties, which a shielded swap cannot do; whether the hosting providers' terms allow this use; tax treatment of the maker's trades and the premium; what the site must say to users in your jurisdiction. | Decide before you open the site to others. You can run the first swaps with the site's address shared with nobody. |

Smaller known gaps:

- The progress screen says "Each block takes about 75 seconds" while the swap waits for
  confirmations (`web/src/app/stageCopy.ts`). Since NU7 it is about 25 s on mainnet
  (24.9 s measured) and 20 s on testnet. This branch does not change it because the text
  is shared with testnet. The engine on mainnet now polls every 25 s
  (`realEngine.ts`, `blockMs`).
- `mainnet.zcashexplorer.app` and `nearblocks.io` (the explorer links in
  `web/public/config/mainnet.json`) answered HTTP 403 to `curl` from the build machine
  (bot protection, presumably). Not checked from a browser.
- `max_pending_quotes`: the maker does not check its USDC balance before it quotes. An
  open that the float cannot pay fails after the taker paid the premium. The taker then
  stops after `quote_timeout_secs` and gets the ZEC back, without the premium. The
  mainnet template keeps 3 requests at a time, and the float must cover 3 maximum swaps.

## 1. What mainnet adds

| Part | Testnet today | Mainnet |
|---|---|---|
| Zcash | testnet, `zcash-testnet.chainsafe.dev` | mainnet, `zcash-mainnet.chainsafe.dev` (same server software; NU7 active since 3,543,405; target 25 s per block, ZIP 218) |
| Token | stand-in `usdc.zecret-3f109eea1e.testnet` | Circle's USDC `17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1` (read on mainnet: symbol `USDC`, 6 decimals, code hash `DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q`, registration 0.00125 NEAR) |
| Verifier | stand-in with the code of `intents.near` | `intents.near` (code hash `BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk`, equal to the copy in `.tools/wasm`; `current_salt` `252812b3`; registered on USDC) |
| Exit | direct: `ft_withdraw` to a NEAR testnet account, sent by the maker's relay | 1Click: one signed `transfer` to a 1Click deposit address; no relay |
| Maker | Railway service `zecret-maker` with relay | a second Railway service, mainnet mode, no relay (section 5) |
| Website | `zecret.chrismg.dev`, testnet build | a second Vercel project on its own hostname, built with `VITE_ZECRET_ALLOW_MAINNET=1` (section 5) |

**Never run with funds anywhere** (exact list): the 1Click exit execution; any Zcash
mainnet transaction by this code; the escrow with the real USDC contract and the real
`intents.near`; the maker on mainnet; the browser engine on mainnet beyond reading the
chain tip; any wallet app as the payer; `zecret-deploy deploy` on mainnet; the Railway
mainnet mode in a real container; the mainnet website build against the real chains.

What this branch adds (all tested without funds, section 10):

- The Zcash wallet runs on mainnet (consensus parameters with NU7, `wallet_for` takes
  `network: "main"`).
- Mainnet configuration templates for the maker, the taker and the website, with timing
  from measured mainnet block intervals and tests of the paper's bounds.
- `zecret-deploy plan`, the dry run: every read-only check against mainnet, the plan and
  its cost.
- A mainnet mode of the hosted maker's start script.
- A build switch for the website, a real-money notice, mainnet landing copy, CSP hosts.
- `scripts/mainnet-preflight.sh`: every no-funds check in one run.

## 2. What you need ready

### 2.1 A NEAR mainnet account with about 8 NEAR

Use a named account (for example `zecret.near`; it did not exist on 2026-11-14). The
escrow becomes `escrow.<owner>` and the maker `maker.<owner>`.

Re-derived on 2026-11-14 from mainnet RPC (storage price from
`EXPERIMENTAL_protocol_config`, gas price from the final block, the escrow wasm size,
USDC's `storage_balance_bounds`), by `zecret-deploy plan`:

| Item | NEAR | Source |
|---|---|---|
| Escrow code storage: 525,881 bytes × 10^19 yoctoNEAR per byte | 5.25881 | wasm size; `storage_amount_per_byte` read on mainnet. Locked for ever: the escrow has no keys. |
| Escrow margin (`escrow.margin_near`) | 0.20000 | config. Locked. Pays the escrow's own deploy transaction. |
| Maker account (`maker.initial_balance_near`) | 1.00000 | config. The maker spends it: 0.1 escrow storage funds at deploy, then gas and stipends. |
| USDC registration of the escrow and the maker, 2 × 0.00125 | 0.00250 | `storage_balance_bounds` read on mainnet. `intents.near` is registered already. |
| Gas of the deployment, about 64.4 Tgas × 10^8 | 0.00644 | testnet gas per transaction × today's mainnet gas price |
| **Total from the owner** | **6.46775** | of which 5.46131 stays locked |

- `deploy` refuses to start unless the owner holds **6.95881 NEAR free** (its own
  bound: the two transfers plus 0.5 NEAR for gas and registrations).
- The maker then needs more NEAR to run: about 0.2 NEAR must stay free for NEAR's gas
  check of its 200 Tgas `withdraw`, and each swap costs it about 0.04 NEAR (0.03
  stipend, about 0.006 storage, about 0.005 gas). With 0.9 NEAR left after its storage
  funds it runs about 17 swaps. Send it 1 NEAR more after deploy (step 5).
- So have **8 NEAR** on the owner account: 6.96 for the deploy check, 1 for the maker's
  top-up, the rest for gas.
- This replaces the 6.58 NEAR estimate in `docs/DEPLOY.md`: that figure counted the 0.1
  NEAR storage funds as an owner cost; the tool pays them from the maker's 1 NEAR.
- If the escrow changes (branch `escrow-light-payout`), its wasm size changes the first
  line. The dry run recomputes it from the wasm you give it.

### 2.2 USDC float for the maker

The float is the USDC the maker holds on NEAR. It limits the loss from a wrong price and
from any bug on the maker's side.

- One swap at the maximum pays `max_zats × price_per_zec / 10^8` USDC units. With the
  template's `max_zats` 2,000,000 (0.02 ZEC) and a price of 890 USDC per ZEC: 17.8 USDC.
- The maker does not check its float before it quotes. Up to `max_pending_quotes` (3 in
  the template) requests can wait at once, so the float must cover 3 maximum swaps: 53.4
  USDC. **Send 60 USDC** (native USDC on NEAR, the contract above; not bridged USDC.e).
- A larger float allows a larger `max_zats` or more requests at once. Raise them later,
  together.
- The maker gets ZEC for the USDC it pays. To refill the float you sell that ZEC
  yourself.

### 2.3 ZEC for the maker's fees

The maker pays one Zcash fee for each quote and each refusal (ZIP-317: 10,000 zats for
up to 2 actions, 15,000 for 3) and gets the lot and the premium back in each finished
swap. **Send 0.01 ZEC** to the maker's address (step 6.3) before it quotes.

### 2.4 A shielded Zcash mainnet wallet to pay from

For the first swap you are the taker. You need a wallet that pays into the Ironwood pool
(gap G5) with about 0.01 ZEC: 0.002 ZEC for the swap, 25,000 zats premium, and the
temporary wallet's fees (the page shows the exact amount). Also have a USDC on Base (or
other chain) address of yours for the exit, and a Zcash address of yours for leftovers.

### 2.5 Where the keys live and how they are backed up

| Key | Made where | Lives where | Backup |
|---|---|---|---|
| Owner's full-access key | your wallet | your machine, `deploy/.keys/mainnet/<owner>.json` (mode 600, ignored by git) | your usual NEAR key backup |
| Escrow's temporary key | `zecret-deploy deploy` | deleted in the same transaction that deploys; the escrow has no keys | none needed |
| Maker's NEAR key (full access on `maker.<owner>`) | `zecret-deploy deploy`, in `deploy/.keys/mainnet/` | your machine, and the Railway variable `ZECRET_NEAR_KEY` | copy the key file to your offline backup |
| Maker's Zcash wallet key and request key (`wallet.json`, `rfq_key.json`) | your machine, `zecret directory --dir D` (step 6.2) | the Railway variable `ZECRET_MAKER_STATE`, then the service's volume | copy `D` to your offline backup before you start the service |

Keys made on your machine first have a backup from the start, and the Railway start
script never prints them. Anyone with access to the Railway project can read the
variables and the volume: that access can take the float, the maker's ZEC and its NEAR.
Keep the project's members to yourself. A lost volume in the middle of a swap is
recovered from your backup of `D` (the swap records on the volume are lost; the maker
then cannot finish or refund that swap; the taker's deadline paths still return the
taker's funds).

## 3. Decisions to make

### Swap limits

The template (`crates/zecret-node/config/mainnet-maker.json`) proposes:

| Setting | Template | Why |
|---|---|---|
| `min_zats` | 200,000 (0.002 ZEC, about 1.8 USDC) | Smaller swaps pay a premium of more than 12 % and are of no use. |
| `max_zats` | 2,000,000 (0.02 ZEC, about 18 USDC) | Real money, unaudited code. The first-run notice on the site shows this number. |
| `max_pending_quotes` | 3 | The float must cover this many maximum swaps. |
| `max_refusals_per_hour` | 6 | Each refusal costs a Zcash fee. |

### Price

`price_per_zec` (USDC base units for 1 ZEC) is a fixed number in `config.json`. **The
code has no price feed.** On mainnet a fixed price is a real money risk: if ZEC falls,
anyone can sell ZEC to the maker above the market until the float is gone.

The simplest safe rule, with what the code has today:

1. Take the market price from a 1Click dry quote of ZEC to USDC. The dry run prints it
   ("0.005 ZEC → USDC on Base: ... USDC per ZEC").
2. Set `price_per_zec` to that price **less 3 %**, rounded down. Example from the dry run
   at 03:23 UTC: market 918.81 USDC per ZEC, so `"price_per_zec": "890000000"`.
3. Put the same number in the website's directory (`rate.usdcPerZec`, in USDC: `890`).
4. Every few hours, and before you leave the maker alone, run the dry run with
   `--maker-config` (step 11). It fails when the price is less than 2 % below the market
   (`MIN_DISCOUNT_BPS`), and warns when it is more than 15 % below. When it fails,
   pause the maker (section 7) or set a new price.
5. A new price takes effect at a restart with no open swap (the start script keeps the
   old settings while a swap is open, because its deadlines depend on them).

What limits the loss: one swap at the old price loses at most `max_zats × (old price −
market)`, and all swaps together at most the float.

### Premium and stipend

- Stipend: 0.03 NEAR (measured; do not lower it). See gap G6.
- `premium_check` is required on mainnet: the maker refuses to start without it.
  The rule: `premium_zats × yocto_near_per_zec / 10^8 ≥ stipend + costs_per_request_yocto`.
- Template: `premium_zats` 25,000, `yocto_near_per_zec` 200 NEAR (a low estimate; the
  market was 273.5 NEAR per ZEC from 1Click's token prices: ZEC 924.59 USD, wNEAR 3.38
  USD), `costs_per_request_yocto` 0.015 NEAR. The premium is then worth 0.05 NEAR at the
  low estimate, against 0.045 NEAR of costs, and 0.068 NEAR at the market. The smallest
  premium that passes at 200 NEAR per ZEC is 22,500 zats.
- 25,000 zats is about 0.23 USDC: 12.5 % of a 0.002 ZEC swap, 1.25 % of a 0.02 ZEC swap.
- The dry run checks that `yocto_near_per_zec` is not above the market.

### NEAR RPC

`rpc.mainnet.fastnear.com` is in every mainnet file (maker, taker, website, deploy
config) and in the CSP. It answered `chain_id` `mainnet`, nearcore 2.14.1. Another
endpoint means changing all four files and the CSP (`web/vercel.json`); the CSP unit test
fails until both agree. The taker trusts this one endpoint (gap G11).

### Hostname

Run mainnet on its own hostname, for example `mainnet.zecret.chrismg.dev`, as a second
Vercel project, and keep `zecret.chrismg.dev` on testnet:

- The testnet site's build has no switch: a link cannot point it at mainnet
  (`?config=mainnet` shows "This site does not run on mainnet").
- Each origin has its own browser storage. On one origin, a testnet swap and a mainnet
  swap would share the engine's storage and its tab lock.
- One `web/vercel.json` serves both projects. Its CSP lists the hosts of both
  configurations (the unit test derives them from `testnet.json` and `mainnet.json`).

## 4. Before you start (on your machine, about 15 minutes)

Check out the branch with this work (after the merge, `main`) and set up:

```sh
cd zecret && ./scripts/setup-env.sh && source .tools/env.sh
(cd contracts/near-escrow && cargo build --target wasm32-unknown-unknown --release)
cd deploy && cargo build -p zecret-deploy && cd ..
T=deploy/target/debug/zecret-deploy
$T hash contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm
```

The hash must be the code hash of the escrow build you decided to ship. Today's escrow
(NV-02 fix) gives `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`, 525,881 bytes. If the
light-payout escrow ships instead, its hash is different; use that one everywhere below
as `ESCROW_HASH`.

Run the preflight (every no-funds check; about 10 minutes, most of it builds):

```sh
OWNER=<owner>.near ESCROW_HASH=<hash> scripts/mainnet-preflight.sh
```

It must end with `PREFLIGHT PASSED`. With `OWNER` unset it uses the existing account
`aurora` as a stand-in owner (read-only) and says so.

## 5. Steps

Set these once in your shell:

```sh
OWNER=<owner>.near                  # your account
ESCROW_HASH=<hash>                  # from step 4
T=deploy/target/debug/zecret-deploy
RPC=https://rpc.mainnet.fastnear.com
q() { curl -s $RPC -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":$1}"; }
view() { q '{"request_type":"call_function","finality":"final","account_id":"'$1'","method_name":"'$2'","args_base64":"'$(printf '%s' "$3" | base64 -w0)'"}' | python3 -c 'import json,sys; r=json.load(sys.stdin); print(bytes(r["result"]["result"]).decode() if "result" in r else r)'; }
```

### Step 1. The owner key and the config (2 min)

```sh
mkdir -p deploy/.keys/mainnet && chmod 700 deploy/.keys/mainnet
# near-cli credentials format: {"account_id":"<owner>","public_key":"ed25519:...","private_key":"ed25519:..."}
$EDITOR deploy/.keys/mainnet/$OWNER.json && chmod 600 deploy/.keys/mainnet/$OWNER.json
sed -i "s/^owner_account = .*/owner_account = \"$OWNER\"/" deploy/configs/mainnet.toml
```

Do not commit `deploy/configs/mainnet.toml` with your account unless you want it public
(it holds no key).

### Step 2. The dry run (1 min)

```sh
$T plan --config deploy/configs/mainnet.toml --expect-escrow-hash $ESCROW_HASH \
  --escrow-wasm contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm
```

It must end with `DRY RUN PASSED: READY TO DEPLOY (nothing was sent)`, or with `NO CHECK
FAILED; OWNER INPUTS MISSING` where the only missing input is the maker config (step 6).
The full output of a run on 2026-11-14 with the stand-in owner `aurora` and an example
maker config is in section 9.

### Step 3. Check the stipend line (now)

In the dry run, the line `0.03 NEAR covers commit and claim ... (stays enough up to a gas
price of 112400000, 1.12× today)` must be `[PASS]`. If it fails, do not continue (gap
G6).

### Step 4. Deploy the contracts (2 min; spends about 6.47 NEAR)

```sh
$T deploy --config deploy/configs/mainnet.toml \
  --escrow-wasm contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm \
  --i-am-the-owner-and-accept-spending-real-funds
```

It prints each transaction with its hash and ends with the `verify` checks. It writes
`deploy/deployment.mainnet.json` (no keys; keep it, commit it if you want the record
public). If a transaction is dropped, run the same command again.

Check on chain:

```sh
q '{"request_type":"view_account","finality":"final","account_id":"escrow.'$OWNER'"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["code_hash"])'   # = ESCROW_HASH
q '{"request_type":"view_access_key_list","finality":"final","account_id":"escrow.'$OWNER'"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["keys"])'  # = []
view escrow.$OWNER get_intents_account '{}'                       # = "intents.near"
view 17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1 storage_balance_of '{"account_id":"escrow.'$OWNER'"}'   # not null
view escrow.$OWNER get_storage_funds '{"account_id":"maker.'$OWNER'","token":"17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"}'   # about 0.099 NEAR
```

Or all at once: `$T verify --config deploy/configs/mainnet.toml --escrow-wasm ...` ends
with `ALL CHECKS PASSED`.

### Step 5. Fund the maker (5 to 30 min, depending on where your USDC is)

- Send **60 USDC** (native USDC on NEAR, contract
  `17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`) to
  `maker.<owner>`. The deploy registered it.
- Send **1 NEAR** to `maker.<owner>`.

Check: `view 17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1 ft_balance_of '{"account_id":"maker.'$OWNER'"}'`
shows `"60000000"`.

### Step 6. The maker's keys and settings (on your machine, 5 min)

6.1. Export the maker's NEAR key and build the maker program:

```sh
D=~/zecret-maker-mainnet
$T maker-key --config deploy/configs/mainnet.toml --out $D      # writes $D/near_key.json (mode 600)
(cd crates && cargo build --release -p zecret-node --bin zecret)
Z=crates/target/release/zecret
```

6.2. Write the settings and make the wallet. Take the birthday from the light-wallet
server just before (the current tip), and the price from step 2's dry run (section 3,
"Price"):

```sh
TIP=$(python3 -I scripts/zcash-block-intervals.py https://zcash-mainnet.chainsafe.dev --tip)
PRICE=890000000      # USDC units per ZEC: market less 3 %, from the dry run
sed "s/OWNER\.near/$OWNER/g" crates/zecret-node/config/mainnet-maker.json \
  | jq --argjson b "$TIP" --arg p "$PRICE" '.chains.birthday = $b | .price_per_zec = $p' > $D/config.json
$Z directory --dir $D > $D/entry.json     # makes wallet.json and rfq_key.json; prints the signed entry
$Z init --dir $D                         # prints the maker's Zcash address (u1...)
$T plan --config deploy/configs/mainnet.toml --expect-escrow-hash $ESCROW_HASH --maker-config $D/config.json \
  --escrow-wasm contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm
```

The last command must end with `DRY RUN PASSED: READY TO DEPLOY`. It now also checks the
maker's accounts, endpoints, price, premium check and float.

6.3. Back up `$D` offline now (it holds every maker key). Then send **0.01 ZEC** to the
maker's address from 6.2.

6.4. Create the second Railway service (do not touch the testnet service `zecret-maker`):

- In the Railway project `zecret`: New service → GitHub repo `officialcmg/zecret`.
  Name it `zecret-maker-mainnet`.
- Settings → Source: branch **`mainnet-release`** (create it from `main` at the merged
  commit; then a push to `main` does not restart the mainnet maker). Builder: Dockerfile,
  path `deploy/maker/railway/Dockerfile`. Watch paths `/crates/**` and
  `/deploy/maker/**`. Restart policy: always. Replicas: 1. No public domain (there is no
  relay on mainnet).
- Volume: new volume `zecret-maker-mainnet-data`, 1 GB, mounted at `/data`.
- Variables (each value from your machine; none is printed by the start script):

| Variable | Value |
|---|---|
| `ZECRET_NETWORK` | `main` |
| `ZECRET_ALLOW_MAINNET` | `1` |
| `ZECRET_MAKER_CONFIG` | output of `base64 -w0 < $D/config.json` |
| `ZECRET_MAKER_STATE` | output of `tar czf - -C $D wallet.json rfq_key.json \| base64 -w0` |
| `ZECRET_NEAR_KEY` | contents of `$D/near_key.json` |
| `ZECRET_NEW_WALLET` | `0` |
| `ZECRET_NEW_NEAR_KEY` | `0` |
| `RUST_BACKTRACE` | `1` |

Deploy. The log must show, in this order: `MAINNET mode: real funds`, `imported
wallet.json`, `imported rfq_key.json`, `wrote near_key.json from ZECRET_NEAR_KEY`, `wrote
config.json from ZECRET_MAKER_CONFIG`, `settings: {"network":"main",...}`, `open swaps:
0`, `wallet address: u1...` (the same as 6.2), `NEAR public key: ed25519:...` (the key in
`$D/near_key.json`), `directory entry: {...}`, `starting: zecret maker --dir /data
(mainnet, no relay)`, then `[maker ...] up: UA u1... NEAR maker.<owner> ...`. It must not
show `--relay`. If it stops with `error:`, the message names the setting.

6.5. Prove a mainnet send before the first swap (gap G2). On your machine, with the
service running, do not use `$D` (two processes must not use one wallet). Use a second
throwaway folder:

```sh
P=~/zecret-payer-mainnet && mkdir -p $P && jq '.chains.birthday = '$TIP $D/config.json > $P/config.json
$Z init --dir $P                         # a payer wallet: send it 0.003 ZEC from your wallet app
$Z balance --dir $P                      # wait until "zats" shows it
$Z pay --dir $P --to <your own u1 address> --zats 100000
```

`pay` prints the transaction id. A refusal prints the node's reason (send it to the
developer). This also tests that your wallet app can pay a single-Orchard-receiver
address (gap G5).

### Step 7. The website (10 to 20 min, most of it the Vercel build)

7.1. Make the directory file from the entry the service logged (or `$D/entry.json`), and
commit it to `main`:

```sh
node deploy/web/directory-from-entry.mjs $D/entry.json 890 escrow.$OWNER > web/public/config/mainnet-directory.json
git add web/public/config/mainnet-directory.json && git commit -m "Publish the mainnet maker's directory entry" && git push
```

The rate (`890`) must be `price_per_zec / 10^6`.

7.2. Create the second Vercel project (do not change the project `zecret`):

- Import `officialcmg/zecret` again as project `zecret-mainnet`. Root directory `web`,
  include files outside the root directory, framework Vite. `web/vercel.json` sets the
  build commands.
- Environment variables (Production):

| Variable | Value |
|---|---|
| `VITE_ZECRET_CONFIG` | `mainnet` |
| `VITE_ZECRET_ALLOW_MAINNET` | `1` |
| `VITE_MAINNET_ESCROW` | `escrow.<owner>` |
| `VITE_MAINNET_ESCROW_CODE_HASH` | `ESCROW_HASH` |

- Domains: add `mainnet.zecret.chrismg.dev` (a CNAME at your DNS host, as Vercel shows).

The build stops at once if a variable or `mainnet-directory.json` is missing
(`deploy/web/vercel-build.sh`).

7.3. Check: open `https://mainnet.zecret.chrismg.dev`. The landing page badge says
"Mainnet · real money · unaudited". `/swap` shows the notice "Real money. Unaudited
hackathon software." with "The maker takes at most 0.02 ZEC per swap.", and no testnet
banner. Then check that `https://zecret.chrismg.dev/swap?config=mainnet` still says "This
site does not run on mainnet".

### Step 8. The first swap (about 10 minutes)

See section 6.

### Step 9. After the first swap: check on chain

```sh
SWAP=<swap id from the maker log "escrow OPEN id ...">
view escrow.$OWNER get_swap '{"id":"'$SWAP'"}'                # state Claimed, a published
view 17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1 ft_balance_of '{"account_id":"maker.'$OWNER'"}'   # 60 USDC less the price
view intents.near mt_balance_of '{"account_id":"escrow.'$OWNER'","token_id":"nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"}'  # "0": nothing stranded
```

And the USDC on Base at your address, and the maker log line `<swap> stage Swept`.

### Step 10. Open the site to others (only after the go / no-go list)

Share the hostname. Until then, nobody knows it.

### Step 11. Every few hours

```sh
$T plan --config deploy/configs/mainnet.toml --expect-escrow-hash $ESCROW_HASH --maker-config $D/config.json \
  --escrow-wasm contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm
$T verify --config deploy/configs/mainnet.toml --escrow-wasm contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm
```

`plan` fails on a stale price, a gas price that breaks the stipend, or a float smaller
than 3 maximum swaps. `verify` shows the maker's NEAR, USDC and storage funds.

## 6. The first swap

- **Amount:** 0.002 ZEC, the maker's minimum (about 1.8 USDC). Small enough to lose. The
  page asks for the amount plus the premium (0.00025 ZEC) and the temporary wallet's fees.
- **Who pays:** you, as the taker, from your own wallet app (gap G5). The exit goes to
  your own USDC on Base address.
- **Before you pay:** save the recovery file (the page asks). Keep the tab open.

What to watch, in order:

The maker log prints one line per stage change, `<swap> stage <Name>`, with the names
`Quoting`, `Quoted`, `Opening`, `Opened`, `Ready`, `Sweeping`, `Swept` (or `RefundSent`,
`Refunded`, `Aborted`), and log lines with each transaction id.

| Page | Maker stage | On chain |
|---|---|---|
| Payment seen, confirmed | | Zcash explorer: your payment |
| Request sent | `Quoting`, then `Quoted` | Zcash: the maker's quote (its first mainnet send if step 6.5 was skipped) |
| Escrow opened | `Opening`, then `Opened`; the line `escrow OPEN id ... amount ...` | NEAR: `ft_transfer_call` on USDC to the escrow, and 0.03 NEAR to a new 64-hex account |
| Waiting for confirmations | `Ready` after 3 Zcash blocks on your payment to the shared address | NEAR: `set_ready` |
| Claiming | | NEAR: `commit_claim` and `claim` signed by the 64-hex account |
| Payout | the maker sends `withdraw` (200 Tgas) | `intents.near` holds the USDC under the 64-hex account |
| Delivery | | 1Click status `SUCCESS`; USDC on Base |
| Done | `Sweeping`, then `Swept` | Zcash: the maker's sweep of the shared address |

How to stop:

- **Before you pay:** close the tab. Nothing moved.
- **After you paid, before the claim:** do nothing. If the maker does not finish, its
  refund publishes its key half and the page takes your ZEC back (less fees and the
  premium if it was paid). If the maker is gone, after the deadline the page claims the
  USDC anyway.
- **After the claim, if the exit fails:** the USDC stays in `intents.near` under the
  swap's key. The page offers to try again. The recovery file holds the key.
- **To stop the maker** for everyone: section 7.

## 7. Pause, stop and roll back

**Pause new swaps, keep open swaps running (preferred).** In
`web/public/config/mainnet-directory.json` set `"status": "paused"`, commit and push to
`main`. After the Vercel build the form says the maker is paused and no new swap starts.
The maker keeps working on open swaps.

**Stop the maker.** In Railway, remove the active deployment of
`zecret-maker-mainnet` (or set replicas to 0). First check the log for open swaps, and
pause the directory first. A stopped maker sends no quote. For swaps already open, the
taker's deadlines still apply: before Ready the taker waits for the maker's refund; if the
maker never comes back, after the punish deadline the taker claims the USDC. Start the
maker again before `t0 + t1 + t2` (40 minutes with the template windows) to refund or to
take the ZEC.

**Change limits or price.** Set the new `ZECRET_MAKER_CONFIG` and redeploy. The start
script applies it only when no swap is open; otherwise it logs `WARNING: the wanted
settings differ ... kept config.json` and applies it at the next start.

**Take the money back.**

- USDC float: with the maker's NEAR key (`$D/near_key.json`), `ft_transfer` from
  `maker.<owner>` to your account (near-cli; the deploy tool has no command for it). Stop
  the maker first.
- Maker's storage funds: `withdraw_storage_funds` on the escrow, signed by the maker.
- Maker's ZEC: stop the service, then `zecret pay --dir $D --to <your address> --zats N`
  on your machine from the backup. Never run two processes on one wallet.

**The escrow cannot be rolled back.** It has no keys; its code cannot change. Its 5.46
NEAR stay locked. A new escrow build means a new `escrow.sub_account` and a new
deployment (`docs/DEPLOY.md`), a new `VITE_MAINNET_ESCROW` and code hash, and a new
maker config.

**The website.** Remove the domain from the Vercel project `zecret-mainnet`, or pause
the project. The testnet site is not affected.

## 8. Monitoring

- Railway logs of `zecret-maker-mainnet`: lines with `transient:` that repeat, a swap
  that stays in one stage longer than its window, `ABORT`, `refused by the node`.
- Step 11 every few hours (price, gas, float).
- The maker's NEAR: keep it above 0.3 NEAR. Its storage funds are topped up by the maker
  from its NEAR.
- The light-wallet server: `scripts/mainnet-preflight.sh --quick` checks it and the NEAR
  RPC and 1Click in about 10 seconds.
- A swap of your own every day (0.002 ZEC) shows that the whole path still works.

## 9. Risks and what limits each loss

| Risk | What limits the loss |
|---|---|
| Wrong or stale price (G7) | `max_zats` per swap (0.02 ZEC); the float (60 USDC); the price rule and step 11; pausing. |
| A bug in the maker or the escrow | The float; `max_pending_quotes` 3; the escrow's refund windows: before Ready the maker refunds; after `t2` the punish path pays the taker. Each exit publishes one key half, so neither side can take both. |
| The taker's funds in a failed swap | The refund path returns the ZEC less fees and the premium; the deadlines are enforced by the escrow, not by the maker. |
| Single trusted NEAR RPC (G11) | Small swaps; your own swaps first. |
| The 1Click leg (G1) | The USDC stays in `intents.near` under the swap's key until a signed exit executes; nothing is signed below the user's minimum; the recovery file holds the key. |
| Gas price above 1.12 × today (G6) | The taker cannot claim; the maker refunds at its deadline; the taker gets the ZEC back. Pause when step 11 fails. |
| Key custody on Railway | Only the float, the maker's ZEC (fees and finished swaps) and its NEAR are on the service. Your owner key never goes there. Move ZEC and USDC out regularly. |
| Light-wallet server down (G4) | Swaps stop progressing; deadlines return funds. No loss beyond fees. |
| Maker's volume lost | Backup of `$D` (keys). Swap records of open swaps are lost: those swaps end by the taker's deadlines; the maker may lose the USDC of a swap the taker then claims after `t2` (at most one maximum swap per open swap). |

## 10. Checks run for this runbook (2026-11-14)

- `zecret-deploy plan` on mainnet, read-only (output in section 11).
- `scripts/mainnet-preflight.sh` (full run, 03:34 UTC) with the stand-in owner, today's
  escrow wasm and hash, and the example maker config: 24 passed, 0 failed (with
  `SKIP_ENGINE_BUILD=1`: the engine had been built from this branch just before).
- Zcash mainnet block intervals (`scripts/zcash-block-intervals.py`, 3000 intervals,
  blocks 3,567,916 to 3,570,916): one block mean 24.9 s, median 17 s, 99.9th percentile
  153 s, maximum 182 s; three blocks 99.9th percentile 250 s, maximum 299 s. So
  `fund_secs` 600 is about twice the worst three blocks plus D. Tests:
  `crates/zecret-node/src/mainnet_config.rs`.
- NU7 activation on mainnet at 3,543,405 (ZIP 218; branch ids of the coinbase
  transactions of blocks 3,543,403 to 3,543,406). Test:
  `wallet::tests::mainnet_parameters_have_nu7_at_the_height_the_chain_shows`.
- Native wallet on mainnet: `zecret balance` scanned to the tip; `zecret directory` signed
  a mainnet entry (`u1...`); the site's directory parser read the converted file.
- Browser: `scripts/mainnet-browser-check.mjs` from a page in Chromium: gRPC-web
  `GetLightdInfo` (chain main, branch `77190ad9`, CORS echoed the page origin), the
  engine's `zcashTip` in a Web Worker (3,570,968), NEAR RPC final block, 1Click tokens.
  The engine from `main` fails the same check with "lightwalletd is supported on testnet
  only"; this branch fixes it.
- Website: unit tests (120), Playwright on the default build (the mainnet refusal, the
  landing page, the tabs; 18 tests) and on the switched build (6 tests).
- Start script: 18 cases with a stand-in binary.

Not checked: anything in the gap list; the Vercel and Railway setups (not created);
Firefox and Safari; the explorer links from a browser.

## 11. Dry-run output (2026-11-14 03:23 UTC)

Stand-in owner `aurora` (an existing account; read-only), today's escrow build, and an
example maker config (`maker-mainnet.json` = the template with owner `aurora`, birthday
3,570,900 and price 890 USDC per ZEC):

```text
$ zecret-deploy plan --config configs/mainnet.toml --owner aurora --escrow-wasm ../contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm --expect-escrow-hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1 --maker-config maker-mainnet.json
escrow wasm ../contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm: 525881 bytes, code hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1
== dry run on mainnet: nothing is signed or sent
network
  [PASS] RPC https://rpc.mainnet.fastnear.com serves chain_id "mainnet" (want mainnet), nearcore "2.14.1"
  [info] final block 224457020 (9n3E23jmCP9hxoqBvEiA9hvzFmWFcxbcyan6MqpcgB1h), gas price 100000000 yoctoNEAR per gas
  [PASS] storage price 10000000000000000000 yoctoNEAR per byte (1.00000 NEAR per 100 kB) equals the tool's
  [info] protocol version 88, min_gas_price in the protocol config "1000000000"
escrow code
  [info] escrow wasm ../contracts/near-escrow/target/wasm32-unknown-unknown/release/near_escrow.wasm: 525881 bytes, code hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1
  [PASS] escrow code hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1 equals the reviewed hash DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1
owner aurora
  [PASS] owner aurora exists: balance 145768.49718 NEAR, storage 14384189674 bytes, free 1926.60044 NEAR
  [OWNER] put the owner's full-access key in configs/../.keys/mainnet/aurora.json (mode 600) before deploy
accounts
  [info] CREATE escrow.aurora: one temporary key, then one transaction deploys DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1, calls new(intents_account = intents.near) and deletes the key
  [info] CREATE maker.aurora with 1 NEAR, then add_storage_funds 0.1 NEAR from it
  [PASS] maker initial balance 1.00000 NEAR covers its storage funds 0.10000 NEAR plus 0.1 NEAR for gas
token 17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1
  [info] token code hash DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q
  [PASS] token symbol "USDC" decimals 6 (config 6)
  [info] token storage_balance_bounds.min 0.00125 NEAR
  [info] REGISTER escrow.aurora on the token (storage_deposit, paid by the owner)
  [PASS] intents.near is registered on the token
  [info] REGISTER maker.aurora on the token (storage_deposit, paid by the owner)
  [PASS] the token is Circle's USDC on NEAR (the 1Click USDC asset)
Verifier intents.near
  [info] Verifier code hash BtA1BEFNS619KkvXFsgcpQ21Tb5yMm32aaUkn7xNothk
  [PASS] Verifier current_salt = "252812b3"
cost to the owner
    5.25881 NEAR  escrow account: 525881 bytes of code storage [locked]
    0.20000 NEAR  escrow margin (escrow.margin_near) [locked]
    1.00000 NEAR  maker account (maker.initial_balance_near; pays its gas, stipends and storage funds)
    0.00250 NEAR  token registrations: 2 × 0.00125 NEAR [locked]
    0.00644 NEAR  gas of the deployment transactions (about 64.4 Tgas at 100000000 yoctoNEAR/gas)
    6.46775 NEAR  TOTAL, of which 5.46131 NEAR stays locked (escrow storage and margin, registrations)
  [info] deploy refuses to start unless the owner holds 6.95881 NEAR free (its own bound: the transfers plus 0.5 NEAR for gas and registrations)
  [PASS] owner free balance 1926.60044 NEAR ≥ 6.95881 NEAR
taker stipend
  [PASS] 0.03 NEAR covers commit and claim: claim's 25 Tgas at 10× the gas price plus about 0.0019 NEAR burnt = 0.02690 NEAR (stays enough up to a gas price of 112400000, 1.12× today)
1Click (dry quotes only)
  [PASS] GET /v0/tokens: HTTP 200, 231 assets
  [PASS] 1Click lists USDC on NEAR with 6 decimals
  [info] 1Click token prices: ZEC Some(924.59) USD, wNEAR Some(3.38) USD
  [PASS] exit route 5 USDC (NEAR) → USDC on Base: amountOut 4997080 minAmountOut 4947109 in about Some(12.0) s; 1Click signature valid
  [PASS] 0.005 ZEC → USDC on Base: amountOut 4594051 (918.8102 USDC per ZEC); 1Click signature valid
maker config maker-mainnet.json
  [PASS] maker chains.network = "main" (want main)
  [PASS] maker near_account = "maker.aurora" (want maker.aurora)
  [PASS] maker chains.escrow = "escrow.aurora" (want escrow.aurora)
  [PASS] maker chains.token = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
  [PASS] maker chains.intents = "intents.near"
  [PASS] maker endpoints are https
  [PASS] maker config has no placeholder left
  [PASS] maker config sets no misbehave
  [PASS] maker birthday 3570900
  [PASS] maker stipend 0.03000 NEAR (0.03 NEAR measured)
  [PASS] maker limits 200000..2000000 zats, premium 25000 zats
  [PASS] maker price 890000000 USDC units per ZEC is 200..1500 bp below the market 918810200
  [PASS] premium_check: 25000 zats at 200.00000 NEAR per ZEC is 0.05000 NEAR ≥ stipend + costs 0.04500 NEAR
  [PASS] premium_check.yocto_near_per_zec 200.00000 NEAR is not above the market 273.54733 NEAR (it must be a low estimate)
  [info] the premium is worth 0.06838 NEAR at the market price
== plan for aurora on mainnet
  1. create escrow.aurora and deploy DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1 in one transaction that deletes its key
  2. register escrow.aurora and intents.near on 17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1 if needed
  3. create maker.aurora, register it on the token, add 0.1 storage funds
  4. verify (read-only); no USDC moves: the owner sends USDC to maker.aurora
  total from the owner: about 6.46775 NEAR
== 0 failed check(s), 1 owner input(s) missing
  owner: put the owner's full-access key in configs/../.keys/mainnet/aurora.json (mode 600) before deploy
DRY RUN PASSED: NO CHECK FAILED; OWNER INPUTS MISSING (nothing was sent)
```

The maker account `maker.aurora` does not exist, so the float check did not run in this
output; with a deployed maker the dry run also compares its USDC balance with 3 maximum
swaps.

## 12. Go / no-go

Go only if every line is true:

- [ ] This branch is merged, and `mainnet-release` points at the merged commit.
- [ ] The escrow and app flow you ship passed its own testnet soak (your rule of
      2026-11-14 in `STATUS.md`: no mainnet step on a flow that has not passed its soak).
- [ ] `scripts/mainnet-preflight.sh` with your `OWNER` and `ESCROW_HASH`: `PREFLIGHT PASSED`.
- [ ] The escrow hash is the one you decided to ship (today's escrow or the light-payout
      escrow), and the same hash is in `VITE_MAINNET_ESCROW_CODE_HASH`.
- [ ] Step 2 and step 3: the dry run passes, the stipend line passes at today's gas price.
- [ ] `verify` after deploy: `ALL CHECKS PASSED`; the escrow has no access keys.
- [ ] The maker holds 60 USDC, about 1.9 NEAR, 0.01 ZEC; `$D` is backed up offline.
- [ ] The maker's log shows mainnet mode, no relay, the expected address and key.
- [ ] Step 6.5: a mainnet send from `zecret pay` worked, and your wallet app paid a
      `u1` single-receiver address.
- [ ] The price is market less 3 % and the directory rate is the same number.
- [ ] The mainnet site shows the real-money notice; the testnet site refuses `?config=mainnet`.
- [ ] You decided the legal questions in G12.
- [ ] You are awake for the next hour to watch the first swap.

No-go if any check fails, if the gas price is above 1.12 × 10^8 yoctoNEAR per gas, or if
1Click's dry quote fails.
