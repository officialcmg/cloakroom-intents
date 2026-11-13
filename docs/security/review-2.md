# Security review 2: the swap engines (`zecret-core`, `zecret-node`)

Date: 2026-11-13. Reviewed code: the working tree of `/home/user/zecret` on that date. The
directory has no git metadata, so there is no commit id. `crates/zecret-core/src/taker.rs`
changed during the review (another session added `reconcile` and a "never fund twice"
check in `on_open`, file time 04:50 UTC). All findings were checked again on that
version (sha256 prefix `9ef1d2cd1c95`). Line numbers refer to it.

Machine-readable records: `findings-2.json` (15 records; the method's
`validate-findings.cjs` says PASS). Proofs of concept: `poc2/`.

## 1. Result

| Id | Severity | Title |
|---|---|---|
| F2-01 | Medium | After a refund the taker sweeps one joint note, then erases `a` while its funding note is still unspent |
| F2-02 | Medium | The first `claim(a)` is sent without checking R2r again, so a late hand-off publishes `a` after the reveal window |
| F2-04 | Medium | Unpaid requests add permanent work to every maker tick and can delay R3d refunds past t2 − B_R |
| F2-03 | Low | The taker accepts any escrow window at least as long as quoted; a reveal window of u64::MAX overflows and freezes both legs |

Needs validation: NV2-01 (one NEAR RPC endpoint is a trust anchor for the taker), NV2-02
(the premium is not tied to the stipend and the open costs).

No finding lets a third party take funds. F2-01 and F2-02 let a malicious maker destroy
or take an honest taker's ZEC under the conditions given. F2-04 lets any sender slow the
maker. A taker with one swap of its own can then take the escrowed tokens.

## 2. Scope and method

**In scope.** `crates/zecret-core` (all of `src/`) and `crates/zecret-node` (`io.rs`,
`main.rs`). The rules come from the Cloakroom yellow paper (Tables 1 and 2, §3 to §5,
read with `pdftotext`), `docs/DESIGN.md` and `docs/ENGINE.md`. Review 1 covered the escrow
contract and `zecret-intents`. This review does not repeat that scope.

**Method.** Cloudflare's published security-audit method
(github.com/cloudflare/security-audit-skill), in guidance mode, by one reviewer: recon,
hunt, validate, record. The companion files used were PROTOCOLS-RPC-AND-MESSAGING,
RESOURCE-EXHAUSTION-AND-AVAILABILITY, DATA-ISOLATION-AND-LIFECYCLE and
SUPPLY-CHAIN-AND-RELEASE. Each candidate was argued against before it was kept. A
finding is confirmed only with a full source trace and a local test. A missing second
layer is a hardening note. Severity is likelihood × impact, and it is never higher than
the impact.

**Execution.** Only local code ran:

- `cargo test -p zecret-core --lib`: 25 tests pass.
- The PoCs in `poc2/` ran from a scratch workspace that depends on
  `crates/zecret-core` by path. It used the existing `crates/target`. The scratch
  workspace is in the session scratchpad, not in the repository.
- The PoCs drive the public `tick` API. A scripted JSON-RPC transport stands in for
  Zebra and NEAR. No network, sandbox, testnet or mainnet was used.
- The harness (`harness/run.py`) was not run. No finding needed it.
- No source file was changed.

**Covered.**

- Taker:
  - every wallet rule as implemented (R1 with the added checks, R1q, R1b, R2c, R2r, R4);
  - hostile quote values (windows, horizon, amounts, overflow, units) and hostile
    directory entries;
  - a look-alike swap, an early refund, Ready without a note, and withholding;
  - stored-state ordering around every reveal and payment;
  - the new `reconcile` and "never fund twice" code, read briefly.
- Maker:
  - R3e, R3d, R3l and L_M, and the premium check before any spending;
  - spam, replays and malformed memos.
- NEAR adapter: the borsh layout, nonces, block hashes, `wait_until`, timeout handling,
  and the parsing of RPC answers.
- Memos: the AEAD keys and nonces, the labels, the binding to request ids and the
  parsers.
- Keys:
  - the Schnorr proofs (challenge binding and rogue keys);
  - sign normalisation and the joint key;
  - the randomness sources on wasm;
  - the signature domains of the maker key.
- Stores: `MemStore`, `FileStore` and the order of writes, restarts and rollback.
- Supply chain: `Cargo.lock`, the pins, the sources and the build scripts.

**Not covered.**

- The TLA+ model in `formal/` was not compared with the code.
- The original `node/` client was used only through the paper.
- The Ironwood proving and signing internals (`pczt`, `orchard`) and the shard-tree
  witness code were not reviewed.
