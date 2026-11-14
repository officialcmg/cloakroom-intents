# Switch the live testnet site and maker to the light-payout escrow

This is a plan. Nothing in it has been done on the live services. Branch:
`escrow-light-payout`. Read [`docs/security/review-5.md`](security/review-5.md) and the
section "Light payout on testnet" of [`docs/TESTNET.md`](TESTNET.md) first.

## What changes

| | Now (live) | After the switch |
|---|---|---|
| Escrow | `escrow2.zecret-3f109eea1e.testnet`, code `DuBGKiy8dYPFkVHXLj4VFHLZiZ4ngQzssRUbXTCuPKq1` | `escrow3.zecret-3f109eea1e.testnet`, code `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536` (deployed 2026-11-14, no access keys, Verifier `intents.zecret-3f109eea1e.testnet`) |
| Maker's tUSDC | plain balance on `usdc.zecret-3f109eea1e.testnet` | in its own Verifier account; the maker deposits the difference before each open by itself |
| Payout | the maker sends `withdraw` (200 Tgas) | the taker's key sends `withdraw(id)` (25 Tgas); the maker only backs it up |
| Stipend | 0.03 NEAR | 0.06 NEAR (the taker still accepts 0.03) |
| Maker account | `maker.zecret-3f109eea1e.testnet` (unchanged) | same account, same key, same wallet and volume |

The token, the Verifier, the owner and the maker account stay. `escrow2` stays deployed
and keeps working for swaps that were opened on it; nobody needs to touch it.

## The one rule

**Switch the maker only when it has no open swap.** The new `zecret` binary reads the
escrow's `payout` field, which `escrow2` does not have; an `escrow2` swap that is still
running when the new binary starts cannot be read by it (the maker logs an error for that
swap and does nothing with it). Old browser tabs that still point at `escrow2` are safe:
they check the escrow named in the maker's quote against their own config and stop
before they pay the premium, losing only one request fee.

## Steps, in order

1. **Pause new swaps on the website.** On `main`, set `"status": "paused"` in
   `web/public/config/testnet-directory.json` and push. Vercel redeploys the site
   (`docs/HOSTING.md`); the form then refuses to start a swap. Wait for the deployment to
   be live (`https://zecret.chrismg.dev/config/testnet-directory.json` shows `paused`).
   The maker is not rebuilt by this push: since 2026-11-13 Railway rebuilds it only when
   a file under `deploy/maker/` changes (`STATUS.md`, "Maker deploys"; `docs/HOSTING.md`
   still names `crates/**` too).
2. **Wait until the hosted maker has no open swap.** In the Railway logs of
   `zecret-maker`, the last start prints `open swaps: N`; a swap in flight finishes within
   its windows (at most `t0 + t1 + t2` = 40 minutes with the testnet template, usually
   under 10). The check: no `swap/*.json` on the volume has a stage other than `Swept`,
   `Refunded` or `Aborted`, the same rule `deploy/maker/railway/start.sh` uses. Also stop
   the soak or health-check runs against the hosted site for the duration.
