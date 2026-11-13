# NV-02: the escrow's Intents withdraw with the real USDC on NEAR

Date: 2026-11-13. Review item: [`review-1.md`](review-1.md) NV-02. Checklist:
[`fixes-1.md`](fixes-1.md) section 5. Nothing was committed. No contract source in the
repository was changed; the proposed change is in section 6.

## 1. Question

`withdraw` pays a taker credit with `ft_transfer_call(intents_account, amount, msg =
owner)` on the token. If that promise ends in `Failed`, `on_withdraw` restores the full
credit. This is correct only if a `Failed` result means that nothing was delivered. For
USDC on NEAR (`17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1`), can
`ft_transfer_call` end in `Failed` after `intents.near` accepted the tokens?

## 2. Answer

**Yes. The rule is not safe with the real USDC.** USDC's `ft_resolve_transfer` panics if
the contract is paused, or if the sender (the escrow) or the receiver (`intents.near`)
is blocklisted. If Circle pauses USDC, or blocklists one of these two accounts, after
`intents.near` credited the taker and before the resolve step runs, the
`ft_transfer_call` result is `Failed`, `on_withdraw` restores the credit, and the next
`withdraw` pays the taker a second time from other swaps' USDC.

This was reproduced in the NEAR sandbox with the USDC code downloaded from mainnet and
the live `intents.near` code (section 4.3): after one claim of 1000 USDC, the taker held
2000 USDC in Intents, and the escrow held 0 USDC while another swap of 1000 USDC was
still open.

Nobody but Circle can cause the trigger. The window is short (one or two blocks per
withdraw). But a pause is a normal Circle operation, and a pause request is visible on
chain before it is executed (it needs a multisig request and approvals), so a taker who
watches for it can time withdraws to the window. The escrow must not depend on this.

A change that removes the problem was implemented in a scratch copy and tested (section
6): the escrow deposits into its **own** Verifier account and then pays the taker with
`mt_transfer` inside the Verifier, which is one atomic receipt. With it, the same race
pays the taker exactly once.

## 3. The source and the deployed code

### 3.1 Where the source is

- Repository: <https://github.com/circlefin/stablecoin-near> (Apache-2.0, "Circle
  stablecoins on NEAR"). HEAD `80e2760` (2025-07-28). Tags `release-2024-01-23T150659`
  (`6d07903`) and `release-2025-07-28T223851` (`80e2760`).