- How far the light wallet can trust its server (tree state, compact blocks, missing
  reorg handling) is listed only as hardening.
- Out of scope, because other sessions are editing them: `crates/zecret-wasm`, `web/`,
  `contracts/` and `deploy/`.
- No end-to-end run on a chain was made for F2-01, F2-02 or F2-04. The chain-side steps
  of those findings are reasoned from the code.

## 3. Confirmed findings

### F2-01 (Medium): recovery erases `a` while a funded joint note is unspent

**Where.** `crates/zecret-core/src/taker.rs`

| Line | What happens |
|---|---|
| 871 | `on_funded` sees `Refunded` at a final block and calls `recover`. |
| 1073 | `recover` takes `wallet.unspent(rfq_id).first()`, the first note at the joint address in scan order, whatever it is. |
| 1103 | It sweeps only that note with a + b. |
| 1115 | `on_recovering` sets `Recovered` once that sweep has k_Z confirmations. It never looks for other joint notes. |
| 368 | `step` erases `a`, `s_t`, `e_t` and the joint key in the terminal stage. |

**Lower-trust principal.** The maker, against an honest taker.

**Attack.**

1. The maker learns the joint address when it accepts (maker.rs:508).
2. It broadcasts a note of more than 10 000 zats to that address together with the
   stipend. The fee is high enough that the note is mined in the funding's block, ahead
   of the funding, or one block earlier.
3. The taker's "never fund twice" check (taker.rs:822) sees only mined notes. A note
   that is still in the mempool does not stop the funding.
4. The maker refunds instead of setting Ready. The escrow allows this in Open before t0,
   or after t1.
5. The taker sweeps the maker's note, then erases `a`.
6. Her funding note stays at the joint address. Nobody can spend it: the maker has b but
   not a.

A second funding by the taker herself ends the same way. Since the new check at
taker.rs:822, that is less likely.

**Impact.** The taker loses the whole funded amount and gets no tokens. This breaks
Definition 1 (taker safety) and rule R4. The maker gains nothing. It spends the note, a
fee, the stipend and the escrow gas.

**Severity.** Likelihood low: it needs a maker that wants to do damage, and a note order
the maker can arrange but not guarantee. Impact high. Overall medium.

**Reproduce.** `poc2/taker_pocs.rs`, `poc_f2_01_…` (see section 7 for how to run it). On
the reviewed code it fails with: `F2-01: stage Recovered, a erased, but 100010000 zats of
the taker's funding are still unspent at the joint address (now unspendable)`.

**Fix (smallest).** In `recover`:

- Ignore notes at or below the sweep fee.
- Sweep the note from `fund_txid` first.

In `on_recovering`, before the stage becomes `Recovered`:

- If a joint note above the fee is still unspent, clear `sweep_height` and `sweep_txid`
  and call `recover` again.
- Keep `a` while `fund_txid` is not mined and the tip is at most `fund_expiry + k_Z`.

The exact code is in `findings-2.json`. Add the PoC as a regression test.

### F2-02 (Medium): R2r is not checked at the hand-off of the claim

**Where.** `crates/zecret-core/src/taker.rs`

| Line | What happens |
|---|---|
| 960 | `on_committed` checks R2r (now + B_R < lockedAt + W) and stores the stage `Revealed`. The claim is left for the next tick. This is correct. |
| 353 | The next tick starts with a Zcash light-wallet sync. A sync error returns before any NEAR work, and the tick is retried every 5 s. |
| 1001 | `on_revealed`, state `Locked` and `claim_at` None: there is no deadline test. |
| 941 | `send_claim` signs and broadcasts `claim(a)`. If the escrow refuses it, `a` is still public in the arguments. |

**Lower-trust principal.** The maker. The attack works best for a maker that runs the
taker's gRPC-web light-wallet endpoint. DESIGN.md §10 plans to run one.

**Attack.**

1. The maker sees the lock become final.
2. It lets one more light-wallet request through, then fails all requests until
   lockedAt + W has passed. The same delay comes from a natural outage, or from a browser
   tab that is closed between the two ticks.
3. The taker then sends `claim(a)`. The escrow reverts it, but `a` is public.
4. The maker refunds the lock (R3l), which publishes b.
5. The maker now knows a + b. It sweeps the joint note while it still blocks the taker's
   light wallet.

The maker's own refund path checks its rule again at the hand-off (maker.rs:787–797).
The taker's claim path does not. An honest maker does not use `a`, so with an honest
maker the swap only fails, and the taker recovers.

