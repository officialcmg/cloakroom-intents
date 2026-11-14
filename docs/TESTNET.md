# One swap on public test networks

On 2026-11-13 the native maker (`zecret maker`) and the native taker (`zecret taker`)
completed one swap on Zcash testnet and NEAR testnet. The taker paid 0.1 shielded
testnet ZEC (TAZ). The taker received 3 test USDC (tUSDC) inside the stand-in NEAR
Intents Verifier. Then a signed `ft_withdraw` moved the 3 tUSDC out of the Verifier to a
NEAR testnet account.

Later the same day, three more swaps ran through the hosted website, with the taker in
the browser: see [Hosted swap](#hosted-swap-2026-11-13-1133-to-1142-utc),
[Hosted swap after the price fix](#hosted-swap-after-the-price-fix-1207-to-1214-utc) and
[Hosted swap paid by the faucet](#hosted-swap-paid-by-the-faucet-1245-to-1251-utc). The
last one was paid by the Fauzec faucet with no wallet; a restore of it in a fresh browser
returned the surplus ([Restore that returned the surplus](#restore-that-returned-the-surplus)).
[A fresh testnet payer](#a-fresh-testnet-payer) lists the commands for a new
command-line wallet.

Public ids, hashes, timings and balances: [`testnet/EVIDENCE.json`](../testnet/EVIDENCE.json).
Logs, the configs that were used and the audit output: [`testnet/`](../testnet/).

## Result

| | |
|---|---|
| Taker result | `Completed`: 3,000,000 tUSDC base units (6 decimals) in `intents.zecret-3f109eea1e.testnet`, owned by the taker key |
| Maker result | `Swept`: the 10,010,000-zat joint note is in the maker wallet |
| Escrow swap `04a469d6…ace4` | `Claimed`; `a` is published; `b` is null (no refund) |
| Exit | `execute_intents` with one `ft_withdraw` signed by the taker key: the taker's Verifier balance went from 3,000,000 to 0, the destination's tUSDC from 3,000,000 to 6,000,000 |
| Zcash privacy | Each of the 6 swap transactions is v6 with 0 transparent inputs and outputs, 0 Sapling, 0 Orchard and exactly 2 Ironwood actions |
| Duration | 222 s from the taker's start to `Completed`; 312 s to the maker's `Swept`; 322 s to the end of the exit |

## What is real and what is a stand-in

| Part | On testnet |
|---|---|
| Zcash | Real public testnet (NU7 active since block 4,465,026). Real shielded transactions in the Ironwood pool, read and sent through ChainSafe's gRPC-web light-wallet server `https://zcash-testnet.chainsafe.dev`. TAZ has no value. |
| NEAR | Real public testnet, RPC `https://rpc.testnet.fastnear.com`. |
| Escrow | The reviewed build with the NV-02 fix, `escrow2.zecret-3f109eea1e.testnet`, code hash `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1`, no access keys (the taker checked this at a final block before it funded). |
| Token | Stand-in: `usdc.zecret-3f109eea1e.testnet`, a NEP-141 with open `mint`, 6 decimals, no value. |
| NEAR Intents | Stand-in: `intents.zecret-3f109eea1e.testnet` runs the code of mainnet `intents.near`. The two-step payout and the signed `ft_withdraw` ran for real on it. |
| Exit to another chain | Not run. 1Click and the solvers do not exist on testnet. The exit here is a direct `ft_withdraw` to a NEAR testnet account. |
| Relay of the exit | The maker account sent the `execute_intents` transaction that carries the taker's signed intent (as `deploy`'s e2e does). Any account can send it. On mainnet, 1Click would relay. |
| Cover delays | Off in the testnet configs. |

## Code that ran

- `zecret` built from the working tree at HEAD `cec2efc` with
  `cd crates && cargo build -p zecret-node --release`. The tree differed from HEAD only in
  a test file (`crates/zecret-core/tests/maker_bounds.rs`). Binary SHA-256
  `a755baab11a2d5f11698cf4869e1d386cd5e688ed853ed9fe7e70d4dab9a9153`. The client attaches
  200 Tgas to `withdraw` (`crates/zecret-core/src/near/escrow.rs:32`).
- The exit and the audit use a small program outside the repository crates
  (`.tools/testnet/exit/`, copy in [`testnet/tools/`](../testnet/tools/)). It uses
  `zecret-intents` (`IntentDraft::ft_withdraw`, `IntentSigner`), `zecret-core`
  (`NearRpc`, `Lwd`) and `zcash_primitives` (`Transaction::read`). It is the same as
  `node/src/bin/txaudit.rs`, but over gRPC-web with the core's own client.

## Steps and commands

The working folder is `.tools/testnet/` (not in git). The maker folder and the taker
folder hold private keys and stay there.

### 1. Testnet ZEC

The wallet reads only the Ironwood pool (`crates/zecret-core/src/wallet.rs:406-419`,
`:464-481`). A payment into the Orchard pool would not be seen.

Faucet: Fauzec (`https://fauzec.com`). It gives 1 TAZ per
address every 24 hours to unified or Sapling addresses. The web form uses a Cloudflare
Turnstile check. The documented API (`/.well-known/openapi.json`) takes claims without
it ("for now", says the site). One request was sent per wallet:

```sh
curl -X POST https://fauzec.com/api/v1/claim -H 'content-type: application/json' \
  -d '{"network":"testnet","address":"<UA>"}'
```

Both claims were accepted at 05:36:15 UTC and mined in block 4,614,924 (block time
05:36:30); Fauzec's status API reported them confirmed at 05:36:52.
Each faucet transaction spends 2 Orchard actions and creates 1 Ironwood action, so the
TAZ arrived in the Ironwood pool, and `zecret balance` showed 100,000,000 zats in each
wallet.

Other faucets were tried or looked up: `testnet.zecfaucet.com` and
`zcashfaucet.jinolabs.xyz` did not answer through this machine's proxy;
`faucet.zecpages.com` reset the connection; `faucet.testnet.z.cash` failed the TLS
handshake. Mining was not needed and was not tried.

### 2. Wallets and maker key

```sh
cd .tools/testnet
cp ../../crates/zecret-node/config/testnet-maker.json maker/config.json
cp ../../crates/zecret-node/config/testnet-taker.json taker/config.json
./bin/zecret check --dir maker          # Zcash tip 4,614,922
# chains.birthday = 4614922 in both configs
./bin/zecret init --dir maker
./bin/zecret init --dir taker
```

`maker/near_key.json` (mode 600) holds the `private_key` string of
`deploy/.keys/testnet/maker.zecret-3f109eea1e.testnet.json`. This is what
`zecret-deploy maker-key` writes (`deploy/tool/src/main.rs:281-301`). It was written with a
short script so that the deploy tool did not need a build (disk space). The key was not
printed.

| Wallet | Unified address (testnet) |
|---|---|
| Maker | `utest1g08wwdh0ye55dah3eagn8nyna8ws3wpweny50p3sjf74faa458jd35aerxt7cjnz2wncujawpwu04dy4gs2smejyrw6uhe3p7u9f3rka` |
| Taker | `utest1zdyw8dscgrelws7muvgznaa2xwld5aysr50w7xy46p4aqyrvlw7t2ejxllda8j6qp9dtc557f6cnnj7uypn8wkmqknzmcd225ca9ca04` |

### 3. Maker, directory entry, taker

```sh
./bin/zecret directory --dir maker > maker_directory.json   # put into taker/config.json "maker"
./bin/zecret check --dir taker     # escrow code hash, no access keys, pays into the Verifier
./bin/zecret maker --dir maker --exit-after 1 &
./bin/zecret taker --dir taker --zats 10000000
```

The configs used are in [`testnet/config/`](../testnet/config/). They are the repository
templates with `birthday` and `maker` filled in.

A first taker run asked for 50,000,000 zats. The maker refused the request: the
template's `max_zats` is 10,000,000 (log: `RFQ rejected: amount 50000000 outside
100000..=10000000`). Nothing was locked (stage `RfqSent`), so the taker was stopped, its
record was moved to `.tools/testnet/aborted_rfq/`, and the taker ran again with
10,000,000 zats. The rejected RFQ cost the taker one 10,000-zat fee (tx `918cc073…7cee`).

### 4. Exit

```sh
./bin/zecret-testnet-exit taker/swap/de41faf899eec6840bdcb2ce212bf71e.json \
    maker/near_key.json zecret-3f109eea1e.testnet
```

It reads the taker key from the swap record, reads `current_salt`, signs one
`ft_withdraw` of the full balance (raw ed25519, deadline 10 minutes), and the maker
account sends it in `execute_intents` with 300 Tgas.

### 5. Audit

```sh
./bin/zecret-testnet-audit 4614920 <txid> ...
```

## Transactions

### Zcash testnet

| Step | Txid | Height | Ironwood actions | Other pools | Fee (zats) |
|---|---|---|---|---|---|
| Faucet to taker | `75d81b9a34d10875969c67b596c11dbebc23368234d02040dd4711ef65600fc9` | 4614924 | 1 | 2 Orchard actions (spent into Ironwood) | |
| Faucet to maker | `a805b089fa495b0708db9f86d4f3a56dc5d82b5cff9a62e2abaea2dc1110208e` | 4614924 | 1 | 2 Orchard actions (spent into Ironwood) | |
| RFQ refused (50,000,000 zats) | `918cc073441430964f47e109253fe6c23dcae240329a33eac984b86fe9777cee` | 4614926 | 2 | none | 10,000 |
| RFQ | `1d0e35b69c9f75b503df44958ba13682567d538e18531a0947de0d05c209be08` | 4614927 | 2 | none | 10,000 |
| Quote | `c0e344e0443e26f6a4d7f149c4725450545a048425dc8458bcfbbc1e30cc2c48` | 4614930 | 2 | none | 10,000 |
| Accept + 1,000-zat premium | `be00a12b1622c2b66a5f620fd5fed9bcb23138275c09a83ac888dc35d1f0dade` | 4614932 | 2 | none | 10,000 |
| Fund joint address (10,010,000) | `48cf8e0fb0144352df7fc9257f44cd15cfee3e03b5c9ca4f3dadcc74c46fef5a` | 4614934 | 2 | none | 10,000 |
| Sweep to maker (a+b) | `2c9987356abb408ae6553df8b014e9de2ddb52bfa20c74f40f2573b21785c368` | 4614937 | 2 | none | 10,000 |

The audit fetched each transaction from the light-wallet server, parsed it, and computed
its txid again; each txid matched. Each swap transaction is 9,166 bytes. Every swap
transaction has the same shape (2 Ironwood actions, value balance 10,000), so an
observer cannot tell an RFQ from a fund or a sweep by its form.

### NEAR testnet

All are final and succeeded with no failed receipts.

| Step | Hash | Signer | Block | Gas burnt (Tgas) |
|---|---|---|---|---|
| `open` (`ft_transfer_call` of 3,000,000 to the escrow) | `DnGg8B2paHKMUcXAhzC2hfYQkTitqv3qVzKT5euZVe8x` | maker | 277468656 | 12.21 |
| Stipend 0.03 NEAR to the taker key | `CTza5f9XvaCXYuXSoGsjSgiA3k3RGRxjNehfp76TXHbf` | maker | 277468662 | 8.57 |
| `set_ready` | `9kn4hxVUxMqd16AzvkwaWQUouYhDrVeoJdyPp4cUXqix` | maker | 277468721 | 2.60 |
| `commit_claim` | `8fdBEHbFBN4p64YFuNyHCX8v2TApKSyQaZjiAxaUVrds` | taker key | 277468729 | 10.21 |
| `claim` (publishes `a`) | `DGShdiHZfLWRyUyWX69dDCt14JV86FFHJq1Twowmk3Dz` | taker key | 277468738 | 24.37 |
| `withdraw` (two-step payout into the Verifier) | `DodbZPEToNesVSF5G9yQUogm1zN9FqF3hdTdwEGPepL6` | maker | 277468747 | 21.64 |
| Exit: `execute_intents` (`ft_withdraw` signed by the taker key) | `D2eRTKBNsJtbasRARwyjGPfjqQ9zYAQQG9LKCiJFic3z` | maker (relay) | 277468922 | 11.35 |

Taker key account: `15968c4ae680388e6c4b4511de8cfaab2a71a8eb0023f2f787ccdc085a2a9067`.
The taker also tried to send `withdraw` itself (`3wt89rSW…ncxrc`). The RPC refused it
before inclusion (see Problems, item 2). It is not on chain.

## Timings (UTC, 2026-11-13)

Times from the program logs (timestamped when each line was written) and from block
times. Zcash block times are set by the miner and are accurate only to some seconds.

| Stage | Time | From previous |
|---|---|---|
| Taker starts the swap | 05:38:42.9 | |
| RFQ built, proved and sent | 05:38:49.1 | 6.2 s |
| RFQ mined (block 4614927) | 05:38:49 | |
| Maker reads the RFQ and sends the quote | 05:39:16.7 | 27.6 s |
| Quote mined (block 4614930) | 05:40:37 | |
| Taker verifies the quote | 05:40:50.3 | 93.6 s |
| Accept sent | 05:40:54.1 | 3.8 s |
| Maker reads the accept | 05:41:20.7 | 26.6 s |
| Escrow `open` in a block | 05:41:23.3 | 2.6 s |
| Stipend in a block | 05:41:26.9 | 3.6 s |
| Taker verifies the escrow (10 checks, final block) | 05:41:34.4 | 7.5 s |
| Fund sent | 05:41:35.1 | 0.7 s |
| Fund mined (block 4614934) | 05:41:37 | |
| `set_ready` in a block (3 confirmations, k_Z = 3) | 05:42:02.7 | 27.6 s |
| `commit_claim` in a block | 05:42:07.5 | 4.8 s |
| `claim` in a block | 05:42:13.0 | 5.5 s |
| Claim final (seen by the taker) | 05:42:18.1 | 5.1 s |
| `withdraw` by the maker in a block | 05:42:18.4 | 0.3 s |
| Taker `Completed` | 05:42:24.9 | 6.5 s |
| Sweep sent | 05:42:26.5 | 1.6 s |
| Sweep with 3 confirmations, maker `Swept` | 05:43:54.9 | 88.4 s |
| Exit `execute_intents` in a block | 05:44:04.5 | 9.6 s |

Totals: 222 s to the taker's `Completed`, 312 s to the maker's `Swept`, 322 s with the
exit. Each proof took about 3 to 6 s on this machine (native, all cores). Most of the
time is waiting for Zcash blocks.

Zcash block times: over the 995 block intervals from 4,613,950 to 4,614,945 the mean was
23.0 s and the median 16 s (the maximum 211 s). This agrees with a 25 s target after NU7;
the consensus rule itself was not checked. The configs still use values for 75 s blocks
(`fund_secs` 900, `k_z` 3), which are larger than needed.

## Balances

| | Before | After |
|---|---|---|
| Taker wallet (zats) | 100,000,000 | 89,949,000 |
| Maker wallet (zats) | 100,000,000 | 109,991,000 |
| Maker tUSDC (base units) | 19,999,000,000 | 19,996,000,000 |
| Maker NEAR (yocto) | 875,594,396,581,694,799,999,997 | 834,012,232,903,572,599,999,996 |
| Maker storage funds in the escrow (yocto) | 491,040,000,000,000,000,000,000 | 485,520,000,000,000,000,000,000 |
| Escrow tUSDC | 0 | 0 |
| Taker key in the Verifier (tUSDC) | 0 | 3,000,000 after the swap, 0 after the exit |
| Exit destination `zecret-3f109eea1e.testnet` (tUSDC) | 3,000,000 | 6,000,000 |
| Taker key account NEAR (yocto) | 0 | 28,047,577,857,125,900,000,000 (stipend less commit and claim gas) |

The Zcash balances add up. Taker: 100,000,000 − 10,000 (refused RFQ) − 10,000 (RFQ) −
11,000 (accept fee and premium) − 10,020,000 (fund and fee) = 89,949,000. Maker:
100,000,000 − 10,000 (quote) + 1,000 (premium) + 10,000,000 (joint note less the sweep
fee) = 109,991,000.

The maker spent 0.04158 NEAR: the 0.03 stipend, and gas for `open`, the stipend
transfer, `set_ready`, `withdraw` and the exit relay. The swap's escrow storage (0.00552
NEAR) came from the maker's storage funds.

## Problems met

1. **Amount above the maker's limit: the taker gets no answer.** The template maker
   config has `max_zats` 10,000,000. The maker refused a 50,000,000-zat RFQ with a log
   line only (`crates/zecret-core/src/maker.rs:440-441`, `bail!`). The taker gets no
   refusal memo and waits up to `quote_timeout_secs` (1,800 s). The directory entry
   (`zecret directory`) does not publish `min_zats` or `max_zats`, so the taker cannot
   check the amount first. Configuration, not a fund risk; it cost one 10,000-zat fee.
2. **The taker's own `withdraw` can never be sent with the stipend.** After the claim,
   the taker tries `withdraw` with 200 Tgas (`crates/zecret-core/src/taker.rs:1045-1056`,
   `GAS_WITHDRAW` in `near/escrow.rs:32`). The RPC refused it: `NotEnoughBalance`,
   balance 0.02805 NEAR, cost 0.20092 NEAR. The taker then waited and the maker's
   `withdraw` paid out 6 s later. This is the designed path ("the maker sends the
   withdraw call"), and no funds were at risk. But if the maker stops after the claim,
   the credit stays in the escrow until some account with about 0.2 NEAR sends
   `withdraw`; the taker's key cannot. The attempt also costs an RPC round trip each
   swap.
3. **A wrong warning in the maker log** (cosmetic). The maker logged `requests wait for
   a quote, but the wallet has 0 spendable zats, less than a quote fee (10000)` in the
   same tick that it broadcast the quote. Cause: `pend` (`maker.rs:418-422`) keeps the
   stage at `Quoting` until the next tick broadcasts; in that tick the note is reserved,
   `spendable` is 0, and the check at `maker.rs:333-339` fires. The maker had 1 TAZ. The
   quote went out normally.
   Status of items 1 to 3 after this run: the maker now answers a request it does not
   quote with a signed refusal memo, publishes its limits in its signed directory entry,
   and the taker checks them before it sends anything; the taker no longer tries a
   `withdraw` it cannot pay for and reports `PayoutPending` with the call if the maker
   does not send it; the false warning is gone; the testnet configs now use the measured
   block times. See `docs/security/fixes-2-taker.md` sections 6 to 8. The hosted swap
   below ran with this code and with the maker's signed limits. It did not cause a
   refusal or `PayoutPending`, so those two paths were not run on testnet.
4. The deploy tool was not built (disk space), so `zecret-deploy maker-key` was not run;
   the same file was written by a short script (step 2).

## Not verified

- The exit to another chain through 1Click: it does not exist on testnet.
- The browser taker: this run used the native taker.
- Refund and punish paths on testnet: only the happy path ran.
- Cover delays: off in these configs.
- The Zcash explorer pages (`https://zexplorer.app/testnet/tx/<txid>`, the link format
  that Fauzec gives) were not opened. The transactions were read through the light-wallet
  server only.
- That NU7 changed the testnet block target to 25 s: only the block intervals were
  measured.

## Where the run can continue

`.tools/testnet/maker` and `.tools/testnet/taker` keep their wallets (109,991,000 and
89,949,000 zats) and their records. Another swap needs a new taker run with an amount
between 100,000 and 10,000,000 zats; the maker's directory entry does not change. The
Fauzec cooldown for these two addresses ends 24 hours after 05:36 UTC on 2026-11-13.

## Hosted setup (2026-11-13)

The website and a maker with its relay are hosted for public use on the same test
networks. Hosts, settings, secrets and how a push to `main` redeploys:
[`docs/HOSTING.md`](HOSTING.md).

### What was run

1. Railway project `zecret`, service `zecret-maker`, built from
   `deploy/maker/railway/Dockerfile` on the branch `main` (commit `d6bac89`). The first
   build took about 2 minutes.
2. At its first start the service made a new wallet and a new NEAR key on its volume
   (`ZECRET_NEW_WALLET=1`, `ZECRET_NEW_NEAR_KEY=1`, then set back to `0`). It printed only
   public values:
   - wallet `utest10kyef9lq7c637hlxx3fwdfrkukqnttzswcr4ukes5h3ewzca4e4nzycm73txxfgf44azfyngreys08dpzxqp8eah8dwacktj3s5yu0wv`;
   - NEAR public key `ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH`;
   - the signed directory entry with limits (100,000 to 10,000,000 zats, premium 1,000
     zats), kept in `deploy/maker/railway/directory-entry.testnet.json`.

   The earlier plan was to copy the wallet of the native run into a Railway variable. It
   was not done: the key would have passed through the build machine's session in clear.
   A key made on the volume never leaves it.
3. The native maker wallet of the run above paid 50,000,000 zats to the hosted wallet for
   its quote fees: Zcash txid
   `b49b2d844aee5e15b2a5272cdda5ea732be38881eef9790479517b6708947f99`.
4. The website's directory `web/public/config/testnet-directory.json` was made from the
   hosted entry with `deploy/web/directory-from-entry.mjs` (rate 30 tUSDC per TAZ, the
   maker's `price_per_zec`).
5. A second deployment (a redeploy at 11:00 UTC) started with the same wallet and NEAR
   key and logged `config.json matches the image`: the volume kept its files.

### Checked

- The relay answers over HTTPS from outside Railway: `OPTIONS /execute-intents` returns
  204 with `access-control-allow-origin: *`; `POST` with `{}` returns 400.
- `zecret maker --relay` with `chains.network` `main` stops with "--relay is for the local
  network and testnet only" (run on the build machine with the same code).

### Hosted swap (2026-11-13, 11:33 to 11:42 UTC)

One swap through the hosted site in headless Chromium, run by `testnet/hosted/run.mjs`.
The payer was the native testnet taker wallet. Result file:
[`testnet/hosted/result-0.05.json`](../testnet/hosted/result-0.05.json) (`run.mjs`
writes `result.json` on each run; this is the copy of this run).

| Item | Value |
|---|---|
| Site | `https://zecret-chrismgs-projects.vercel.app` (commit `908f763`) |
| Paid | 0.05 TAZ (5,000,000 zats) |
| Received | 1.4877 test USDC on NEAR testnet, account `zecret-3f109eea1e.testnet` |
| Time from payment to done | 493 s |
| Taker's NEAR account | `aa30f22c8cb6c2361984ffb2d9094fcc6c967e2956a786f340ec27b1fb5a843e` (new key, gas from the maker's stipend) |

| Step | Chain | Transaction |
|---|---|---|
| Payment to the page's wallet | Zcash | `721caf712d80630c1f22982791f3329fccb1a141ae2ea851f1638bc7c02d2aef` |
| Request memo | Zcash | `e05eb06125cdb72fa25d039e2ee7a86ac9dc7149e8ac5ced65f3ce3115d6321c` |
| Acceptance and premium | Zcash | `28f6474119d0fb310601bbc723a3ba8f9cf328ffcca2eede54f0d7a103db65e8` |
| Funding of the shared address | Zcash | `2c7df41c91e56dadfd6723e5e670168d489ed2d2269f9d82616246e9fdfccd30` |
| `commit_claim` | NEAR | `8uh57oHrWRxcLDRHHDGnzXs9ayPNuyoP3DfX1TqR4Rti` |
| `claim` | NEAR | `D4gdMsdEYku88YMyHa9j5DKTVT9wHGo77mnM1CEEswA6` |
| Signed exit, sent by the maker's relay | NEAR | `DAKZAsvRKaN9hb6JJTUhLThtVgxpx1PbCccrLsDiXxvT` |

Checked after the run, not taken from the page:

- NEAR RPC: `commit_claim` and `claim` were signed by the taker's account and succeeded.
  `execute_intents` was signed by the maker account with the hosted maker's key and
  succeeded.
- `zecret-testnet-audit`: the four Zcash transactions are version 6 with no transparent
  inputs or outputs and no Sapling or Orchard parts; each txid was recomputed from the
  raw bytes.
- The page reported no Content-Security-Policy violation.

In the browser: key build 20 s; proofs 29 s, 40 s and 31 s; the relay answered in 4 s.

### Hosted swap that refunded (11:26 to 11:32 UTC)

A first run with 0.01 TAZ stopped with `QUOTE_BELOW_MINIMUM` and the ZEC went back to the
return address (refund txid
`2aaebd8f9245a3ee60f21a0a9baba48ecfbd529c1161d73a15a6c7ae30c1b087`). Result file:
`testnet/hosted/result-refund-0.01.json`.

Cause: the website's price estimate subtracted three network fees
(`web/src/lib/price.ts`), and the engine reserves four (`required_deposit` in
`crates/zecret-core/src/taker.rs`). For 1,000,000 zats the site set a minimum of 287,793
micro-USDC and the maker's correct quote was 287,700. The difference is larger than the
1% tolerance for amounts below about 1,050,000 zats.

### Hosted swap after the price fix (12:07 to 12:14 UTC)

A rerun with 0.01 TAZ on the site with the fixed price estimate (commit `5d35000`), run
the same way by `testnet/hosted/run.mjs`. Result file:
[`testnet/hosted/result-0.01-fixed.json`](../testnet/hosted/result-0.01-fixed.json). The
result file of the 0.05 TAZ run above is
[`testnet/hosted/result-0.05.json`](../testnet/hosted/result-0.05.json).

| Item | Value |
|---|---|
| Site | `https://zecret-chrismgs-projects.vercel.app` (commit `5d35000`) |
| Paid | 0.01 TAZ (1,000,000 zats) |
| Received | 0.2877 test USDC (287,700 micro-USDC) on NEAR testnet, account `zecret-3f109eea1e.testnet` |
| Time from payment to done | 392 s (12:07:40 to 12:14:11) |
| Page | Reached "done"; no Content-Security-Policy violation |

| Step | Chain | Transaction |
|---|---|---|
| Payment to the page's wallet | Zcash | `9fba50e7efeb09aac0246f86135272c306a83f350b7e22f8b13a179e8a75639e` |
| Request memo | Zcash | `cc72efd93389c0ea16441e1f8f764efa3969627291a22c8adc727c5e997322bc` |
| Acceptance and premium | Zcash | `e490eac9c116bfa25a82f0cfd0aee116287de83cc2c1ed3648c01f74712e6d91` |
| Funding of the shared address | Zcash | `e2eff18977c1133a72df4bff7f4b7589b07d64af65f59fbe7abf8915b6122e8e` |
| `commit_claim` | NEAR | `9z3av19SmUrqjyqhU7uCsvJTNmFRUPDsdnwrSxKJzPW` |
| `claim` | NEAR | `8xVtDKbzCcnmQCddE8EwW5EJqQZciXJmmvbMHXFbDCWY` |
| Signed exit, sent by the maker's relay | NEAR | `C9T9LJvohjbuhfcjXuVG9ZiFpHvh6dF2pQYFksDbuNAJ` |

The transactions of this run are taken from the page's result file. They were not
audited separately (no RPC check and no `zecret-testnet-audit` run, unlike the 0.05 TAZ
run).

### Hosted swap paid by the faucet (12:45 to 12:51 UTC)

No wallet paid this swap. `testnet/hosted/run.mjs` with `ZECRET_PAY=faucet` sent one
claim to the Fauzec API for the page's deposit address. The faucet pays a fixed 1 TAZ
(100,000,000 zats). The page asked for 0.05 TAZ, swapped that amount, and kept the rest
for the return address (the test wallet of the native run). Result file:
[`testnet/hosted/result-faucet-0.05.json`](../testnet/hosted/result-faucet-0.05.json).

| Item | Value |
|---|---|
| Site | `https://zecret-chrismgs-projects.vercel.app` |
| Paid | 1 TAZ by the faucet (claim `01M7AF2PMFWXT7V5S7533Q1D97`); the swap used 0.05 TAZ (5,000,000 zats) |
| Received | 1.4877 test USDC (1,487,700 micro-USDC) on NEAR testnet, account `zecret-3f109eea1e.testnet` |
| Time from payment to done | 314 s (12:45:58 to 12:51:12) |
| Taker's NEAR account | `040f110c1be1ae6081262960aa763fbadce91a127e03a2b1edb964fb46d24856` |
| Page | Reached "done"; no Content-Security-Policy violation |

| Step | Chain | Transaction |
|---|---|---|
| Faucet payment to the page's wallet | Zcash | `f9105dd87822797f511f703725078d040a723fdd8516fea478193297239b74a5` |
| Request memo | Zcash | `f5320a8d6f48ecef33f10cd8e0ccaf3fcac1bc30808aa547a0caa8a3c966389a` |
| Acceptance and premium | Zcash | `a696ac7db1c9eb6a78633dd51351c6c533e3eb0b083a5ecb75f77bf7034c245e` |
| Funding of the shared address | Zcash | `32201776780197c1f188d5557524a01234ea73b6af587cfd1637f13327624c76` |
| `commit_claim` | NEAR | `HwMr8MSseJzrqk5oPGQ2vamSiqa4JE629BxDX9Ty9wH5` |
| `claim` | NEAR | `Ckg42b1nRMdWWzp3kHKGasGVnMCRCT6p3oQjJnXCYRQg` |
| Signed exit, sent by the maker's relay | NEAR | `DJ1BgVavScKvPnZE1S2EGapybLaAh7mem78zAQpfz3JR` |

Stages from the result file (UTC): `awaiting_deposit` 12:45:48, `deposit_seen` 12:46:18,
`request_sent` 12:46:47, `quote_received` 12:47:30, `accepted` 12:48:01, `escrow_open`
12:48:31, `funding_sent` 12:49:04, `ready` 12:50:52, `claimed` 12:50:58, `paid_out`
12:51:06, `done` 12:51:12. In the browser: key build 20.8 s; proofs 27.7 s, 28.8 s and
31.0 s; the relay answered in 3.9 s.

The runner closed the browser at "done". The page had not yet sent the surplus, so it
stayed in the swap's temporary wallet. The transactions of this run are taken from the
page's result file; they were not audited separately.

### Restore that returned the surplus

`node testnet/hosted/restore.mjs <site> <recovery file>` loaded the recovery file of the
faucet-paid swap in a fresh headless browser on the hosted site and waited until the page
said "It went back to". The page sent the surplus to the return address:

| Item | Value |
|---|---|
| Leftover transaction (Zcash) | `33bb8defb4bbc41716cf978c6c2339dec76f1a579446b1561a27cefac1dca34f` |
| Amount | 94,990,000 zats (100,000,000 − 5,000,000 swapped − 10,000 fee) |
| Return wallet balance (`zecret balance`) | 83,899,000 zats before, 178,889,000 after (+94,990,000) |

The balances were read by the main build session with the project's command-line wallet.
This also shows the gap that the website now closes: a swap that reaches "done" while
leftover ZEC is still unsent keeps its keys, and "Start a new swap" waits until the
leftover transaction is mined (or the user confirms leaving it behind).

### Maker outage and the swaps after the fix (2026-11-13 and 2026-11-14)

From about 14:30 to 16:35 UTC on 2026-11-13 the hosted maker sent no quotes: the node
refused every transaction it built ("unknown Ironwood anchor"). Two swaps through the
site refunded after the 10-minute quote wait. Cause, fix and transactions:
[`docs/incidents/2026-11-13-maker-unknown-anchor.md`](incidents/2026-11-13-maker-unknown-anchor.md).
The fix was deployed at 16:35 UTC (commit `cdaf614`).

Two swaps through the site completed after the fix, each 0.01 TAZ for 0.2877 test USDC:

- 16:41 to 16:49 UTC on 2026-11-13, the swap recorded for the demo video. Facts:
  `testnet/hosted/demo/demo-facts.json`.
- 00:57 to 01:04 UTC on 2026-11-14, a health check 8.5 hours after the deployment.
  Result file: `testnet/hosted/result-0.01-after-sync-fix.json`. Zcash: request
  `2f4641227d0052814f903d48d62a89cdff8eb45415b9e56cc5b16840901e0b7d`, acceptance `40167313b64098d11a9c90d6cb65531756da4f9b36ae875fc3f8a3d1e7fd58d0`, funding `4fc20d8485b69947aa4d2f6a961a9f88b9b2a1e88c73ded231ae040c7c8bcf21`.
  NEAR: `commit_claim` `HkjduriUhApmqiDdJPpeYhssMfzPbLYGtQDQaWpb77Z9`, `claim` `HiBWv49ZgJoMGnEN6KwTtFk7QagPbijhgLBdEVT4cZc4`, exit
  `5gXPiER9ex5cqZGPikwZESbybu1QkPoBFzfforE1Dxfi`.

The transactions of these two runs come from the page's result files. They were not
audited separately.

## Soak test, 2026-11-14

Real 0.01 TAZ swaps through the hosted site (`https://zecret.chrismg.dev`), driven by
`testnet/hosted/soak.sh` and `testnet/hosted/soak-one.sh` in headless Chromium, paid from
the command-line testnet wallet. Result lines: `testnet/hosted/soak/results.jsonl`. Every
failure and fix: `testnet/hosted/soak/NOTES.md`.

| Phase (UTC) | Escrow | Completed | Not completed |
|---|---|---|---|
| 03:00 to 06:07, one swap every 15 minutes | `escrow2` | 12 | 1: the test script's payer wallet got HTTP 502 from the light-wallet server before the site was opened |
| 06:25, health swap after the switch | `escrow3` | 1 | |
| 06:30 to 09:45 | `escrow3` | 0 | The test machine stopped during a swap. The maker then missed that swap's refund: its only NEAR endpoint refused every request (`docs/incidents/2026-11-14-maker-missed-refund.md`). Site paused, fix released at 09:29 |
| 09:40, deliberate abandoned swap | `escrow3` | refund in 5 minutes | |
| 10:21 | `escrow3` | 1 | 2 slots lost: the test machine is suspended when the build session is idle |
| 11:08 to 12:44, 12 swaps back to back | `escrow3` | 12 | 0 |

Totals: 26 swaps completed through the site on this day (25 soak lines and the health
swap). No swap that ran to its end failed. The 12 back-to-back swaps took 364 to 574 s each
(mean 472 s). After them the maker's health route showed `ok`, 900 NEAR requests since
its start at 09:31, and no rate limiting.

The lost slots are faults of the test setup, except the missed refund, which was a fault
of the maker and is fixed. The transactions of these swaps are in the per-slot result
files; they were not audited one by one.

## A fresh testnet payer

These commands make a new command-line wallet, fund it from the faucet and pay an address.
They need Rust (the version in `rust-toolchain.toml`) and are run from the repository
root. `check`, `init` and `balance` were run on 2026-11-13 for this list; the claim and
`pay` were not run with this new wallet (the same commands funded and paid in the runs
above).

```sh
(cd crates && cargo build -p zecret-node --release)
Z=$PWD/crates/target/release/zecret
P=.tools/payer                                 # not in git: it will hold the wallet key
mkdir -p $P && cp crates/zecret-node/config/testnet-taker.json $P/config.json
TIP=$($Z check --dir $P | sed -n 's/^Zcash .*: tip \([0-9]*\).*/\1/p')
python3 -I -c 'import json,sys; c=json.load(open(sys.argv[1])); c["chains"]["birthday"]=int(sys.argv[2]); json.dump(c,open(sys.argv[1],"w"),indent=2)' $P/config.json "$TIP"
UA=$($Z init --dir $P)                       # the wallet's unified address
curl -X POST https://fauzec.com/api/v1/claim -H 'content-type: application/json' \
  -d '{"network":"testnet","address":"'"$UA"'"}'
$Z balance --dir $P                          # repeat until zats is 100000000
$Z pay --dir $P --to <address from the page> --zats <amount in zats>
```

The faucet gives one claim per address per 24 hours and has a cooldown per network, so a
claim can be refused for a while. Its web form at <https://fauzec.com> has a human check;
the API above did not ask for one on 2026-11-13.

## Light payout on testnet

The switch rehearsal of 2026-11-14 05:00 UTC (a new maker `maker4` that had never used
the Verifier, the site and taker configuration of the switch commit, one native and one
browser swap) is in [`docs/LIGHT_PAYOUT_SWITCH.md`](LIGHT_PAYOUT_SWITCH.md), "Rehearsal".

Date: 2026-11-14, 04:05 to 04:36 UTC. Branch `escrow-light-payout`. The live site and the
hosted maker were not touched: they still use `escrow2`. These runs used a new escrow and
a new maker next to them.

### What ran

| Part | Value |
|---|---|
| Escrow | `escrow3.zecret-3f109eea1e.testnet`, code hash `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536` (471 730 bytes, source commit `dccd718`), no access keys, Verifier `intents.zecret-3f109eea1e.testnet` |
| Deployed with | `zecret-deploy deploy --config deploy/configs/testnet-light.toml` (owner key used in place through a link, not copied); record in `deploy/deployment.testnet-light.json` and the section `light_payout` of `deploy/deployment.testnet.json` |
| Deployment transactions | escrow account `62LaNpQ6CPAmTZ8FfTbHKckREomub9QxHG4g2mYaz1dE`; deploy + `new` + delete key `FMSRRYU8qsLwHaeWeMnjjyxPWCPcRaytqXQPxbYmgAmA`; maker account `D4MNN4zbLCY9L81gtLKArZA7yMyxDSMuqdBJ43VPzWgA`; token registration `Dahwa9SCEbo3Xng8pxRD6WqFQcZocrMb6989MSLvCwNA`; mint 200 tUSDC `3Wr1WEL12W8hUSBzh8euhYLT289jJt1N7jSEvnkAfh8Z`; storage funds `7ywR89HfwMxCnAg1W7R8Kdhmtvpr4pukTJdSk86qdav5`; deposit of 100 tUSDC into the maker's Verifier account `3n8s6Ejtbj8q5WWJ72xvkSZy8bC44CZjiUbrPg2zfHc7` |
| Maker | `maker3.zecret-3f109eea1e.testnet`, a new account funded by the owner (2 NEAR); a new Zcash wallet; the native `zecret maker` on this computer with its relay on `127.0.0.1:3411`; stipend 0.06 NEAR |
| Zcash funds | Fauzec faucet, one claim each for the new maker wallet and a new payer wallet (`97ad930b…c07c`, `5fbc637e…ebf4`, both accepted). The payer paid 0.03 TAZ to each of three new taker wallets. The old wallets in `.tools/testnet/` were not used. |
| Binaries | `zecret` built from this branch (debug profile, optimised); `zecret-testnet-audit` from `testnet/tools` |

`zecret-deploy verify --config deploy/configs/testnet-light.toml` after the runs: all
checks pass; the escrow's Verifier balance is 0 and it owes 0; the maker holds 99.1123
tUSDC in its Verifier account (100 minus 0.3 + 0.3 + 0.2877 paid out; the refunded 0.3
came back there).

### 1. Normal swap, native taker (`debfe1a4`)

0.01 TAZ for 0.3 tUSDC. The taker's key sent `commit_claim`, `claim` and `withdraw(id)`.

| Step | NEAR transaction | Signer | Result (RPC `tx`, final) |
|---|---|---|---|
| open (`mt_transfer_call` on the Verifier) | `5dEfDzDfurGBmKf7e4L7vJiUqsmVYNW9MnqNqWVKhdLL` | maker3 | ok, event `opened`, 15.51 Tgas |
| stipend 0.06 NEAR | `RfhYw8d5b7LPyrcQQigADRPdbte4rW14EkhP1Ld5jW5` | maker3 | ok, 0.007115 NEAR burnt (account creation) |
| `set_ready` | `8X8Sspi4MiNzFeM8og9aDeFBrLm5qAd3BHww4wAEgh7B` | maker3 | ok, `ready` |
| `commit_claim` | `5B9nY5q3JxYM3ufgJ2B91QYQsUsQqF49bwRXH9HVNbeK` | taker key `5fbd6e68…d09c` | ok, `locked` |
| `claim` (25 Tgas) | `7qutzFN2UWLBLTSoHap9mtGSGnaKesHpHRKiQtvriZrG` | taker key | ok, `claimed`; payout Due |
| `withdraw(id)` (25 Tgas) | `38foMhdX32ewMLYJxTLZr4sZJ9Duf6g5mjegpVk8SEvb` | taker key | ok, `paid`; 0.3 tUSDC under the taker key in the Verifier |

The taker key holds 0.057126 NEAR afterwards.

### 2. The maker is killed when the claim is final (`336d13ce`)

A watcher (`python3`, log `.tools/testnet-lp/logs/swap2-watch.log`, not in git) started the
taker, read the taker account's NEAR balance every second, and killed the maker process
(`kill -9`) when the taker logged "claim is final", at final height 277 607 377. The maker
was then in stage Ready: it had not seen the claim and had sent no `withdraw`. The taker
finished alone.

| Step | NEAR transaction | Block | Signer |
|---|---|---|---|
| open | `2ZPgDGpeW9s8k7jyDp2xSeNcpwnN7YvxbvQ1twiqGccA` | 277 607 022 | maker3 |
| stipend | `CEimbhGpU4j1ZRJ3vAspXghrq9eiRbSGBAtauz17RKLR` | 277 607 028 | maker3 |
| `set_ready` | `CxrysAgGSfPcKTuAFzKgszbeT1EagbVffGsBunkFoRS5` | 277 607 354 | maker3 |
| `commit_claim` | `EXcU4VYaFfTgPBeML5qaXLkLFQ7UijfWB4cAHg9kFNzU` | 277 607 362 | taker key `f0899f16…ab4` |
| `claim` | `EFYy87ZFPQagaBZQLtvmn6LHVVZVjdo9eXAcGiVYcCW` | 277 607 373 | taker key |
| maker killed | | final height 277 607 377 | |
| `withdraw(id)` | `Bd2Wpzt9V1YS6BtYPgaRsxcEQM6SCM91iMrrzsJV6wTe` | 277 607 382 | taker key; event `paid` |

All six succeeded with no failed receipt (RPC `tx`, `wait_until: FINAL`). After it:
`get_swap` shows `Claimed`, payout `paid`, recipient the taker key;
`mt_balance_of(taker key)` is 300 000 (0.3 tUSDC).

The taker key's NEAR balance, read every second (yoctoNEAR / 10^24). Each call first
lowers it by the cost NEAR checks (attached gas at 10^9 yoctoNEAR per gas), then the
refund arrives:

| Moment | Balance (NEAR) | Change |
|---|---|---|
| after the stipend | 0.060000 | |
| `commit_claim` submitted | 0.044079 | −0.015921 held for 15 Tgas |
| after its refund | 0.059739 | spent 0.000261 |
| `claim` submitted | 0.033819 | −0.025920 held for 25 Tgas |
| after its refund | 0.058074 | spent 0.001665 |
| `withdraw` submitted | 0.032155 | −0.025919 held |
| after its refund | 0.057124 | spent 0.000950 |

Total spent by the taker's key: 0.002876 NEAR. The largest balance needed was before
`withdraw`: 0.001926 already spent + 0.025919 held = 0.027845 NEAR.

The maker was started again afterwards; it read `a` from the escrow and swept the ZEC
(`f4c26153…783b`). With the maker gone, the taker could not have run the testnet exit,
which goes through the maker's relay; the tokens waited under the taker's key.

### 3. Maker refund (`9aaaae98`)

The taker stopped after its acceptance (`--vanish-at Accepted`) and never funded. Once
the funding window closed, the maker refunded before t0. The refund attached 50 Tgas, so
its payout ran in the same transaction.

| Step | NEAR transaction | Result |
|---|---|---|
| open | `6LbvPkgx5WszaDThbhNM1yHgGf9N4quZnRo7UhBZuuMx` | ok, `opened` |
| stipend | `BA5KcGi7g8ynv5zVzPvLUvBf4WfTrJNQzD8ykM9FCgz1` | ok |
| `refund` | `7UuTenAbf7HT9XrenU15vjPJ3imuWXqby9Tm7BiyJPhJ` | ok, events `refunded` and `paid`; 0.3 tUSDC back in the maker's Verifier account |

### 4. One swap with the browser engine (`8fd1350b`)

A production build of this branch's site (`VITE_ZECRET_CONFIG=testnet-light npx vite
build --outDir dist-light`) with `testnet/light/testnet-light.json` (escrow3, its code
hash, the local relay) and `testnet/light/testnet-light-directory.json` (maker3's signed
entry), served by `testnet/light/serve.mjs` with the headers of `web/vercel.json`.
`testnet/hosted/run.mjs` drove it in headless Chromium (with
`ZECRET_PROXY_BYPASS=127.0.0.1,localhost`, because the browser otherwise sent the local
requests to the outbound proxy and got 405). The payer wallet paid the page's address.
Result: `testnet/light/browser-result.json`, screenshots `testnet/light/browser-*.jpg`.

The page went from the payment to `done` in 311 s (04:31:27 to 04:35:55 UTC after the
payment was seen; 577 s from opening the page). It showed "You received 0.2877 test USDC
on NEAR". No Content-Security-Policy violation.

| Step | Transaction | Signer |
|---|---|---|
| user payment | Zcash `57e7c881…0745` | payer wallet |
| request, acceptance, funding | Zcash `8acae810…1989`, `42454bf3…4389`, `234ecf7b…6a50` | browser wallet |
| open | `6xTTHskSatEkjdrPnq8LV6qLNtwpi4vUV8teWQ6s3c71` | maker3 |
| stipend | `6kW9axRSTM5pSiV9SVJHjQ2n9FBqY57vtxJWMjCbfrQ` | maker3 |
| `set_ready` | `J2xEMK6kSPbCNjdMEXhrktbYNKeg2uqvdU5V9pAfAHNR` | maker3 |
| `commit_claim` | `97dAuu48hjSb5ok2NozAvGSUNJBABXgojPos9wbF5VRE` | browser's key `0b7d52b1…cf4c` |
| `claim` | `9ZhqsNHmp8maUWrH6cH1fNk8vAEYWKPhE4SURmayEqMb` | browser's key |
| `withdraw(id)` | `5rJeiMuJRecYXzbDHAmSKGB4JDnX5R9xZCkbfp2DMw4z` | browser's key; event `paid` |
| exit (`execute_intents`, `ft_withdraw` signed by the browser's key) | `Et1d7ceCtbrDDhoh77RWfZRckbmSJrA3TSu2a71W3sen` | maker3 (relay) |

The browser's key holds 0.057142 NEAR afterwards. Engine timings in the browser: proving
key 19.9 s, proofs 27.7 s, 27.4 s and 30.9 s, exit relay 4.9 s.

### Checks

- Every NEAR transaction above was read back with RPC `tx` (`wait_until: FINAL`): signer,
  receiver, method, status `SuccessValue`, no failed receipt, the escrow's events
  (`.tools/testnet-lp/scripts/neartx.py`, not in git; output in this section).
- Shielded audit (`zecret-testnet-audit`, through ChainSafe's gRPC-web server) of 22
  Zcash transactions: the three payer payments to the takers and the one to the browser
  wallet, both takers' and the browser's requests, acceptances and fundings, the maker's
  four quotes and three sweeps, the refund taker's request and acceptance. Every one is
  v6 with 2 Ironwood actions, 0 transparent inputs and outputs, 0 Sapling, 0 Orchard,
  value balance 10 000 zats (the fee), and its txid recomputes.

### Not verified here

- The hosted website and the hosted maker with `escrow3` (see `docs/LIGHT_PAYOUT_SWITCH.md`).
- A Verifier that refuses the payout on testnet (tested in the sandbox only).
- Mainnet.