- `src/` is the same in every commit from `a5190cb` (2023-11-07, "Initial commit:
  release fiat_token") to HEAD. Later commits change only `Cargo.toml`, `Cargo.lock`,
  tests and CI.
- The token is near-contract-standards 4.1.1 `FungibleToken` wrapped by
  `src/fiat_token.rs`, with near-sdk 4.1.1 and near-sdk-contract-tools 1.1.x (roles and
  a 2-of-n multisig).

### 3.2 Does it match the deployed code

Mainnet, `https://free.rpc.fastnear.com`, final block 224310980:

- `view_account`: `code_hash` `DVJx7F3dHEHJYwpYPwXHu8K9Uo7h4kXFcALucY5oNX8q`.
- `view_code` (block 224310990): 321491 bytes. Its SHA-256 in base58 is the same
  `DVJx7F3d…`, so the downloaded file is the deployed code.
- `ft_metadata`: name `USDC`, 6 decimals (the name set by `migrate` in this source).

Comparison:

1. Panic strings in the deployed wasm name the build machine's registry paths:
   `near-contract-standards-4.1.1`, `near-sdk-4.1.1`, `near-sdk-contract-tools-1.1.0`,
   `base64-0.13.0`, `serde-1.0.189`, `serde_json-1.0.107`, `once_cell-1.16.0`,
   `itoa-1.0.3`, rustc commit `84c898d65` (Rust 1.69.0, which the README requires).
   Every application string (`FiatToken: paused`, `FiatToken: … is blocklisted`,
   `FiatToken: not approved for upgrade`, the role names, the action names) is in the
   source.
2. Build: commit `a5190cb` (its lock file has near-sdk-contract-tools 1.1.0), Rust
   1.69.0, the six crates above pinned to the versions in the deployed strings,
   `--remap-path-prefix` to the deployed path, `cargo build --lib --target
   wasm32-unknown-unknown --release`, custom sections removed:
   - the export section is byte-identical (same 40 exports in the same order), and the
     import, table, memory and global sections are byte-identical;
   - the data section has the same size (33075 bytes); the differences are function
     table indices;
   - the code section is 285457 bytes against 285458 deployed.
   - Not byte-identical, so the build is **not** reproduced. The remaining difference
     is most likely another transitive crate version; this was not pursued.

How sure: high that the deployed code is this source (same strings, same exports, same
toolchain, same section layout, near-identical size), not proven byte for byte. **The
conclusion does not depend on this**: every behaviour in section 4 was observed by
running the deployed wasm itself in the sandbox.

### 3.3 Who controls the contract (mainnet, read on 2026-11-13)

- `view_access_key_list`: one **FullAccess** key,
  `ed25519:2ZH9CzFkmTVSKM9rQ14b8D4myiuFZv17jvdfZmYjTcak`. Its holder can deploy any
  code to the account directly, without the contract's upgrade rules.
- Contract upgrade path (`upgrade`, from near-sdk-contract-tools, with `on_upgrade` at
  `src/fiat_token.rs:753`): the caller must have the Admin role, must not be
  blocklisted, and `approved_for_upgrade` must be set by an executed `ApproveForUpgrade`
  multisig request (2 Admin approvals; threshold 2 is set in `init`).
- Roles: 4 admins, 4 owners, 4 pausers, 4 master minters (all 64-hex implicit
  accounts), 1 blocklister
  (`afed4c0a3a10daa967174eaf88a485c7ce018101b9620196c0d928e6021e0908`). Pause and
  unpause need a 2-of-n pauser multisig request. **Blocklisting needs one transaction
  from the blocklister** (`blocklist` at `src/fiat_token.rs:485`, no multisig).
- At that block USDC was not paused (`ft_balance_of` answered; it panics when paused),
  and `is_blocklisted("intents.near")` was `false`. `intents.near` held 3115877.018973
  USDC. 218 multisig requests had been created.

## 4. The checklist from fixes-1.md section 5

### 4.1 Source reading

`src/fiat_token.rs` (commit `80e2760`, same as `a5190cb`):

```rust
fn ft_transfer_call(&mut self, receiver_id, amount, memo, msg) -> PromiseOrValue<U128> {   // line 781
    require!(!self.paused, "FiatToken: paused");
    require_not_blocklisted(&env::predecessor_account_id());
    require_not_blocklisted(&receiver_id);
    self.token.ft_transfer_call(receiver_id, amount, memo, msg)
}

#[private]
fn ft_resolve_transfer(&mut self, sender_id, receiver_id, amount) -> U128 {              // line 808
    require!(!self.paused, "FiatToken: paused");
    require_not_blocklisted(&sender_id);
    require_not_blocklisted(&receiver_id);
    let (used_amount, burned_amount) =
        self.token.internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
    ...
    used_amount.into()
}
```

near-contract-standards 4.1.1, `fungible_token/core_impl.rs`:
`GAS_FOR_RESOLVE_TRANSFER = 5 Tgas`, `GAS_FOR_FT_TRANSFER_CALL = 30 Tgas`;
`ft_transfer_call` requires `prepaid_gas > 30 Tgas`, moves the balance, then calls
`ft_on_transfer` with static gas `prepaid − 30 Tgas` and `ft_resolve_transfer` with
static gas 5 Tgas.

| # | Item | Answer | Evidence |
|---|---|---|---|
| 1 | `ft_resolve_transfer` cannot panic after the receiver kept the tokens | **Fails.** It panics when paused, and when the sender or the receiver is blocklisted. Its other panics (`receiver doesn't have enough balance`, overflows) cannot happen in practice. | Source above; sandbox 4.2 and 4.3 |
| 2 | Fixed gas for the resolve step | Yes: 5 Tgas static (plus a share of unused gas). Measured use: 1.39 to 1.84 Tgas, including when it panics. Running out of gas is not the risk. | `core_impl.rs`; sandbox receipts |
| 3 | 60 Tgas is enough (H8) | Yes. `intents.near`'s `ft_on_transfer` gets at least 30 Tgas and used 3.52 Tgas. USDC's `ft_transfer_call` receipt used 2.89 Tgas. A whole withdraw burnt 13.77 Tgas. | Sandbox 4.2 |
| 4 | Result is the JSON string of the amount used | Yes: `"1000000000"` for 1000 USDC. | Sandbox 4.2 |
| 5 | Pause or blocklist between transfer and resolve cannot give `Failed` after a delivery | **Fails.** Pause, blocklist of the receiver, and blocklist of the escrow all do it. Blocklisting the taker has no effect on this call: the taker is only named in `msg`. In `ft_transfer_call` itself the checks run before any state change, so a restriction that is in place before the withdraw is handled correctly. | Sandbox 4.2 and 4.3 |
| 6 | Upgrades | The FullAccess key holder (any code, any time), and the Admin multisig path. Every answer above holds only for code hash `DVJx7F3d…`. | Section 3.3 |

Also: when paused, `ft_balance_of` and `ft_total_supply` panic. A fix must not depend on
reading the escrow's USDC balance during a pause.

### 4.2 Sandbox runs with the deployed wasm

Setup: near-workspaces 0.24, the repository's sandbox (`LD_PRELOAD=$ZECRET_SANDBOX_PRELOAD`),
the USDC wasm from `view_code`, initialised with
`init(admin_ids, master_minter_ids, owner_ids, pauser_ids, blocklister_id, metadata)`;
minting set up through the multisig (`ConfigureController` ×2,
`ConfigureMinterAllowance`, then `mint`). The escrow was built from the current
`contracts/near-escrow` source and deployed as its README says. A swap is opened with
USDC's `ft_transfer_call`, then `set_ready`, `commit_claim`, `claim`, which gives the
taker an Intents credit of 1000 USDC. A second swap of 1000 USDC stays open as "other
swaps' funds".

| Test | Receiver | Result |
|---|---|---|
| Normal withdraw | live `intents.near` code | Delivered 1000, restored 0, taker credited in Intents. |
| Paused before withdraw | live `intents.near` | USDC's `ft_transfer_call` panics `FiatToken: paused`; restored 1000; escrow still holds 2000. After unpause: paid once; escrow keeps 1000. |
| `intents.near` blocklisted before withdraw | live `intents.near` | Panics `… is blocklisted`; restored; after unblocklist paid once. |
| Blocklist of the receiver between receiver and resolve | test receiver (below) | Resolve panics `FiatToken: <receiver> is blocklisted`; on_withdraw restores 1000; receiver holds 1000. After unblocklist, a second withdraw pays again: **receiver holds 2000, escrow 0**, other swap still open. |
| Blocklist of the escrow between receiver and resolve | test receiver | Same: resolve panics, credit restored, paid twice, escrow 0. |
| Pause between receiver and resolve | test receiver | Same: resolve panics `FiatToken: paused`, paid twice, escrow 0. |

The test receiver stands in for `intents.near` so that the timing is exact: it is the
blocklister (or a pauser), and its `ft_on_transfer` first runs the blocklist (or the
`execute_multisig_request` of an approved Pause), and then returns `"0"`. USDC's resolve
step waits for that whole chain, so the restriction lands before it every time.

### 4.3 The race with the live `intents.near` code

A Pause request is created and approved by two pausers. `withdraw` is sent, and the
`execute_multisig_request` transaction is sent `d` blocks later. Receipts were read with
`EXPERIMENTAL_tx_status`, `wait_until: FINAL`.

Current escrow code:

| d | What happened | Taker in Intents after unpause and a second withdraw | Escrow USDC (another swap of 1000 open) |
|---|---|---|---|
| 0, 1 | `ft_transfer_call` panics `paused` before any transfer; credit restored | 1000 | 1000 |
| 2, 3 | USDC `ft_transfer` event; `intents.near` `mt_mint` to the taker; USDC resolve panics `FiatToken: paused`; escrow `on_withdraw` returns `"0"` and restores 1000 | **2000** | **0** |
| 4, 5 | Pause after the resolve step; normal | 1000 | 1000 |

After withdraw 1 at d = 2 and 3, the taker already held 1000 in Intents and still had a
credit of 1000.

## 5. Exact scenario on mainnet

1. A swap is claimed; the taker has an Intents credit `x` in the escrow. Other swaps in
   USDC are open, so the escrow holds more than `x`.
2. Anyone calls `withdraw(usdc, taker)`. The credit is set to 0. USDC moves `x` from the
   escrow to `intents.near`; `intents.near` credits `x` to the taker.
3. Before USDC's `ft_resolve_transfer` runs, Circle executes a Pause, or blocklists the
   escrow or `intents.near`.
4. The resolve step panics. The `ft_transfer_call` promise is `Failed`. `on_withdraw`
   restores `x`.
5. When the restriction is lifted, `withdraw` pays `x` again. The second `x` comes from
   the other swaps. The last maker or taker to withdraw finds the escrow short by `x`.

A related case that the current code does not handle either: if `intents.near`'s
`ft_on_transfer` fails and the resolve step also panics, the tokens stay in
`intents.near`'s USDC balance with nobody credited, and the escrow restores the credit.

## 6. Proposed change

### 6.1 Idea

Do not pay the taker by a call whose `Failed` result is ambiguous. Pay in two steps:

1. `ft_transfer_call(intents_account, x, msg = <escrow account>)`: deposit into the
   escrow's own Verifier account. Ignore the result.
2. `intents.mt_transfer(taker, "nep141:<token>", x)` from the escrow's Verifier balance.
   This is one receipt inside the Verifier. It succeeds only if the escrow's Verifier
   balance covers `x`.
3. `on_withdraw` restores the credit only if `mt_transfer` failed.

Why this is safe: the taker is paid only by a successful `mt_transfer`, and each credit
unit can reach exactly one `mt_transfer` (the credit is zeroed before step 1 and
restored only when its `mt_transfer` failed). Only the escrow can move tokens out of its
own Verifier account: it is a named account with no access keys, so it has no implicit
Verifier key, and as far as known only the account itself (as predecessor) can add a
key for it in the Verifier. That last point was not checked in the Verifier source. Other people can only add to that account, which never causes a double
payment. Rejected alternatives: reading the escrow's USDC balance after a failure (an
attacker can inflate it for a block with a failing `ft_transfer_call` to the escrow,
and `ft_balance_of` panics during a pause), and reading the taker's Intents balance (the
taker can lower it).