**Impact.** The taker can lose the ZEC while the maker keeps the tokens.

**Severity.** Likelihood low: it needs a hand-off delay larger than the slack, plus a
maker that races for the note. Impact high. Overall medium.

**Reproduce.** `poc_f2_02_…`. On the reviewed code it fails with: `F2-02: the taker
broadcast claim(a) 50s after lockedAt + W; a is now public in a reverted transaction`.

**Fix.**

1. In `on_revealed`, when `claim_at` is None, check `in_time(tnow, lockedAt + W)` again.
   Use `saturating_add` for the deadline. If the check fails, go back to `Committed`. Do
   not send.
2. Make the first claim two-phase:
   - one tick signs the claim and stores it (`claim_tx`, the raw bytes, `claim_at`);
   - the next tick broadcasts it.

   Then `claim_at == None` means that nothing left the engine. Re-send the stored bytes
   while `tnow < deadline`.
3. In `step`, a Zcash sync error stops the tick only in the stages that need the Zcash
   wallet. In `Funded`, `Committed`, `Revealed` and `Claimed`, emit a warning and
   continue. A refund that needs a sweep still fails safely, because `build` syncs again.

### F2-04 (Medium): unpaid requests grow the maker's tick without bound

**Where.** `crates/zecret-core/src/maker.rs` and `wallet.rs`

| Line | What happens |
|---|---|
| maker.rs:260, 466 | Any valid request memo becomes a `Quoting` swap. There is no premium, no cap and no expiry. `Quoted` swaps do not expire either. |
| maker.rs:287 | Every tick advances every non-terminal swap, in rfq_id order. The sender chooses the rfq_id. |
| maker.rs:581 | `send_quote` tries `build_pay` again every tick. |
| wallet.rs:676 | `build_pay` first syncs, which is one RPC round trip. Then the note choice fails ("insufficient funds") when the maker's notes are reserved by quotes that are not mined yet, or when its ZEC for quote fees is used up. `reply_address` and the note choice are each O(n). |
| maker.rs:793 | R3d: if a visit to a swap comes too close to t2, the maker logs "L_M violated" and does not refund. |
| maker.rs:472, 269 | An Accept memo for an unknown request is never marked as seen. It is retried, with a warning, every tick. |

**Measured** (mock RPC, one tick):

| Stuck requests | RPC round trips per tick | CPU per tick | Still `Quoting` after 30 days |
|---|---|---|---|
| 100 | 101 | 0.09 s | 100 |
| 400 | 401 | 0.56 s | 400 |
| 1600 | 1601 | 4.2 s | 1600 |

**Lower-trust principal.** Anyone who can send Zcash memos to the maker's published
address. For the token loss, the same person also takes one real swap: it pays the
premium and never funds.

**Attack.**

1. The maker refunds an Open or Ready swap after t1 in one tick, and must send the
   refund in a later tick before t2 − B_R.
2. With the testnet windows (t2 − t1 = 900 s, B_R = 110 s), a tick longer than about
   400 s makes the maker miss L_M.
3. With 100 ms per light-wallet round trip, about 4 000 stuck requests do this. That is
   about 0.2 ZEC of the attacker's fees. The requests can be built offline and sent
   once, and they stay until they are quoted.
4. The attacker's own swap then reaches t2 without a refund. The attacker commits
   through the punish path and claims the USDC (Proposition 1).

**Impact.** The maker loses the escrowed tokens of every attacker swap whose refund it
misses. Honest swaps are delayed: Ready comes late or not at all, and those swaps
refund.

**Severity.** Likelihood low: it needs volume, timing, and a round-trip time that was not
measured on the deployment. Impact high. Overall medium. The first two steps are shown
locally. The missed refund is reasoned from the source.

**Fix.**

- Add `max_pending_quotes`. `on_rfq` refuses requests while `Quoting` + `Quoted` reach
  it.
- Add `quote_ttl_secs`. Store the request time in `MakerSwap`. `Quoting` and `Quoted`
  swaps older than the TTL become `Aborted`, and their secrets are erased.
- In `send_quote`, skip the build without any RPC when
  `wallet.spendable("wallet") < zip317_fee(1, 2)`. Build at most `max_quotes_per_tick`
  quotes per tick.
- In `step`, advance the swaps in `Opening` to `Sweeping` before `Quoting` and `Quoted`.
- Mark an Accept for an unknown request as handled.

### F2-03 (Low): R1 uses ≥ for the windows; the window arithmetic overflows

**Where.** `crates/zecret-core/src/taker.rs`

