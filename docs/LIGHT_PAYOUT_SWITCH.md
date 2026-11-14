# Switch the live testnet services to the light-payout escrow

Runbook for the main session. Branch `escrow-light-payout`. Its last commit IS the switch:
merging it into `main` moves the hosted maker (Railway) and the website (Vercel) from
`escrow2` to `escrow3`. Read [`docs/security/review-5.md`](security/review-5.md) and
[`docs/experiments/light-payout.md`](experiments/light-payout.md) for why.

## What changes

| | Before (live) | After the switch |
|---|---|---|
| Escrow | `escrow2.zecret-3f109eea1e.testnet`, code `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` | `escrow3.zecret-3f109eea1e.testnet`, code `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536` (no access keys, Verifier `intents.zecret-3f109eea1e.testnet`) |
| Maker's tUSDC | plain balance on `usdc.zecret-3f109eea1e.testnet` | still plain at first; before each open the maker deposits what the swap needs into its own Verifier account, then opens with `mt_transfer_call` |
| Payout | the maker sends `withdraw` (200 Tgas) | the taker's key sends `withdraw(id)` (25 Tgas); the maker sends it only as a backup |
| Stipend | 0.03 NEAR | 0.06 NEAR (the taker still accepts 0.03) |
| Maker account, key, wallet, volume | `maker.zecret-3f109eea1e.testnet`, key `ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH` | unchanged |
| Maker's directory entry | signed by that key | unchanged: the signature covers the Zcash address, epoch, request key, NEAR account and limits, not the escrow (`keys::directory_digest_v2`); only the unsigned field `escrowContract` changes |

`escrow2` stays deployed. Swaps opened on it can still finish there, but the new
`zecret` binary cannot read them (it expects the field `payout`), so the maker must have
no open swap when it restarts (step 3).

## Files of the switch (in the branch, all in one merge)

| File | Change |
|---|---|
| `crates/zecret-node/config/testnet-maker.json` | `chains.escrow` = `escrow3…`, `stipend` = `60000000000000000000000`. The Railway image copies this file (`deploy/maker/railway/Dockerfile`). |
| `crates/zecret-node/config/testnet-taker.json` | `chains.escrow` = `escrow3…`, `escrow_code_hash` = `AqzB7mtM…` |
| `web/public/config/testnet.json` | `near.escrow` = `escrow3…`, `near.escrowCodeHash` = `AqzB7mtM…`; `minStipend` stays 0.03 NEAR |
| `web/public/config/testnet-directory.json` | `maker.escrowContract` = `escrow3…`, **`status` = `paused`**; signature unchanged |
| `deploy/maker/RELEASE` | one new line: this makes Railway rebuild the maker |
| `deploy/web/directory-from-entry.mjs` | default testnet escrow `escrow3…` |
| `deploy/deployment.testnet.json`, `deploy/configs/testnet.toml` | `escrow3` is current; `escrow2` under `retired_escrows` |
| code | the light-payout engine, maker and site (everything since `main` in this branch) |

## How the hosted maker takes the new settings

`deploy/maker/railway/start.sh`, section 3:

1. It counts open swaps: files `/data/swap/*.json` whose `stage` is a JSON string other
   than `Swept` or `Refunded`. `Aborted` is stored as an object and does not count.
   `Quoting` and `Quoted` count: a request that was quoted but not accepted stays open
   until the maker drops it after `quote_ttl_secs` (3600 s by default).
2. If the image's settings (`/etc/zecret/config.testnet.json`, the template above with
   `chains.birthday` from the Dockerfile) differ from `/data/config.json` and no swap is
   open, it copies the old file to `config.json.prev-<time>` and writes the new one. Log:
   `config.json updated (no open swap); the old one is config.json.prev-…`.
3. With an open swap it keeps the old file. Log: `WARNING: the wanted settings differ
   from config.json, but N swap(s) are open: kept config.json …`. The new binary would
   then run on `escrow2`, which it cannot read. That is the failure this runbook avoids.