### 6.2 Patch (tested in a scratch copy of `contracts/near-escrow/src/lib.rs`)

```diff
@@ pub const GAS_ON_WITHDRAW: Gas = Gas::from_tgas(10);
+/// Gas reserved for `on_deposit`, which starts `mt_transfer` and `on_withdraw`.
+pub const GAS_ON_DEPOSIT: Gas = Gas::from_tgas(30);
+/// Gas reserved for `mt_transfer` on the Intents contract.
+pub const GAS_MT_TRANSFER: Gas = Gas::from_tgas(15);
@@ trait FungibleToken { ... }
+#[ext_contract(ext_mt)]
+#[allow(dead_code)]
+trait MultiToken {
+    fn mt_transfer(
+        &mut self,
+        receiver_id: AccountId,
+        token_id: String,
+        amount: U128,
+        approval: Option<(AccountId, u64)>,
+        memo: Option<String>,
+    );
+}
@@ pub fn withdraw(&mut self, token: AccountId, owner: AccountId) {
+                // Step 1: deposit into the escrow's own Intents account. Only the escrow
+                // can move tokens out of it. The result is not used (see on_deposit).
                 Route::Intents => ext_ft::ext(token.clone())
                     .with_attached_deposit(one_yocto)
                     .with_static_gas(GAS_FT_TRANSFER_CALL)
                     .ft_transfer_call(
                         self.intents_account.clone(),
                         U128(amount),
                         None,
-                        owner.to_string(),
+                        env::current_account_id().to_string(),
                     ),
             };
-            transfer
-                .then(
-                    Self::ext(env::current_account_id())
-                        .with_static_gas(GAS_ON_WITHDRAW)
-                        .with_unused_gas_weight(0)
-                        .on_withdraw(token.clone(), owner.clone(), route, U128(amount)),
-                )
-                .detach();
+            let next = match route {
+                Route::Direct => Self::ext(env::current_account_id())
+                    .with_static_gas(GAS_ON_WITHDRAW)
+                    .with_unused_gas_weight(0)
+                    .on_withdraw(token.clone(), owner.clone(), route, U128(amount)),
+                Route::Intents => Self::ext(env::current_account_id())
+                    .with_static_gas(GAS_ON_DEPOSIT)
+                    .with_unused_gas_weight(0)
+                    .on_deposit(token.clone(), owner.clone(), U128(amount)),
+            };
+            transfer.then(next).detach();
         }
     }
 
+    /// Step 2 of an Intents withdraw. The `ft_transfer_call` result is not used: a token
+    /// can report failure after it delivered (USDC on NEAR does when it is paused, or the
+    /// sender or receiver is blocklisted, before its resolve step). The taker is paid only
+    /// by `mt_transfer` from the escrow's own Intents balance, one atomic receipt.
+    #[private]
+    pub fn on_deposit(&mut self, token: AccountId, owner: AccountId, amount: U128) {
+        ext_mt::ext(self.intents_account.clone())
+            .with_attached_deposit(NearToken::from_yoctonear(1))
+            .with_static_gas(GAS_MT_TRANSFER)
+            .with_unused_gas_weight(0)
+            .mt_transfer(owner.clone(), format!("nep141:{token}"), amount, None, None)
+            .then(
+                Self::ext(env::current_account_id())
+                    .with_static_gas(GAS_ON_WITHDRAW)
+                    .with_unused_gas_weight(0)
+                    .on_withdraw(token, owner, Route::Intents, amount),
+            )
+            .detach();
+    }
+
-    /// Callback of `withdraw`. Restores the part of `amount` that was not delivered ...
-    /// - Intents: `ft_transfer_call` returns the amount the receiver kept. ...
+    /// Callback of the last step of a withdraw: `ft_transfer` (Direct) or `mt_transfer`
+    /// (Intents). Both are single receipts, so Failed means nothing moved: restore all.
     #[private]
     pub fn on_withdraw(...) -> U128 {
         let result = env::promise_result_checked(0, 64);
-        let delivered = match (route, result) {
-            (_, Err(PromiseError::Failed)) => 0,
-            (Route::Direct, _) => amount.0,
-            (Route::Intents, Ok(bytes)) => match parse_u128_json(&bytes) {
-                Some(used) => used.min(amount.0),
-                None => amount.0,
-            },
-            (Route::Intents, Err(_)) => amount.0,
-        };
+        let delivered = match result {
+            Err(PromiseError::Failed) => 0,
+            _ => amount.0,
+        };
```