| Line | What happens |
|---|---|
| 782–784 | R1 compares the escrow's windows with the quote using `>=`. The escrow refuses only W = 0 (contracts/near-escrow/src/lib.rs:462). |
| 844 | The taker funds. |
| 958 (and 1012) | `locked_at + reveal_window` is computed in u64. |

**Effect of W = u64::MAX.**

- A debug build (the native reference taker) panics in `tick`.
- A release or wasm build wraps the sum, so R2r never passes and the taker never reveals.
- The escrow computes the sum in u128, so the maker can never refund the lock either.
- Both legs stay frozen until the taker runs a client without the overflow. `a` is kept,
  so the taker can still claim later.

`v.t0 + t1_secs` and `v.t1 + t2_secs` (lines 782–783) also panic in a debug build when
t0 is close to u64::MAX. In a release build, the horizon check at line 797 stops that
case.

**Severity.** Likelihood low. Only the maker can cause it, and it freezes its own tokens
too. Impact low, because the funds are frozen, not lost. Overall low.

**Reproduce.** `poc_f2_03_r1_…` and `poc_f2_03_huge_…`. The first test fails because the
taker emits `EscrowVerified` for W = u64::MAX (the quote said 600 s). The second panics
at taker.rs:958 with "attempt to add with overflow".

**Fix.**

- Require equality: `v.t1.checked_sub(v.t0) == Some(t1_secs)`,
  `v.t2.checked_sub(v.t1) == Some(t2_secs)` and `v.reveal_window == reveal_secs`. The
  maker writes exactly these values (maker.rs:643–647).
- Use `saturating_add` for every deadline that comes from escrow values.

## 4. Needs validation

### NV2-01: one NEAR RPC endpoint is a trust anchor for the taker

**What the code does.**

- The taker reads all NEAR state from one RPC endpoint, with no proof:
  - the escrow (R1, code hash, keys, Intents account) at taker.rs:761 and 794;
  - the time of the final block;
  - Ready and Locked at taker.rs:951.
- It gives the signed `claim(a)` to the same endpoint (taker.rs:941).

**What a lying RPC can do.**

1. It shows a matching Open escrow that does not exist, then Ready, then a final lock.
2. The taker funds the joint address and hands `a` to the RPC.
3. Together with the maker, who has b and both seeds, that is enough to sweep the joint
   note. No tokens were ever escrowed.

An RPC alone can only delay the swap or make it stall, because it does not have b or the
seeds. DESIGN.md and ENGINE.md do not state this trust assumption.

**Fact that is not known.** Which endpoints the production website gives to takers, and
who runs them. `web/src/engine/api.ts:127` takes `nearRpcUrl` as an option.

**Local check.** Use the mock in `poc2/common.rs`. Answer `get_swap`, `view_account` and
`view_access_key_list` with matching values for an escrow that does not exist. The taker
emits `EscrowVerified` and builds the funding.

**Owner check.** Name the endpoints and their operators. Then either:

- document that the RPC operator must not collude with the maker, and let users choose
  the endpoint; or
- read the open, the Ready and the lock from a second, independent RPC before the taker
  funds or reveals.

### NV2-02: the premium does not cover the stipend

**What the code does.** The maker sends the 0.03 NEAR stipend to the taker's own
account before any funding (maker.rs:706). It also pays the open gas, the storage and the
refund gas. The taker pays only the premium and two Zcash fees, and it keeps the stipend.
The maker checks only that the premium paid is at least `premium_zats` (maker.rs:490).

**Values in the repository.**

- `testnet-maker.json`: `premium_zats` is 1 000.
- `deploy/maker/README.md`: `premium_zats` is 20 000.
- DESIGN.md:176 says "The premium covers it."

**Fact that is not known.** The mainnet values and the ZEC and NEAR prices at launch.

**Owner check.** Make sure the premium is worth more than the stipend + open gas +
refund gas + storage, with a margin. Add a startup check in `Maker::open` that uses
prices given in the configuration.

## 5. Rejected candidates

These are recorded in `findings-2.json` with their reasons.

- **A rogue key B.** The proof of knowledge binds the request id, the role and the point.
  The taker verifies it before B is used (taker.rs:727).
- **Memo AEAD key or nonce reuse.** Each (label, key pair) encrypts one message. Quote
  and Accept use different labels. A rebuilt request is the one exception: see H6.
- **Memo parser panics and allocation bombs.** All lengths are bounded by the 512-byte
  memo, and the cursor checks every read.
- **Mixed time units.** The escrow and the client both use whole seconds of NEAR block
  time. The wall-clock `now` is used only for timeouts and cover delays.
