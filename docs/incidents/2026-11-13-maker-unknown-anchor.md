# Incident 2026-11-13: the hosted maker's quotes were refused ("unknown Ironwood anchor")

Network: Zcash testnet and NEAR testnet. Service: the hosted maker on Railway
(`deploy/maker/railway`), light-wallet server `https://zcash-testnet.chainsafe.dev`.

## Summary

From 14:30 UTC the hosted maker answered no swap request. It decrypted each request, built
a quote transaction, and the node refused every quote transaction with "unknown Ironwood
anchor". The maker built the quote again every few seconds and was refused every time.
The anchor of a transaction is the root of the wallet's note commitment tree. The maker
had run without a restart since 12:33:50, and its tree was no longer the tree of the
chain. Short-lived wallets (the command-line wallet, freshly opened browser wallets)
were not affected: they scan from their birthday each time.

## Impact

- Two swaps did not run. Each taker stopped after its 10-minute quote timeout and got
  its ZEC back, less the transaction fees.
  - Taker payment `ed012cee980576876ff6b4d6a4806d66b0b08be517014c5570e4f87ca1ca5976`
    (block 4,616,186, 14:28:38). Request read by the maker at 14:30:10 (swap
    `3425730f`). Refund `f1a1d2e7de299c3f2f9e9fdbb93cf98bffcb89ee8e0fabd2622b2e02ee48d8ec`
    (block 4,616,224, 14:40:26).
  - Taker payment `14a7e49eaec064d0c712c760df13d75d4dd5bde155d21cc8b4326fe15f81ba61`
    (block 4,616,233, 14:45:48). Request read at 14:48:07 (swap `d657cae1`). Refund
    `37dd359dd32a1a47ddb29521d2893987476c3707080a89b5a3b7a5a32fd2c212` (block 4,616,261,
    14:59:22).
- No funds were lost. No escrow was opened, so no USDC was locked.
- The maker kept building quotes for both requests after the takers had stopped. Each
  build is a proof (seconds of CPU). This went on until at least 15:05.

## Timeline (UTC)

Block heights and times are from the light-wallet server (`zecret-testnet-audit`,
`zecret-testnet-chaincheck`). Maker times are the Unix times in its log lines.

| Time | Event |
|---|---|
| 12:33:50 | The maker container starts. The wallet scans from its birthday (4,614,922). |
| 12:45 | Swap `8279226c` is accepted. Adding the swap's viewing key makes the wallet rescan from the birthday. |
| 12:51:04 | The maker's sweep `edf2e1f6…` is accepted by the node and mined in block 4,615,959. The maker's tree was correct at that time. |
| 12:53:38 | Swap `8279226c` ends (`Swept`). |
| 12:53:45 to 14:30:10 | The maker logs nothing. No sync warning: every sync in this period returned without an error. |
| 14:28 | The command-line wallet pays a taker wallet (`ed012cee…`, block 4,616,186). It works. |
| 14:30:10 | The maker reads request `3425730f`. Quote `97fd3cee` is refused: unknown Ironwood anchor `34987c52…`. "building it again" repeats every 2 to 4 s. |
| 14:40:26 | Refund `f1a1d2e7…` of the first taker (block 4,616,224). |
| 14:41:49 | The refused anchor is now `4bb389d9…`: new blocks were added to the same wrong tree. |
| 14:43 | The command-line wallet pays again (`14a7e49e…`, block 4,616,233). It works. |
| 14:48:07 | The maker reads request `d657cae1`. Its quotes are refused in the same way. |
| 14:59:22 | Refund `37dd359d…` of the second taker (block 4,616,261). |
| 15:05:23 | The refused anchor is `37c98a1d…`. Both requests are still being quoted. |

Earlier on the same day a browser wallet that had been open for a long time (restored from
a recovery file) was refused once with the same error. A retry after the wallet was
opened again from storage worked.

## Root cause

**The defect (certain).** The wallet (`crates/zecret-core/src/wallet.rs`) kept its tree
as a list of every note commitment it had scanned and built each anchor from that list. It
had no defence against a list that differs from the chain:

1. It did not check that a block links to the block before (`prev_hash`). After a chain
   reorganisation it kept the commitments of the replaced blocks and never scanned the
   blocks that replaced them. Nothing ever removed them.
2. It applied a block range action by action. An error in the middle of a range (for
   example a failed fetch of a full transaction) left part of the range applied, and the
   next sync applied it again.
3. It set its height to the end of the requested range, even if the server sent fewer
   blocks.