Also needed (not implemented in the scratch copy):

- **`sweep(token)`**, public: read `intents.mt_balance_of(<escrow>, "nep141:<token>")`,
  then `intents.ft_withdraw(token, receiver_id = <escrow>, amount)`. It returns USDC
  that stays in the escrow's Verifier account. That happens when a deposit landed but
  `mt_transfer` failed (for example the Verifier was paused or the escrow's Verifier
  account was locked by its admins), or when the Verifier kept only part of a deposit.
  Nobody is paid twice in those cases, but without `sweep` the escrow's USDC alone
  would no longer cover its credits. A sweep that runs while a withdraw is between
  step 1 and step 2 only makes that withdraw's `mt_transfer` fail and its credit be
  restored, which is correct.
- `withdraw` needs more gas. The six existing sandbox tests passed with the 100 Tgas they
  attach; attach 150 Tgas to be safe and measure.
- README "Withdraw": the two steps, the new table, and that the taker's account still
  needs no NEAR (`mt_transfer` names it; it does not have to exist).
- DESIGN.md step "withdraw" (table at line 146) and its decisions log, in the same
  commit, per CLAUDE.md.
- `parse_u128_json` is no longer used by the contract.

Residual risk with the patch: the "related case" in section 5. The Verifier's deposit
fails and USDC's resolve step panics in the same window. Then the tokens stay
uncredited in `intents.near`'s USDC balance. The patch still pays nobody twice: the
taker is paid once (from a later deposit, or from stranded escrow Verifier funds). But
the escrow is short by `x` until the Verifier's admins return the tokens. This needs a
Circle restriction and a Verifier deposit failure at the same time.