- **A look-alike swap.** The taker computes the escrow id itself from the maker account
  and the salt, and checks every field.
- **An early refund, Ready without a note, or withholding.** With one funding, the taker
  keeps `a` until it can sweep. It loses at most the premium. The case with several
  notes is F2-01.
- **NEAR transaction layout and nonces.** The layout matches `Transaction::V0`. A re-sent
  claim is a new transaction and fails without harm.
- **An RPC timeout taken as success.** Every caller reads the escrow again at a final
  block on the next tick. The worst result is a refund instead of a completed swap.
- **The pinned pre-release Zcash crates.** They are exact pins with checksummed lock
  entries from the upstream repositories. There are no git sources, no `[patch]` and no
  build scripts in our crates. The owner should still confirm the crates.io publishers.

## 6. Hardening notes (not vulnerabilities)

- **H1. Quote check fails open.** `reject_quote` returns "no objection" when the
  configured maker key does not parse: `parse_public_key(..).ok()?` at taker.rs:721. Only
  `verify_maker` in `start` protects this today. Return a rejection instead.
- **H2. Zcash sync blocks the maker's NEAR duties.** `Maker::step` returns at once when
  the Zcash sync fails (maker.rs:251). An outage of the light-wallet server during
  [t1, t2 − B_R) makes the maker miss L_M on every open swap. Run the NEAR duties
  (refunds, Ready, withdraw) even when the sync fails. For the taker, see F2-02 fix 3.
- **H3. gRPC-web frame length can overflow.** `i + 5 + len` (zcash.rs:75) can overflow on
  wasm32, where `len` comes from the server. Release builds then panic on a slice. Use
  `checked_add`. A hostile server can already stall the client, so this is
  defence-in-depth.
- **H4. FileStore durability and permissions.** `FileStore::put` renames without `fsync`
  of the file and its directory, so a power loss can bring back an old record. For
  example, `Opened` could come back after a refund made b public. The wallet key, the
  RFQ key and the swap records are written with the default umask. Call `fsync` before
  and after the rename, and create the files with mode 0600.
- **H5. Test-only settings in production configs.** `misbehave` is accepted from the
  taker and maker configuration JSON. For example, `no-reveal` makes an honest taker
  never reveal. Allow it only behind a cargo feature that is off in release builds.
- **H6. Rebuilt request reuses a nonce.** A request that is rebuilt after a refusal
  encrypts a different plaintext (a fresh PoK nonce) under the same key and the zero
  nonce. Only the maker, who has the key, sees both. Derive the PoK nonce
  deterministically, or store the sealed memo.
- **H7. Reserved notes are not stored.** `Wallet::reserved` is lost on a restart. After a
  restart, another build (for example `return_leftover`) can spend the inputs of a
  pending funding. The result is a conflicting transaction, not a loss.
- **H8. The light wallet trusts its server.** It has no reorg handling and does not check
  that each block hash links to the one before. It relies on k_Z and an honest server.
  State this in ENGINE.md.
- **H9. Native HTTP limits.** The native transport reads response bodies without a size
  limit and waits up to 120 s per request. This blocks the single-threaded maker loop
  (see F2-04 and H2).
- **H10. "Funded" from any joint note.** `on_open` and `reconcile` take any note at the
  joint address as the funding (taker.rs:663, 822), including a note the maker sent.
  This is safe for the taker, because she then funds nothing. But `fund_txid` can then
  name a maker transaction. The F2-01 fix must not depend on `fund_txid` alone.

## 7. Proofs of concept (`poc2/`)

| File | Tests |
|---|---|
| `common.rs` | A scripted JSON-RPC transport, a taker configuration, a note builder |
| `taker_pocs.rs` | `poc_f2_01_…`, `poc_f2_02_…`, `poc_f2_03_r1_…`, `poc_f2_03_huge_…` |
| `maker_pocs.rs` | `poc_f2_04_…` (prints the growth table) |

How to run them:

1. Copy `common.rs` to `crates/zecret-core/tests/poc2_common.rs`.
2. Copy the PoC file to `crates/zecret-core/tests/`.
3. In the copied PoC file, replace `use super::common::*;` with
   `#[path = "poc2_common.rs"] mod common; use common::*;`.
4. Run `cd crates && cargo test -p zecret-core --test <file> -- --nocapture`.

All five tests use only the public API and fail on the reviewed code. Each test passes
once its finding is fixed. F2-03 has two tests: the R1 fix makes the first pass, and the
`saturating_add` fix makes the second pass.