4. It never touches `wallet.json`, `near_key.json` (it rewrites it only if
   `ZECRET_NEAR_KEY` changed), `rfq_key.json`, `seen.json`, `swap/` or `relay/`. So the
   Zcash address, the NEAR key and the signed directory entry stay the same.

## Preconditions (done on 2026-11-14)

- `escrow3` deployed and verified (tx `FMSRRYU8qsLwHaeWeMnjjyxPWCPcRaytqXQPxbYmgAmA`:
  deploy, `new` and key deletion in one transaction; `docs/TESTNET.md`, "Light payout
  on testnet").
- Nothing has to be done on chain for `maker.zecret-3f109eea1e.testnet` before the
  switch, and nothing was done:
  - it is registered on the token (storage 0.00125 NEAR) and holds 19,989.8599 tUSDC
    plain, 0 in the Verifier, 10.293 NEAR (read at 05:08 UTC);
  - the Verifier needs no registration: `ft_transfer_call` to it with an empty `msg`
    credits the sender's account there, also an account that never used it;
  - storage funds in `escrow3` can be added only by the maker itself
    (`add_storage_funds` refuses another `account_id`); the maker adds 0.1 NEAR before
    its first open (`storage_min` 0.01 NEAR);
  - `intents.zecret-3f109eea1e.testnet` is registered on the token; `escrow3` needs no
    token registration.
- That exact path (an account with plain tUSDC only, no Verifier balance, no storage
  funds in `escrow3`) was run on testnet by the rehearsal below: storage funds, deposit,
  open, in that order, all executed.
- Finding: the owner's key folder `deploy/.keys/testnet/maker.zecret-3f109eea1e.testnet.json`
  holds a second full-access key of the hosted maker account
  (`ed25519:8xU2efSdeDJRF6tLDFec17oW4vzbMm8Dj5ybUvGjYtov`, from the first deployment).
  It was not used. `zecret-deploy deploy --config deploy/configs/testnet.toml` would
  sign as the maker with it; do not run `deploy` with that file.

## Rehearsal (2026-11-14, 04:55 to 05:20 UTC)

The exact configuration that goes live, apart from the maker's identity and the relay
URL, on testnet, without touching the hosted services:

- maker: `maker4.zecret-3f109eea1e.testnet`, created for this with
  `zecret-deploy deploy --config deploy/configs/testnet-rehearsal.toml` (create
  `6ur3T4iRwvet23EV4PaNqLJpi4siGt9H8X1xeGxMu3zH`, token registration
  `d6Y7ukMBy4NSnj5jjdjbHhD8nz7bpW8yzywQeXHbBq4`, mint 200 tUSDC
  `2BwCnT1GT1aJiRY2JkP5Khc5fYEgws22KyE5sEpEhjPq`); 1 NEAR, 200 tUSDC plain, nothing in the
  Verifier, no storage funds in `escrow3`. Its settings: `testnet-maker.json` of this
  branch with only `near_account` and `chains.birthday` changed. Native `zecret maker`
  with its relay on `127.0.0.1:3412`.
- Zcash: the Fauzec faucet refused new claims from this machine (`ip_on_cooldown` until
  2026-11-15 03:41 UTC). The new maker wallet and the new taker wallet were paid from
  this worktree's faucet-funded rehearsal wallets of 04:00 UTC (`.tools/testnet-lp/`;
  Zcash txs `e098509a…bcc3`, 0.5 TAZ, and `3573eda2…7845`, 0.03 TAZ). The main tree's
  wallets were not used.
- Site: a production build of this branch with `web/public/config/testnet.json` changed
  only in `directoryUrl` and `exit.relayUrl` (`testnet/light-switch/testnet-switch.json`),
  and a directory made by `deploy/web/directory-from-entry.mjs` from maker4's signed entry
  with the new default escrow (`testnet/light-switch/testnet-switch-directory.json`),
  served by `testnet/light/serve.mjs` with the headers of `web/vercel.json`, driven by
  `testnet/hosted/run.mjs` unchanged.