### 6.3 Results with the patch (scratch copy)

- The race of section 4.3 with the patched escrow: at d = 2 and 3, USDC's resolve still
  panics `FiatToken: paused`, then `intents.near` `mt_transfer` to the taker succeeds,
  and `on_withdraw` reports 1000 delivered. Final state for every d in {0, 1, 2, 3, 4,
  5}: taker 1000 in Intents, escrow 1000 USDC (the other swap), escrow Verifier balance
  0, taker credit 0.
- At d = 0 and 1: the deposit fails before any transfer, `mt_transfer` fails (the escrow
  has no Verifier balance), the credit is restored, and after unpause it is paid once.
- Existing tests on the patched copy: all 6 sandbox tests in `tests/sandbox.rs` pass
  (including the happy path into the live `intents.near` code and both Intents failure
  cases of `failed_withdraw_restores_credit`). Unit tests: 48 pass, 2 fail as expected
  because they assert the old behaviour: `happy_path_commit_claim_withdraw` (checks
  `msg = owner` and the `on_withdraw` callback) and
  `on_withdraw_restores_unused_part_of_ft_transfer_call`.

### 6.4 Test plan for the patch

1. Unit: rewrite the two failing tests. `withdraw` creates `ft_transfer_call` with
   `msg` = escrow account and the `on_deposit` callback; `on_deposit` creates
   `mt_transfer(owner, "nep141:<token>", amount)` with 1 yoctoNEAR and the
   `on_withdraw` callback; `on_withdraw` on the Intents route restores all on `Failed`
   and nothing on success.