3. **Merge `escrow-light-payout` into `main`** (fast-forward or a merge commit), and in the
   same push change the four live settings below. One commit is easiest to roll back:
   - `crates/zecret-node/config/testnet-maker.json`: `chains.escrow` =
     `escrow3.zecret-3f109eea1e.testnet`, `stipend` = `60000000000000000000000`.
   - `crates/zecret-node/config/testnet-taker.json`: `chains.escrow` = `escrow3…`,
     `escrow_code_hash` = `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536`.
   - `web/public/config/testnet.json`: `near.escrow` = `escrow3…`, `near.escrowCodeHash`
     = `AqzB7mtMCvNfffA3vgYSPezYa132U7eZ18BdNQNsS536`. `minStipend` stays
     `30000000000000000000000`.
   - `web/public/config/testnet-directory.json`: `maker.escrowContract` = `escrow3…`;
     keep `status` = `paused` in this push (the signed entry itself does not change: the
     escrow is not in the maker's signature).
   - `deploy/maker/RELEASE`: one new line (this is what makes Railway rebuild the maker),
     for example
     `2026-11-14: light payout: escrow3, tokens in the Verifier, the taker sends withdraw(id); stipend 0.06 NEAR`.
   - Optional, for the record: `testnet/config/*.json` and `deploy/configs/testnet.toml`
     (`[escrow] sub_account = "escrow3"`) so that `zecret-deploy verify --config
     configs/testnet.toml` checks the new escrow. Do not run `zecret-deploy deploy` with
     that file: the escrow exists; `deploy/configs/testnet-light.toml` is the config it
     was made with.
   Do not merge the branch without these settings in the same push: Vercel would build
   the new engine with the old site config (`escrow2`), and the new engine cannot read
   `escrow2` swaps.
4. **Railway rebuilds the maker** (the push changes `deploy/maker/RELEASE`). At start,
   `deploy/maker/railway/start.sh` sees that the image's `config.json` differs from the
   volume's and that no swap is open, so it writes the new settings and keeps the old file
   as `config.json.prev-<time>`. Check the start log:
   - `config.json updated from the image (no open swap)`;
   - `settings: {"escrow":"escrow3.zecret-3f109eea1e.testnet", …}`;
   - `open swaps: 0`.
   If the log says `WARNING: the image's settings differ from config.json, but N swap(s)
   are open`, step 2 was not finished: the maker then still uses `escrow2` with a binary
   that cannot read it. Roll back (below) and redo step 2.
5. **Vercel rebuilds the site** with the new engine and config. Check
   `https://zecret.chrismg.dev/config/testnet.json` shows `escrow3` and the new code hash.
6. **Before the first real user:** the maker account needs, before its first open on
   `escrow3`:
   - storage funds in `escrow3`: the maker adds `storage_topup` itself when its funds are
     below `storage_min` (logged as `added … yoctoNEAR of storage funds to the escrow`);
   - tUSDC in its Verifier account: it deposits the difference from its plain tUSDC
     before the open (logged as `deposited … into the maker's Verifier account before the
     open`). It held 20 000 tUSDC plain at the last read of `deploy/deployment.testnet.json`.
   - NEAR for gas: about 0.07 NEAR per swap (stipend and account creation) plus 0.1 NEAR
     free for a 100 Tgas open.
   Nothing has to be done by hand for these.
7. **Set the site online again**: one more push with `"status": "online"` in
   `web/public/config/testnet-directory.json`, once both deployments are live.
   **Run one swap through the site** (the health-check script):
   `node testnet/hosted/run.mjs https://zecret.chrismg.dev <payer folder> <NEAR account> 0.01`.
   Check on testnet NearBlocks: the `withdraw` transaction of the swap is signed by the
   taker's implicit account (64 hex), not by `maker.zecret-3f109eea1e.testnet`; the
   escrow's `get_swap(id).payout` is `paid`; `mt_balance_of(escrow3, nep141:usdc…)` ≥
   `get_owed(usdc…)`; `zecret-deploy verify --config configs/testnet-light.toml` passes.
8. Update `README.md` ("Try it, step 4" names `escrow2`), `docs/HOSTING.md`,
   `STATUS.md` ("Open by decision: … the taker cannot pay for `withdraw`" is no longer
   true) and `deploy/deployment.testnet.json` (`escrow` becomes `escrow3`; `escrow2`
   moves to `retired_escrows`; the section `light_payout` already records `escrow3`).

## Roll back to escrow2 in one commit

`git revert <the switch commit>` and push. That restores the old binary, the maker template
(`escrow2`, stipend 0.03) and the site config. Before the push, again pause the site and
wait until the maker has no open swap on `escrow3`: the old binary cannot read `escrow3`
swaps (it expects `get_credit`), and `start.sh` keeps the volume's config while a swap is
open. Funds are not stuck by a roll-back:

- tUSDC the maker deposited into its Verifier account stays there. The old maker opens
  from its plain balance and does not see it. To move it back, sign with the maker key:
  `ft_withdraw(token = usdc.zecret-3f109eea1e.testnet, receiver_id =
  maker.zecret-3f109eea1e.testnet, amount)` on `intents.zecret-3f109eea1e.testnet` with 1
  yoctoNEAR and 50 Tgas.
- A swap left Due on `escrow3` can still be paid by anyone with `withdraw(id)`.

## Risks

- **An open swap at switch time.** Covered by steps 1, 2 and 4. If it happens anyway, the
  affected swap needs a manual `withdraw(token, owner)` on `escrow2` (any account, 200
  Tgas, about 0.2 NEAR held) after its claim, and the maker's refund duties for it must
  be done with the old binary.
- **Old browser tabs** keep the old engine until reloaded. They refuse quotes that name
  `escrow3` before paying the premium; the user loses one request fee.
- **Recovery files saved before the switch** for swaps on `escrow2` restore into the new
  engine with the new config (`escrow3`). A swap that already ended is unaffected; a swap
  still on `escrow2` must be finished before the switch (step 2).
- **The maker's NEAR.** The stipend doubles. Watch the maker account's balance.
- **NEAR Intents operators** of the stand-in Verifier hold its full-access key on testnet
  (`deployment.testnet.json`, "keeps its access key"); on testnet that is the owner.
- **Not verified by the switch itself:** mainnet; Firefox and Safari; a maker restart in
  the middle of an `escrow3` swap on Railway (tested locally: s4, s25 in the harness).

## Recommendation

See the final report of this branch for the recommendation on whether to switch before the
submission. Doing nothing is also safe: `escrow2` keeps working as before.