Native swap `ca29a33a`, 0.01 TAZ for 0.3 tUSDC (swap id `d239fbc8…c492`; read with RPC
`tx`, final, no failed receipt):

| Step | NEAR transaction | Signer |
|---|---|---|
| `add_storage_funds` 0.1 NEAR (first open on `escrow3`) | `D3Gd4dngSviPRWnc8jieu9Xsx5eP9RK77Y1QQuj1QJYC` | maker4 |
| deposit 0.3 tUSDC into maker4's Verifier account (`ft_transfer_call`) | `BriYuQZv1NCdJ3nR9iYqk56kqc5X54cUoQUoLUWWUPgF` | maker4 |
| open (`mt_transfer_call` on the Verifier) | `3c49eCirtPH9N432ATh1STFjcUBwYVmy1hVRomVwEnDw` | maker4 |
| stipend 0.06 NEAR | `XNQQk7wqscu5hMhYxTNmaAnUs5xyvVa3iZbwUxuf55L` | maker4 |
| `set_ready` | `6zpf4L5z4NHfXYQzoHDEdGPLTs6jyNqhVM4sAP7tGxjQ` | maker4 |
| `commit_claim` | `GJmUrwdCSkLdXuUdPBnU2MzuipNsoBvb9qS9Y5HYBRTq` | taker key `781af672…e76a` |
| `claim` | `FrXhYB4FqGn9QCVFpyyhC68oCzvWZYWGWt4GuiEe28aq` | taker key |
| `withdraw(id)` | `2X61FHSecZkjnjwc4oTUodUrezRwJFEV6HL8dyqWtsZd` | taker key |

`get_swap` shows `Claimed`, payout `paid` (README, "Try it", step 4, prints it).

Browser swap `eba68c40`, 0.01 TAZ through the page in headless Chromium (swap id
`31efa309…ca3b`; payment `4ac156b2…4e63` from the rehearsal payer with `zecret pay`):
from the payment to `done` in 403 s; the page showed "You received 0.2877 test USDC on
NEAR"; no Content-Security-Policy violation. Result:
`testnet/light-switch/browser-result.json`, screenshots `testnet/light-switch/browser-*.jpg`.