2. Sandbox, add `tests/usdc.rs` with the USDC wasm from mainnet (check its hash is
   `DVJx7F3d…` before the test uses it; store it next to the other live wasm files in
   `$ZECRET_WASM_DIR`):
   - normal withdraw into the live `intents.near` code: taker paid once; gas per
     receipt printed;
   - paused before withdraw, and `intents.near` blocklisted before withdraw: credit
     restored, paid once after the restriction is lifted;
   - the race of section 4.3 for d in 0..=5: assert, for every d, taker's Intents
     balance = credit, escrow USDC + escrow Verifier balance >= the open swap's amount,
     and no credit left;
   - blocklist of the escrow at the same point (one transaction by the blocklister,
     easier to time than a pause);
   - Verifier refuses `mt_transfer` (for example `force_lock_account` of the escrow by
     a Verifier admin in the sandbox): credit restored, deposit stays as escrow
     Verifier balance, `sweep` returns it, then one withdraw pays once.
3. Run the whole existing suite: unit, `tests/sandbox.rs`, and the core client tests
   that read the `withdrawn` event.

## 7. What remains unverified

- The build is not byte-identical to the deployed code (section 3.2). The behaviour was
  tested on the deployed bytes, so the conclusion does not rest on the source match.
- How often Circle pauses USDC on NEAR or blocklists accounts, and how long the
  receipt window is on mainnet under load. Not measured. The sandbox runs on one shard;
  on mainnet the escrow, USDC and `intents.near` may be on different shards, which can
  make the window longer.
- Who holds the FullAccess key on the USDC account. Not known.
- The patch was tested only in a scratch copy and only in the sandbox. `sweep` was not
  written. Mainnet `intents.near` behaviour for `mt_transfer` by a contract
  predecessor was tested only with the code in `$ZECRET_WASM_DIR`, not on mainnet. The
  Verifier's own admin powers (`force_lock_account`,
  `force_disable_auth_by_predecessor_ids`, `pa_pause_feature`) were not tested against
  the patch.
- The scratch test code (`tests/usdc.rs`, the test receiver contract) was deleted with
  the scratch directory. Section 4 describes it; the test plan above says what to add.