4. A refused transaction kept its notes reserved until its expiry height (40 blocks). In
   the regtest reproduction the maker had no other note and stopped building quotes
   without a warning.
5. A wrong tree was permanent while the process ran. A refusal for the anchor did not
   change the tree, so every rebuilt quote got the same refusal.

**The event (likely, not proven).** What made the tree wrong between block 4,615,959 (the
last accepted transaction, 12:51) and the first refusal (14:30) is not known with
certainty. The evidence:

- The refused anchors are not the chain's tree at any height in the 15 minutes before each
  refusal (`testnet/tools/src/chaincheck.rs`). So the tree was wrong, not only behind
  the node.
- The server's chain today is consistent from the birthday to the tip: every block links
  to the one before, the tree size after each block matches the block's chain metadata,
  and the computed root matches `GetTreeState` at three heights.
- No single fault in one block from 4,615,900 to 4,616,272 gives the refused anchor: a
  skipped block, a block applied twice, the first k actions applied twice, one action
  missing, or an orphaned block whose transactions are any subset of the transactions
  mined in the next three blocks (4,263 cases, none matches).
- No sync error was logged between 12:53 and 14:30. A fault of kind 2 logs a warning, so
  it did not happen. A fault of kind 3 needs a server that answers a range with fewer
  blocks and status 0; the server today answers a range past its tip with an error
  (grpc-status 11), and twelve probes found one consistent server.
- A testnet reorganisation explains every observation: the maker scanned blocks that a
  reorganisation then replaced, and their commitments stayed in its tree. Wallets that
  scan from their birthday each time (the command-line wallet, browsers opened again) are
  not affected. A second long-running wallet (the browser) had the same error on the same
  day, which points to the chain, not to the maker. On a local Zebra regtest chain, the
  pre-fix maker binary shows the same refusal after a reorganisation of 8 blocks
  (`harness/run.py s24` with `ZECRET_BIN` set to the old binary; log in
  `harness/evidence/s24_reorg_before_fix/`). The taker there stopped after its quote
  timeout, as on testnet.
- An orphaned block cannot be read after the fact. The orphan cases above cover only one
  block whose transactions were all mined later; an orphan with transactions that were
  never mined, or a deeper reorganisation, gives a root that cannot be found this way.

## Fix

- The wallet applies a block completely or not at all, and its height moves block by
  block. A stream that ends early skips nothing.
- Each block must link to the one before. The wallet keeps the hash and tree size of the
  last 100 blocks. On a block that does not link, it rolls back to the newest block that
  the server's chain also has, and scans the new chain. A tip replaced at the same height
  is found by its hash. With no common block among the kept ones, it rescans from the
  birthday.
- A block whose Ironwood tree size (from the server) does not fit is not applied. If the
  wallet's own size already differs from the server's tree, it rescans.
- A refusal for an unknown anchor starts a repair: roll back to a common block, compare
  the tree root with the server's tree at the same height, and rescan from the birthday
  if they differ (at most once per tip height). The notes of a refused transaction are
  freed at once. `Wallet::pay` builds the payment once more after a repair.
- The anchor is the tree two blocks below the tip (or of the newest block with a spent
  note).
- The maker drops a request whose quote no node took within `quote_send_secs` (300 s)
  after the maker read it. Before, it rebuilt the quote for up to `quote_ttl_secs`
  (3600 s).
- Rollbacks and repairs are logged as `Zcash wallet: …` lines.

Details: `docs/ENGINE.md` §1.4. Tests: `crates/zecret-core/src/wallet_sync_tests.rs`
(reorganisation of depth 1 and 3, a replaced tip, a reorganisation deeper than the kept
blocks, a short stream, a stream cut in a frame, a block with a missing action, the repair
after a refused anchor), `crates/zecret-core/tests/maker_refusal.rs`
(`a_quote_not_sent_in_time_is_dropped`), and `harness/run.py s24`.

On testnet, the old native maker wallet (`.tools/testnet/maker`) paid itself with a tree
corrupted on purpose (`zecret pay --corrupt-tree`, test builds only). The node refused the
first transaction for its anchor, the wallet rescanned, and the second transaction
`04a0fa8759e4283e8afb7a3ef0ec67999012d233a59d059c3df66c1e3e6e3f04` was accepted.

## Not known

- Which blocks made the maker's tree wrong, and whether testnet reorganised at that time.
  Replaced blocks cannot be read now.
- Whether the light-wallet host had more than one backend at that time.
- The hosted maker's log does not show its sync heights, so the exact tip at each refusal
  is not known (the search used every block in the 15 minutes before).