| Step | NEAR transaction | Signer |
|---|---|---|
| deposit 0.2877 tUSDC into maker4's Verifier account | `5p1TeSA3Ky5DWRejdj1Crw1oXhEH7CeC1ABEuaMxYnVY` | maker4 |
| open (`mt_transfer_call`) | `9x2GJqgVZ6k1mXZnKawP1Wzjv7mGfhRpxziLnMUp3o1x` | maker4 |
| stipend 0.06 NEAR | `CKQiugXocLFt9Y7FSMXn8CkhKSuZMYK48B365ZSzQEcL` | maker4 |
| `set_ready` | `CFcpYWXwfoyAAasQYHoMUjTBSF4453v5ZRmuYByp1xeC` | maker4 |
| `commit_claim` | `2jA1zobJpV7ynVwq6yeNStmhaPNfKL2psyh1Dyyyhvwa` | taker key `7efc266a…20ff` (made in the page) |
| `claim` | `99rKhgAxrUnGWKPWUVZdMWZ3jGGFQmMbQbETHGCJE6VP` | taker key |
| `withdraw(id)` | `9TS68HwpCesNdZokqJgr5XwTaWLeY2t1dHWkHDkENc7n` | taker key |
| direct exit to `zecret-3f109eea1e.testnet` (`execute_intents`, the taker's signed transfer, sent by the relay) | `CELhj23GT8iyihWbRWDPPFkapucki1SWo1Yd6KNgHDQx` | maker4 |

All transactions of both swaps were read with RPC `tx` (`wait_until: FINAL`): each
succeeded with no failed receipt.

## The switch: three pushes, the switch itself is one

The escrow is not in the maker's signature, so the hosted maker's entry stays valid and
the site and the maker can change in one push. Two more one-line pushes of the directory
file wrap it, because Railway and Vercel finish at different times and the maker must be
drained before it restarts.

Expected time: 30 to 90 minutes. The site refuses new swaps from the end of push 1 to the
end of push 3. Swaps in flight are not interrupted.

### Push 1: pause (on `main`, before the merge)

1. Stop the soak script between two slots, so that it does not record failures against
   the paused site:
   ```sh
   pgrep -fa "testnet/hosted/soak.sh|hosted/run.mjs"   # wait until no run.mjs is listed
   pkill -f "testnet/hosted/soak.sh"                     # stops the loop; slots already done stay in results.jsonl
   ```
   Also pause any scheduled health-swap task for the window.
2. In `web/public/config/testnet-directory.json` on `main`, set `"status": "paused"`.
   Commit (message must not start with `WIP`) and push. Only Vercel rebuilds
   (`deploy/web/vercel-ignore.sh`; no file under `deploy/maker/` changes).
3. Check: `curl -s https://zecret.chrismg.dev/config/testnet-directory.json | grep status`
   prints `"status": "paused"`. The form then says the maker is paused.

### Drain

Wait until the hosted maker has no open swap. In the Railway log of `zecret-maker`, follow
each swap to `stage Swept`, `stage Refunded` or `stage Aborted`. A swap in flight ends
within `t0 + t1 + t2` = 40 minutes, usually within 10. A request in `Quoted` whose taker
went away is dropped after 1 hour (`quote_ttl_secs`); it counts as open for `start.sh`.
If you cannot read the volume, the safe sign is: no new log line with a swap id for 1
hour, or every swap id seen in the log since the last start reached one of the three
final stages.

### Push 2: the switch

```sh
cd /home/user/zecret && git fetch origin escrow-light-payout
git merge --no-ff origin/escrow-light-payout   # conflicts: only testnet-directory.json if push 1 is not in the branch: keep "paused" and escrow3
git push origin main
```

Railway rebuilds the maker (`deploy/maker/RELEASE` changed); Vercel rebuilds the site and
the engine.

Check the maker's start log (Railway, `zecret-maker`):

- `config.json updated (no open swap); the old one is config.json.prev-…`
- `settings: {"network":"test","escrow":"escrow3.zecret-3f109eea1e.testnet",…}`
- `open swaps: 0`
- `NEAR public key: ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH`
- `directory entry: {…"sig":"7c756258d242…"…}`: the same signature as in
  `web/public/config/testnet-directory.json`
- `up: UA utest10kyef9lq7c637hlxx3fwdfrkukqnttzswcr4ukes5h3ewzca4e4nzycm73txxfgf44azfyngreys08dpzxqp8eah8dwacktj3s5yu0wv NEAR maker.zecret-3f109eea1e.testnet …`

If it says `WARNING: the wanted settings differ from config.json, but N swap(s) are
open`, the drain was not finished: roll back (below).

Check the site: `curl -s https://zecret.chrismg.dev/config/testnet.json` shows
`escrow3.zecret-3f109eea1e.testnet` and `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536`;
the directory still says `paused`.

### Push 3: online

Once both deployments are live: `"status": "online"` in
`web/public/config/testnet-directory.json`, commit, push. Then one health swap:

```sh
source .tools/env.sh && export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers ZECRET_BIN=$PWD/.tools/soak/zecret
node testnet/hosted/run.mjs https://zecret.chrismg.dev .tools/testnet/taker zecret-3f109eea1e.testnet 0.01
```

`run.mjs` and `soak.sh` need no change (the rehearsal ran `run.mjs` unchanged against
the switched configuration; the direct exit to `zecret-3f109eea1e.testnet` worked). The
`zecret pay` binary the soak uses only pays Zcash, so the old `.tools/soak/zecret` works.
On the first swap the maker logs, before its open, `added 100000000000000000000000
yoctoNEAR of storage funds to the escrow` and `deposited … into the maker's Verifier
account before the open`.

Check that the payout was sent by the taker's key. Take the swap's `withdraw` hash from
the page console or `testnet/hosted/result.json`, or list the escrow's last
transactions on `https://testnet.nearblocks.io/address/escrow3.zecret-3f109eea1e.testnet`
(the `withdraw` row's "From" is a 64-hex account, not `maker.zecret-3f109eea1e.testnet`).
With RPC (`<taker>` is that 64-hex account):

```sh
curl -s https://rpc.testnet.fastnear.com -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tx","params":{"tx_hash":"<withdraw hash>","sender_account_id":"<taker>","wait_until":"FINAL"}}' \
  | python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]; t=r["transaction"]; print(t["signer_id"], t["receiver_id"], [a["FunctionCall"]["method_name"] for a in t["actions"] if "FunctionCall" in a], list(r["status"]))'
# <taker> escrow3.zecret-3f109eea1e.testnet ['withdraw'] ['SuccessValue']
```

Then `cd deploy && ./target/debug/zecret-deploy verify --config configs/testnet.toml`
ends with `ALL CHECKS PASSED` (before the maker's first open it fails only on `maker
escrow storage funds 0.00000 NEAR`).

Restart the soak for the new flow (the owner wants at least 12 swaps after the switch, in
a separate results folder): copy `testnet/hosted/soak/` aside first, then
`nohup testnet/hosted/soak.sh <next quarter hour, UTC> 12 &`.

## Roll back in one commit

`git revert -m 1 <merge commit of push 2>` and push. That restores the old code, the
maker template (`escrow2`, stipend 0.03) and the site config, and `deploy/maker/RELEASE`
loses its line, so Railway rebuilds the old maker. Before that push, pause the site
again (push 1) and drain `escrow3` swaps: the old binary cannot read `escrow3` swaps,
and `start.sh` keeps the volume's config while a swap is open. Funds are not stuck:

- tUSDC the maker deposited into its Verifier account stays there; the old maker opens
  from its plain balance and does not see it. To move it back, sign with the maker key:
  `ft_withdraw(token = usdc.zecret-3f109eea1e.testnet, receiver_id =
  maker.zecret-3f109eea1e.testnet, amount)` on `intents.zecret-3f109eea1e.testnet`,
  1 yoctoNEAR, 50 Tgas. The deficit does not matter on testnet: the maker holds 19,990
  tUSDC.
- A swap left Due on `escrow3` can be paid by anyone with `withdraw(id)` (25 Tgas).
- The maker's storage funds in `escrow3` stay there (`withdraw_storage_funds`, signed by
  the maker, returns them).

## Risks

- **A swap open at the restart.** Covered by push 1 and the drain. If it happens on
  `escrow2`: the swap needs a manual `withdraw(token, owner)` on `escrow2` after its claim
  (any account, 200 Tgas, about 0.2 NEAR held), and its refund duties need the old binary.
- **Old browser tabs** keep the old engine until reloaded. A new swap from one is refused
  before the premium: the quote names `escrow3`, the tab trusts `escrow2`. The user loses
  one request fee.
- **Recovery files saved before the switch** for swaps that already ended are not
  affected. A swap still on `escrow2` must end before push 2.
- **The maker's NEAR.** About 0.075 NEAR per swap now (stipend 0.06, account creation
  0.007, storage 0.005, gas); 10.29 NEAR lasts about 130 swaps.
- **NEAR Intents operators** can move or freeze the escrow's tokens for the whole swap
  (review 5, R5-02). On testnet the stand-in Verifier's operator is the owner.
- **Not verified:** the Railway and Vercel builds of this branch; a maker restart on
  Railway in the middle of an `escrow3` swap (tested locally: s4, s25); Firefox and Safari.
