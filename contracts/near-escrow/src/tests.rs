//! Unit tests. They port the intent of `contracts/test/CloakroomEscrow.t.sol`,
//! `CloakroomEscrow.audit.t.sol`, `AuditFindings.t.sol` and `PallasRealCurve.t.sol`.

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
use near_sdk::{testing_env, PromiseResult, RuntimeFeesConfig};
use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::group::{Group, GroupEncoding};
use pasta_curves::pallas;
use std::panic::{catch_unwind, AssertUnwindSafe};

// Vectors from contracts/test/CloakroomEscrow.t.sol (orchard 0.16.0), as big-endian uint256.
const A_SEC: &str = "195661e2e4789b7f57137b2e777736d79720270638d12e4882fd95e52a73d6b3";
const AX: &str = "17a182b1eb9594581ecaad0fcb61546f4c366cc57b9575ddde22ca56963c304c";
const AY: &str = "19b252972f768cc5708b251baca77a9afd02dddd9531510aafb13c4824d7b61f";
const B_SEC: &str = "2b8bbfabc504814096b330ffba3d828335e8db236f79aa7fded4d77640369e2a";
const BX: &str = "183a6db8e5f566dfd863f5c0c0b846ecfb01d519efbd2259d829319dfd780e95";
const BY: &str = "0b8b2c14d8675f49fedab2ee56d17c8547d6627cc8f83b2988e88fad65cb5972";
// Field modulus p and group order q of Pallas, big-endian.
const P_HEX: &str = "40000000000000000000000000000000224698fc094cf91b992d30ed00000001";
const Q_HEX: &str = "40000000000000000000000000000000224698fc0994a8dd8c46eb2100000001";

const NOW: u64 = 1_700_000_000;
const T0: u64 = NOW + 3600;
const T1: u64 = NOW + 3 * 3600;
const T2: u64 = NOW + 5 * 3600;
const W: u64 = 12 * 3600;
const AMT: u128 = 3_500_000_000;
const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}
fn escrow_id() -> AccountId {
    acc("escrow.near")
}
fn maker() -> AccountId {
    acc("maker.near")
}
fn token() -> AccountId {
    acc("usdc.near")
}
fn intents() -> AccountId {
    acc("intents.near")
}

/// Big-endian hex (as in Solidity) to 32 little-endian bytes.
fn le(be_hex: &str) -> [u8; 32] {
    let mut b = [0u8; 32];
    hex::decode_to_slice(be_hex, &mut b).unwrap();
    b.reverse();
    b
}

/// The Solidity affine (x, y) to the 32-byte compressed encoding used here.
fn compress(x_be: &str, y_be: &str) -> [u8; 32] {
    let mut out = le(x_be);
    let y = le(y_be);
    out[31] |= (y[0] & 1) << 7;
    out
}

/// Adds two 256-bit little-endian integers (no overflow in the cases used here).
fn add_le(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in 0..32 {
        let s = a[i] as u16 + b[i] as u16 + carry;
        out[i] = s as u8;
        carry = s >> 8;
    }
    assert_eq!(carry, 0);
    out
}

fn small(n: u8) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0] = n;
    b
}

fn a_pub() -> [u8; 32] {
    compress(AX, AY)
}
fn b_pub() -> [u8; 32] {
    compress(BX, BY)
}
fn taker_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}
fn taker_pk() -> [u8; 32] {
    taker_key().verifying_key().to_bytes()
}
fn taker_account() -> AccountId {
    taker_account_of(&taker_pk())
}

fn ctx(predecessor: &AccountId, t: u64) -> VMContextBuilder {
    let mut b = VMContextBuilder::new();
    b.current_account_id(escrow_id())
        .predecessor_account_id(predecessor.clone())
        .signer_account_id(predecessor.clone())
        .block_timestamp(t * 1_000_000_000)
        .prepaid_gas(Gas::from_tgas(300));
    b
}

fn at(predecessor: &AccountId, t: u64) {
    testing_env!(ctx(predecessor, t).build());
}

fn at_with_deposit(predecessor: &AccountId, t: u64, yocto: u128) {
    testing_env!(ctx(predecessor, t)
        .attached_deposit(NearToken::from_yoctonear(yocto))
        .build());
}

fn at_with_results(predecessor: &AccountId, t: u64, results: Vec<PromiseResult>) {
    testing_env!(
        ctx(predecessor, t).build(),
        near_sdk::test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        results
    );
}

/// Runs `f` and returns its panic message. Fails the test if `f` does not panic.
fn err<F: FnOnce()>(f: F) -> String {
    let r = catch_unwind(AssertUnwindSafe(f));
    match r {
        Ok(()) => panic!("expected a panic"),
        Err(e) => e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    }
}

fn assert_err<F: FnOnce()>(code: &str, f: F) {
    let msg = err(f);
    // The mock reports a contract panic as `GuestPanic { panic_msg: "<code>: ..." }`.
    assert!(msg.contains(code), "expected {code}, got: {msg}");
}

fn terms(salt: u8) -> serde_json::Value {
    json!({
        "salt": hex::encode([salt; 32]),
        "taker_pk": hex::encode(taker_pk()),
        "a": hex::encode(a_pub()),
        "b": hex::encode(b_pub()),
        "t0": T0,
        "t1": T1,
        "t2": T2,
        "reveal_window": W,
    })
}

/// A fresh escrow; the maker has 1 NEAR of storage funds.
fn setup() -> Escrow {
    // Start from empty contract storage; the mock keeps storage between calls.
    near_sdk::mock::with_mocked_blockchain(|b| drop(b.take_storage()));
    at(&maker(), NOW);
    let mut c = Escrow::new(intents());
    at_with_deposit(&maker(), NOW, ONE_NEAR);
    c.add_storage_funds(token(), None);
    c
}

fn open_with(c: &mut Escrow, msg: serde_json::Value, amount: u128) -> U128 {
    at(&token(), NOW);
    c.ft_on_transfer(maker(), U128(amount), msg.to_string())
}

fn open(c: &mut Escrow) -> [u8; 32] {
    assert_eq!(open_with(c, terms(1), AMT), U128(0));
    compute_swap_id(&maker(), &[1; 32])
}

fn state(c: &Escrow, id: &[u8; 32]) -> State {
    c.get_swap(Hex32(*id)).unwrap().state
}

fn ready(c: &mut Escrow, id: &[u8; 32]) {
    at(&maker(), NOW + 1);
    c.set_ready(Hex32(*id));
}

fn sign(key: &SigningKey, escrow: &AccountId, id: &[u8; 32], commitment: &[u8; 32]) -> Hex64 {
    Hex64(
        key.sign(&build_commit_message(escrow, id, commitment))
            .to_bytes(),
    )
}

fn commit_at(c: &mut Escrow, id: &[u8; 32], t: u64) {
    let cm = compute_commitment(id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), id, &cm);
    at(&acc("anyone.near"), t);
    c.commit_claim(Hex32(*id), Hex32(cm), sig);
}

fn credit(c: &Escrow, owner: &AccountId) -> CreditView {
    c.get_credit(token(), owner.clone())
}

fn events() -> Vec<serde_json::Value> {
    get_logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str(j).unwrap())
        .collect()
}

// ======================================================================== curve

#[test]
fn g_equals_reddsa_orchard_spendauth_basepoint() {
    // The client (node/src/keys.rs) takes G as the verification key of scalar 1 for
    // the RedPallas SpendAuth signature type (orchard wraps reddsa::orchard::SpendAuth).
    // Derive it the same way.
    let one = reddsa::SigningKey::<reddsa::orchard::SpendAuth>::from_bytes(
        &pallas::Scalar::ONE.to_repr(),
    )
    .unwrap();
    let vk: [u8; 32] = reddsa::VerificationKey::from(&one).into();
    assert_eq!(curve::g().to_bytes(), vk);
    assert_eq!(
        hex::encode(vk),
        "63c975b884721a8d0ca1707be30c7f0c5f445f3e7c188d3b06d6f128b32355b7"
    );
}

#[test]
fn g_matches_pallas_sol_coordinates_and_is_on_curve() {
    let x_hex = "375523b328f1d6063b8d187c3e5f445f0c7f0ce37b70a10c8d1a7284b875c963";
    let y_hex = "1ad0357fdf1a66db7b10bcfcfed624fbdfc914fec005bdd84ce33e817b0c3bc9";
    assert_eq!(curve::G_X, le(x_hex));
    assert_eq!(curve::G_Y, le(y_hex));
    let x = pallas::Base::from_repr(curve::G_X).unwrap();
    let y = pallas::Base::from_repr(curve::G_Y).unwrap();
    assert_eq!(y.square(), x.square() * x + pallas::Base::from(5u64));
    // Decompressing the compressed form gives the same point as the affine constructor.
    let decompressed = pallas::Point::from_bytes(&compress(x_hex, y_hex)).unwrap();
    assert_eq!(decompressed, curve::g());
}

#[test]
fn solidity_vectors_map_to_compressed_encoding() {
    // a·G = A and b·G = B with the encodings described in the README.
    assert!(curve::is_discrete_log_of(&le(A_SEC), &a_pub()));
    assert!(curve::is_discrete_log_of(&le(B_SEC), &b_pub()));
    assert!(!curve::is_discrete_log_of(&le(A_SEC), &b_pub()));
    assert!(curve::is_discrete_log_of(&small(1), &curve::g().to_bytes()));
    // q - 1 gives -G: same x, the other y parity.
    let mut q_minus_1 = le(Q_HEX);
    q_minus_1[0] -= 1;
    assert!(curve::is_discrete_log_of(
        &q_minus_1,
        &(-curve::g()).to_bytes()
    ));
    assert_eq!(
        (-curve::g()).to_bytes(),
        compress(
            "375523b328f1d6063b8d187c3e5f445f0c7f0ce37b70a10c8d1a7284b875c963",
            // p - GY
            "253fca8020e59924840ef3030129db04449f83fd09473b434c49f26b84f3c438"
        )
    );
}

#[test]
fn edge_scalars_rejected() {
    let q = le(Q_HEX);
    let mut q_plus_1 = q;
    q_plus_1[0] += 1;
    // q ends in ...00000001 (little-endian first byte 0x01), so q - 2 borrows.
    let q_minus_2 = le("40000000000000000000000000000000224698fc0994a8dd8c46eb20ffffffff");
    for bad in [
        [0u8; 32],
        q,
        q_plus_1,
        [0xff; 32],
        small(2),
        q_minus_2,
        small(1),
    ] {
        assert!(!curve::is_discrete_log_of(&bad, &a_pub()));
        assert!(!curve::is_discrete_log_of(&bad, &b_pub()));
    }
    assert!(curve::parse_scalar(&[0u8; 32]).is_none());
    assert!(curve::parse_scalar(&q).is_none());
}

#[test]
fn scalar_aliases_rejected() {
    // k and k + q name the same point; only the canonical scalar is accepted.
    let q = le(Q_HEX);
    assert!(!curve::is_discrete_log_of(
        &add_le(&small(1), &q),
        &curve::g().to_bytes()
    ));
    assert!(!curve::is_discrete_log_of(
        &add_le(&le(A_SEC), &q),
        &a_pub()
    ));
    assert!(!curve::is_discrete_log_of(
        &add_le(&le(B_SEC), &q),
        &b_pub()
    ));
}

#[test]
fn invalid_points_rejected() {
    // The identity encodes as 32 zero bytes.
    assert!(curve::parse_point(&[0u8; 32]).is_none());
    assert_eq!(pallas::Point::identity().to_bytes(), [0u8; 32]);
    // x = 0 with the sign bit set.
    let mut x0_odd = [0u8; 32];
    x0_odd[31] = 0x80;
    assert!(curve::parse_point(&x0_odd).is_none());
    // Non-canonical x: x = p, and x + p for the x of G.
    assert!(curve::parse_point(&le(P_HEX)).is_none());
    let g_alias = add_le(&curve::G_X, &le(P_HEX));
    assert!(g_alias[31] & 0x80 == 0);
    assert!(curve::parse_point(&g_alias).is_none());
    // An x with no point on the curve.
    let mut off = None;
    for i in 1u8..=50 {
        if curve::parse_point(&small(i)).is_none()
            && curve::parse_point(&{
                let mut b = small(i);
                b[31] |= 0x80;
                b
            })
            .is_none()
        {
            off = Some(small(i));
            break;
        }
    }
    let off = off.expect("some small x is not on the curve");
    assert!(curve::parse_point(&off).is_none());
    // Valid points decode, and each point has exactly one encoding.
    assert!(curve::parse_point(&a_pub()).is_some());
    let mut flipped = a_pub();
    flipped[31] ^= 0x80;
    let other = curve::parse_point(&flipped).unwrap();
    assert_eq!(other, -curve::parse_point(&a_pub()).unwrap());
}

// ======================================================================== open

#[test]
fn open_stores_swap_and_emits_opened() {
    let mut c = setup();
    let id = open(&mut c);
    let v = c.get_swap(Hex32(id)).unwrap();
    assert_eq!(v.maker, maker());
    assert_eq!(v.token, token());
    assert_eq!(v.amount, U128(AMT));
    assert_eq!(v.taker_pk, Hex32(taker_pk()));
    assert_eq!(v.taker_account.as_str(), hex::encode(taker_pk()));
    assert_eq!((v.t0, v.t1, v.t2, v.reveal_window), (T0, T1, T2, W));
    assert_eq!(v.state, State::Open);
    assert_eq!(
        (v.locked_at, v.commitment, v.a, v.b),
        (None, None, None, None)
    );
    let ev = events();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["standard"], EVENT_STANDARD);
    assert_eq!(ev[0]["event"], "opened");
    assert_eq!(ev[0]["data"][0]["id"], hex::encode(id));
    assert_eq!(ev[0]["data"][0]["amount"], AMT.to_string());
    // Storage was paid from the maker's storage funds.
    assert!(c.get_storage_funds(maker(), token()).0 < ONE_NEAR);
    assert_eq!(
        credit(&c, &maker()),
        CreditView {
            direct: U128(0),
            intents: U128(0)
        }
    );
}

#[test]
fn open_rejects_bad_params() {
    let mut c = setup();
    let cases: Vec<(serde_json::Value, u128)> = vec![
        (terms(1), 0),
        (
            {
                let mut t = terms(1);
                t["t0"] = json!(NOW);
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["t0"] = json!(NOW - 1);
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["t1"] = json!(T0);
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["t2"] = json!(T1);
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["reveal_window"] = json!(0);
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["taker_pk"] = json!(hex::encode([0u8; 32]));
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["a"] = json!(hex::encode([0u8; 32]));
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["b"] = json!(hex::encode(le(P_HEX)));
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["a"] = json!("00");
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t["relayer_fee"] = json!("1");
                t
            },
            AMT,
        ),
        (
            {
                let mut t = terms(1);
                t.as_object_mut().unwrap().remove("salt");
                t
            },
            AMT,
        ),
        (json!("not terms"), AMT),
    ];
    for (msg, amount) in cases {
        let m = msg.clone();
        assert_err("BadParams", || {
            open_with(&mut c, m, amount);
        });
    }
    // Nothing was stored.
    assert!(c
        .get_swap(Hex32(compute_swap_id(&maker(), &[1; 32])))
        .is_none());
}

#[test]
fn open_rejects_point_not_on_curve() {
    let mut c = setup();
    let off = (1u8..=50)
        .map(small)
        .find(|b| curve::parse_point(b).is_none())
        .unwrap();
    let mut t = terms(1);
    t["a"] = json!(hex::encode(off));
    assert_err("BadParams", || {
        open_with(&mut c, t, AMT);
    });
}

#[test]
fn open_rejects_duplicate_swap_id() {
    let mut c = setup();
    open(&mut c);
    assert_err("BadState", || {
        open_with(&mut c, terms(1), AMT);
    });
}

#[test]
fn swap_ids_are_maker_scoped() {
    let mut c = setup();
    let id = open(&mut c);
    // Another maker with the same salt gets a different id and cannot touch the first swap.
    let other = acc("attacker.near");
    at_with_deposit(&other, NOW, ONE_NEAR);
    c.add_storage_funds(token(), None);
    at(&token(), NOW);
    c.ft_on_transfer(other.clone(), U128(1), terms(1).to_string());
    let id2 = compute_swap_id(&other, &[1; 32]);
    assert_ne!(id, id2);
    assert_eq!(c.get_swap(Hex32(id)).unwrap().maker, maker());
    assert_eq!(c.get_swap(Hex32(id2)).unwrap().maker, other);
    assert_eq!(c.swap_id(maker(), Hex32([1; 32])), Hex32(id));
    // The length prefix keeps (maker, salt) pairs apart.
    assert_ne!(
        compute_swap_id(&acc("ab.near"), &[1; 32]),
        compute_swap_id(&acc("a.near"), &[1; 32])
    );
}

/// `ft_on_transfer` trusts the calling token for `sender_id`. A fake token that names the
/// maker cannot spend the maker's storage funds for the real token. This alone does not
/// stop a fake token from taking the maker's swap id: a third party could fund
/// (maker, fake token) for it. That is review-1 F-01, and it is stopped only because
/// `add_storage_funds` adds funds for the caller alone (see
/// `poc_f01_third_party_cannot_take_the_makers_swap_id` and
/// `add_storage_funds_only_for_the_caller`). (On chain the failed call is reverted; the
/// mock does not revert, so this test only checks the panic and the funds.)
#[test]
fn fake_token_cannot_spend_maker_storage_funds() {
    let mut c = setup();
    let funds = c.get_storage_funds(maker(), token());
    at(&acc("fake-token.near"), NOW);
    assert_err("InsufficientStorage", || {
        c.ft_on_transfer(maker(), U128(AMT), terms(1).to_string());
    });
    assert_eq!(c.get_storage_funds(maker(), token()), funds);
}

#[test]
#[should_panic(expected = "InsufficientStorage")]
fn open_needs_maker_storage_funds() {
    at(&maker(), NOW);
    let mut c = Escrow::new(intents());
    open_with(&mut c, terms(1), AMT);
}

#[test]
fn storage_funds_deposit_and_withdraw() {
    let mut c = setup();
    let before = c.get_storage_funds(maker(), token()).0;
    assert!(before > ONE_NEAR - ONE_NEAR / 100);
    open(&mut c);
    let after = c.get_storage_funds(maker(), token()).0;
    let per_swap = before - after;
    // One swap with two new credit entries costs well under 0.01 NEAR.
    assert!(
        per_swap > 0 && per_swap < ONE_NEAR / 100,
        "per swap {per_swap}"
    );
    assert_err("Requires attached deposit", || {
        at(&maker(), NOW);
        c.withdraw_storage_funds(token(), None);
    });
    at_with_deposit(&maker(), NOW, 1);
    assert_err("InsufficientStorage", || {
        c.withdraw_storage_funds(token(), Some(U128(after + 1)));
    });
    at_with_deposit(&maker(), NOW, 1);
    assert_eq!(
        c.withdraw_storage_funds(token(), Some(U128(10))),
        U128(after - 10)
    );
    let r = get_created_receipts();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].receiver_id, maker());
    at_with_deposit(&maker(), NOW, 1);
    assert_eq!(c.withdraw_storage_funds(token(), None), U128(0));
}

// ======================================================================== set_ready

#[test]
fn set_ready_guards() {
    let mut c = setup();
    let id = open(&mut c);
    at(&acc("taker.near"), NOW + 1);
    assert_err("NotMaker", || c.set_ready(Hex32(id)));
    at(&maker(), T0);
    assert_err("TooLate", || c.set_ready(Hex32(id)));
    assert_eq!(state(&c, &id), State::Open);
    at(&maker(), T0 - 1);
    c.set_ready(Hex32(id));
    assert_eq!(state(&c, &id), State::Ready);
    assert_eq!(events()[0]["event"], "ready");
    assert_err("BadState", || c.set_ready(Hex32(id)));
    assert_err("BadState", || c.set_ready(Hex32([9; 32])));
}

// ======================================================================== happy path

#[test]
fn happy_path_commit_claim_withdraw() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    commit_at(&mut c, &id, T1 - 1);
    let v = c.get_swap(Hex32(id)).unwrap();
    assert_eq!(v.state, State::Locked);
    assert_eq!(v.locked_at, Some(T1 - 1));
    assert_eq!(
        v.commitment,
        Some(Hex32(compute_commitment(&id, &le(A_SEC))))
    );
    assert_eq!(events()[0]["event"], "locked");

    // Refund is closed while the reveal window is open.
    at(&maker(), T1 - 1 + W - 1);
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));

    at(&acc("anyone.near"), T1 - 1 + W - 1);
    c.claim(Hex32(id), Hex32(le(A_SEC)));
    let v = c.get_swap(Hex32(id)).unwrap();
    assert_eq!(v.state, State::Claimed);
    assert_eq!(v.a, Some(Hex32(le(A_SEC))));
    assert_eq!(v.b, None);
    let ev = events();
    assert_eq!(ev[0]["event"], "claimed");
    assert_eq!(ev[0]["data"][0]["a"], hex::encode(le(A_SEC)));
    assert_eq!(credit(&c, &taker_account()).intents, U128(AMT));
    assert_eq!(credit(&c, &taker_account()).direct, U128(0));
    assert_eq!(credit(&c, &maker()).direct, U128(0));

    // Exactly one exit.
    assert_err("BadState", || c.claim(Hex32(id), Hex32(le(A_SEC))));
    at(&maker(), T2 + 2 * W);
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));

    // Withdraw sends the taker's credit into Intents and zeroes it first.
    at(&acc("anyone.near"), T2 + 2 * W);
    c.withdraw(token(), taker_account());
    assert_eq!(credit(&c, &taker_account()).intents, U128(0));
    let r = get_created_receipts();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].receiver_id, token());
    assert_eq!(r[1].receiver_id, escrow_id());
    let expected_args = json!({
        "receiver_id": intents(),
        "amount": AMT.to_string(),
        "memo": null,
        "msg": taker_account(),
    })
    .to_string();
    match &r[0].actions[0] {
        near_sdk::mock::MockAction::FunctionCallWeight {
            method_name,
            args,
            attached_deposit,
            prepaid_gas,
            ..
        } => {
            assert_eq!(method_name, b"ft_transfer_call");
            assert_eq!(String::from_utf8(args.clone()).unwrap(), expected_args);
            assert_eq!(*attached_deposit, NearToken::from_yoctonear(1));
            assert_eq!(*prepaid_gas, GAS_FT_TRANSFER_CALL);
        }
        other => panic!("unexpected action {other:?}"),
    }
    match &r[1].actions[0] {
        near_sdk::mock::MockAction::FunctionCallWeight {
            method_name,
            prepaid_gas,
            gas_weight,
            ..
        } => {
            assert_eq!(method_name, b"on_withdraw");
            assert_eq!(*prepaid_gas, GAS_ON_WITHDRAW);
            assert_eq!(gas_weight.0, 0);
        }
        other => panic!("unexpected action {other:?}"),
    }
    // A second withdraw has nothing to pay.
    at(&acc("anyone.near"), T2 + 2 * W);
    c.withdraw(token(), taker_account());
    assert!(get_created_receipts().is_empty());
}

#[test]
fn no_claim_without_ready_even_after_t0() {
    let mut c = setup();
    let id = open(&mut c);
    let cm = compute_commitment(&id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), T0 + 1);
    assert_err("NotClaimable", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    assert_err("BadState", || c.claim(Hex32(id), Hex32(le(A_SEC))));
    // From t1 the maker refunds.
    at(&maker(), T1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    assert_eq!(state(&c, &id), State::Refunded);
    assert_eq!(credit(&c, &maker()).direct, U128(AMT));
}

#[test]
fn late_commit_rejected_and_refund_follows() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    let cm = compute_commitment(&id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), T1);
    assert_err("NotClaimable", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    at(&maker(), T1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    let v = c.get_swap(Hex32(id)).unwrap();
    assert_eq!(v.state, State::Refunded);
    assert_eq!(v.b, Some(Hex32(le(B_SEC))));
    assert_eq!(v.a, None);
    assert_eq!(events()[0]["event"], "refunded");
}

#[test]
fn locked_without_reveal_refunds_after_window() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    let locked_at = NOW + 10;
    commit_at(&mut c, &id, locked_at);
    at(&maker(), locked_at + W - 1);
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
    at(&acc("anyone.near"), locked_at + W);
    assert_err("TooLate", || c.claim(Hex32(id), Hex32(le(A_SEC))));
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    assert_eq!(state(&c, &id), State::Refunded);
    assert_eq!(credit(&c, &maker()).direct, U128(AMT));
    assert_eq!(credit(&c, &taker_account()).intents, U128(0));
}

// ======================================================================== refund windows

#[test]
fn refund_window_open_before_t0() {
    let mut c = setup();
    let id = open(&mut c);
    at(&acc("anyone.near"), T0 - 1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    assert_eq!(state(&c, &id), State::Refunded);
    assert_eq!(credit(&c, &maker()).direct, U128(AMT));
}

#[test]
fn refund_window_open_or_ready_in_t1_t2() {
    for set_ready in [false, true] {
        let mut c = setup();
        let id = open(&mut c);
        if set_ready {
            ready(&mut c, &id);
        }
        // Closed in [t0, t1) (and for Ready also before t0).
        for t in [T0, T1 - 1] {
            at(&maker(), t);
            assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
        }
        if set_ready {
            at(&maker(), NOW + 2);
            assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
        }
        // Closed from t2.
        at(&maker(), T2);
        assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
        // Open in [t1, t2).
        at(&maker(), T2 - 1);
        c.refund(Hex32(id), Hex32(le(B_SEC)));
        assert_eq!(state(&c, &id), State::Refunded);
    }
}

#[test]
fn refund_window_locked_from_locked_at_plus_w() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    commit_at(&mut c, &id, NOW + 5);
    at(&maker(), NOW + 5 + W - 1);
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
    at(&maker(), NOW + 5 + W);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    assert_eq!(state(&c, &id), State::Refunded);
}

#[test]
fn wrong_share_at_refund_door() {
    let mut c = setup();
    let id = open(&mut c);
    at(&maker(), NOW + 1);
    let q = le(Q_HEX);
    for bad in [le(A_SEC), [0u8; 32], add_le(&le(B_SEC), &q), q, small(1)] {
        assert_err("BadSecret", || c.refund(Hex32(id), Hex32(bad)));
    }
    assert_eq!(state(&c, &id), State::Open);
    assert_eq!(credit(&c, &maker()).direct, U128(0));
}

#[test]
fn refund_after_exit_or_unknown_rejected() {
    let mut c = setup();
    let id = open(&mut c);
    at(&maker(), NOW + 1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
    assert_eq!(credit(&c, &maker()).direct, U128(AMT));
    assert_err("BadState", || c.refund(Hex32([5; 32]), Hex32(le(B_SEC))));
    // After a refund the taker cannot lock or claim.
    let cm = compute_commitment(&id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), T2);
    assert_err("BadState", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    assert_err("BadState", || c.claim(Hex32(id), Hex32(le(A_SEC))));
}

// ======================================================================== commit_claim

#[test]
fn commit_needs_taker_signature() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    let cm = compute_commitment(&id, &le(A_SEC));
    let other_id = compute_swap_id(&maker(), &[2; 32]);
    let other_key = SigningKey::from_bytes(&[8u8; 32]);
    let bad = [
        // Wrong key.
        sign(&other_key, &escrow_id(), &id, &cm),
        // Signature for another swap.
        sign(&taker_key(), &escrow_id(), &other_id, &cm),
        // Signature for another escrow account.
        sign(&taker_key(), &acc("other-escrow.near"), &id, &cm),
        // Signature over another commitment.
        sign(&taker_key(), &escrow_id(), &id, &[3; 32]),
        // Garbage.
        Hex64([0u8; 64]),
    ];
    at(&acc("anyone.near"), NOW + 2);
    for sig in bad {
        assert_err("BadSignature", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    }
    // A signature over the raw message by the right key, but not this message format.
    let raw = Hex64(
        taker_key()
            .sign(&[id.as_slice(), cm.as_slice()].concat())
            .to_bytes(),
    );
    assert_err("BadSignature", || c.commit_claim(Hex32(id), Hex32(cm), raw));
    assert_eq!(state(&c, &id), State::Ready);
    let good = sign(&taker_key(), &escrow_id(), &id, &cm);
    c.commit_claim(Hex32(id), Hex32(cm), good);
    assert_eq!(state(&c, &id), State::Locked);
    // Locked once only.
    assert_err("BadState", || c.commit_claim(Hex32(id), Hex32(cm), good));
}

#[test]
fn commit_claim_unknown_swap() {
    let mut c = setup();
    at(&acc("anyone.near"), NOW);
    assert_err("BadState", || {
        c.commit_claim(Hex32([4; 32]), Hex32([0; 32]), Hex64([0; 64]))
    });
}

#[test]
fn commit_message_view_matches_signed_bytes() {
    let mut c = setup();
    let id = open(&mut c);
    let cm = compute_commitment(&id, &le(A_SEC));
    at(&acc("anyone.near"), NOW);
    let msg = c.commit_message(Hex32(id), Hex32(cm)).0;
    let mut expected = b"near-escrow/v1/commit-claim".to_vec();
    expected.push(11);
    expected.extend_from_slice(b"escrow.near");
    expected.extend_from_slice(&id);
    expected.extend_from_slice(&cm);
    assert_eq!(msg, expected);
    let mut pre = b"near-escrow/v1/commitment".to_vec();
    pre.extend_from_slice(&id);
    pre.extend_from_slice(&le(A_SEC));
    assert_eq!(
        c.commitment_of(Hex32(id), Hex32(le(A_SEC))).0,
        env::sha256_array(&pre)
    );
    let mut pre = b"near-escrow/v1/swap-id".to_vec();
    pre.push(10);
    pre.extend_from_slice(b"maker.near");
    pre.extend_from_slice(&[1; 32]);
    assert_eq!(id, env::sha256_array(&pre));
}

// ======================================================================== claim

#[test]
fn reveal_must_match_commitment() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    commit_at(&mut c, &id, NOW + 2);
    at(&acc("anyone.near"), NOW + 3);
    assert_err("BadSecret", || c.claim(Hex32(id), Hex32(le(B_SEC))));
    let mut a_off = le(A_SEC);
    a_off[0] ^= 1;
    assert_err("BadSecret", || c.claim(Hex32(id), Hex32(a_off)));
    assert_eq!(state(&c, &id), State::Locked);
    c.claim(Hex32(id), Hex32(le(A_SEC)));
    assert_eq!(state(&c, &id), State::Claimed);
}

/// The commitment opens, but the opened value is not a valid share for A.
fn claim_with_committed(value: [u8; 32]) {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    let cm = compute_commitment(&id, &value);
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), NOW + 2);
    c.commit_claim(Hex32(id), Hex32(cm), sig);
    assert_err("BadSecret", || c.claim(Hex32(id), Hex32(value)));
    assert_eq!(state(&c, &id), State::Locked);
    assert_eq!(credit(&c, &taker_account()).intents, U128(0));
}

#[test]
fn claim_rejects_non_canonical_scalar() {
    claim_with_committed(add_le(&le(A_SEC), &le(Q_HEX)));
}

#[test]
fn claim_rejects_zero_scalar() {
    claim_with_committed([0u8; 32]);
}

#[test]
fn claim_rejects_wrong_scalar_with_matching_commitment() {
    claim_with_committed(le(B_SEC));
}

#[test]
fn claim_needs_locked_state() {
    let mut c = setup();
    let id = open(&mut c);
    at(&acc("anyone.near"), NOW + 1);
    assert_err("BadState", || c.claim(Hex32(id), Hex32(le(A_SEC))));
    ready(&mut c, &id);
    assert_err("BadState", || c.claim(Hex32(id), Hex32(le(A_SEC))));
    assert_err("BadState", || c.claim(Hex32([6; 32]), Hex32(le(A_SEC))));
}

// ======================================================================== punish path

#[test]
fn punish_path_from_open_after_t2() {
    let mut c = setup();
    let id = open(&mut c);
    // Not before t2.
    for t in [NOW + 1, T0, T1, T2 - 1] {
        let cm = compute_commitment(&id, &le(A_SEC));
        let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
        at(&acc("anyone.near"), t);
        assert_err("NotClaimable", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    }
    commit_at(&mut c, &id, T2);
    assert_eq!(state(&c, &id), State::Locked);
    at(&acc("anyone.near"), T2 + W - 1);
    c.claim(Hex32(id), Hex32(le(A_SEC)));
    assert_eq!(state(&c, &id), State::Claimed);
    assert_eq!(credit(&c, &taker_account()).intents, U128(AMT));
}

#[test]
fn punish_path_from_ready_after_t2() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    commit_at(&mut c, &id, T2 + 1);
    at(&acc("anyone.near"), T2 + 1);
    c.claim(Hex32(id), Hex32(le(A_SEC)));
    assert_eq!(state(&c, &id), State::Claimed);
}

#[test]
fn gap_between_t1_and_t2() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    let cm = compute_commitment(&id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), T1 + 1);
    assert_err("NotClaimable", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    at(&maker(), T2);
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
}

#[test]
fn maker_refund_in_window_blocks_punish() {
    let mut c = setup();
    let id = open(&mut c);
    at(&maker(), T2 - 1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    let cm = compute_commitment(&id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), T2);
    assert_err("BadState", || c.commit_claim(Hex32(id), Hex32(cm), sig));
}

/// Port of test_info_lateCommitSignatureReplayableAtT2: a late commit fails, but its
/// signature is public and anyone can replay it from t2. The lock only helps the taker.
#[test]
fn late_commit_signature_replayable_at_t2() {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    let cm = compute_commitment(&id, &le(A_SEC));
    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
    at(&acc("anyone.near"), T1 + 1);
    assert_err("NotClaimable", || c.commit_claim(Hex32(id), Hex32(cm), sig));
    at(&acc("dead.near"), T2);
    c.commit_claim(Hex32(id), Hex32(cm), sig);
    assert_eq!(state(&c, &id), State::Locked);
    assert_err("TooEarly", || c.refund(Hex32(id), Hex32(le(B_SEC))));
    at(&maker(), T2 + W);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    assert_eq!(state(&c, &id), State::Refunded);
}

/// Port of testFuzz_exactlyOneExit, run over a fixed grid of times instead of a fuzzer:
/// for each path and each time, at most one exit happens and the credits never exceed
/// the amount.
#[test]
fn exactly_one_exit_over_time_grid() {
    let times = [
        NOW + 1,
        T0 - 1,
        T0,
        T1 - 1,
        T1,
        T2 - 1,
        T2,
        T2 + W - 1,
        T2 + W,
        T2 + 2 * W,
    ];
    for path in 0..4u8 {
        for &t1x in &times {
            for &t2x in &times {
                if t2x < t1x {
                    continue;
                }
                let mut c = setup();
                let id = open(&mut c);
                if path & 1 == 1 {
                    at(&maker(), t1x);
                    let _ = err_or_ok(|| c.set_ready(Hex32(id)));
                }
                if path & 2 == 2 {
                    let cm = compute_commitment(&id, &le(A_SEC));
                    let sig = sign(&taker_key(), &escrow_id(), &id, &cm);
                    at(&acc("anyone.near"), t1x);
                    let _ = err_or_ok(|| c.commit_claim(Hex32(id), Hex32(cm), sig));
                }
                at(&acc("anyone.near"), t2x);
                let claimed = err_or_ok(|| c.claim(Hex32(id), Hex32(le(A_SEC))));
                let refunded = err_or_ok(|| c.refund(Hex32(id), Hex32(le(B_SEC))));
                assert!(!(claimed && refunded));
                let total = credit(&c, &maker()).direct.0 + credit(&c, &taker_account()).intents.0;
                assert!(total <= AMT);
                let st = state(&c, &id);
                assert_eq!(claimed, st == State::Claimed);
                assert_eq!(refunded, st == State::Refunded);
                if claimed || refunded {
                    assert_eq!(total, AMT);
                }
            }
        }
    }
}

fn err_or_ok<F: FnOnce()>(f: F) -> bool {
    let r = catch_unwind(AssertUnwindSafe(f));
    r.is_ok()
}

// ======================================================================== withdraw

/// A swap refunded to the maker, so the maker has a direct credit of AMT.
fn refunded() -> Escrow {
    let mut c = setup();
    let id = open(&mut c);
    at(&maker(), NOW + 1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    c
}

/// A swap claimed by the taker, so the taker has an Intents credit of AMT.
fn claimed() -> Escrow {
    let mut c = setup();
    let id = open(&mut c);
    ready(&mut c, &id);
    commit_at(&mut c, &id, NOW + 2);
    at(&acc("anyone.near"), NOW + 3);
    c.claim(Hex32(id), Hex32(le(A_SEC)));
    c
}

#[test]
fn withdraw_maker_uses_ft_transfer_and_zeroes_first() {
    let mut c = refunded();
    at(&acc("anyone.near"), NOW + 5);
    c.withdraw(token(), maker());
    assert_eq!(credit(&c, &maker()).direct, U128(0));
    let r = get_created_receipts();
    assert_eq!(r.len(), 2);
    match &r[0].actions[0] {
        near_sdk::mock::MockAction::FunctionCallWeight {
            method_name,
            args,
            attached_deposit,
            ..
        } => {
            assert_eq!(method_name, b"ft_transfer");
            let a: serde_json::Value = serde_json::from_slice(args).unwrap();
            assert_eq!(a["receiver_id"], "maker.near");
            assert_eq!(a["amount"], AMT.to_string());
            assert_eq!(*attached_deposit, NearToken::from_yoctonear(1));
        }
        other => panic!("unexpected action {other:?}"),
    }
}

#[test]
fn withdraw_with_no_credit_does_nothing() {
    let mut c = setup();
    at(&acc("anyone.near"), NOW);
    c.withdraw(token(), maker());
    assert!(get_created_receipts().is_empty());
    assert!(events().is_empty());
}

#[test]
fn on_withdraw_failed_transfer_restores_credit() {
    for (mut c, owner, route) in [
        (refunded(), maker(), Route::Direct),
        (claimed(), taker_account(), Route::Intents),
    ] {
        at(&acc("anyone.near"), NOW + 5);
        c.withdraw(token(), owner.clone());
        let zero = CreditView {
            direct: U128(0),
            intents: U128(0),
        };
        assert_eq!(credit(&c, &owner), zero);
        at_with_results(&escrow_id(), NOW + 6, vec![PromiseResult::Failed]);
        assert_eq!(
            c.on_withdraw(token(), owner.clone(), route, U128(AMT)),
            U128(0)
        );
        let cr = credit(&c, &owner);
        assert_eq!(cr.direct.0 + cr.intents.0, AMT);
        let ev = events();
        assert_eq!(ev[0]["event"], "withdrawn");
        assert_eq!(ev[0]["data"][0]["amount"], "0");
        assert_eq!(ev[0]["data"][0]["restored"], AMT.to_string());
    }
}

#[test]
fn on_withdraw_success_keeps_credit_zero() {
    let mut c = refunded();
    at(&acc("anyone.near"), NOW + 5);
    c.withdraw(token(), maker());
    at_with_results(
        &escrow_id(),
        NOW + 6,
        vec![PromiseResult::Successful(vec![])],
    );
    assert_eq!(
        c.on_withdraw(token(), maker(), Route::Direct, U128(AMT)),
        U128(AMT)
    );
    assert_eq!(credit(&c, &maker()).direct, U128(0));

    let mut c = claimed();
    at(&acc("anyone.near"), NOW + 5);
    c.withdraw(token(), taker_account());
    let used = format!("\"{AMT}\"").into_bytes();
    at_with_results(&escrow_id(), NOW + 6, vec![PromiseResult::Successful(used)]);
    assert_eq!(
        c.on_withdraw(token(), taker_account(), Route::Intents, U128(AMT)),
        U128(AMT)
    );
    assert_eq!(credit(&c, &taker_account()).intents, U128(0));
}

#[test]
fn on_withdraw_restores_unused_part_of_ft_transfer_call() {
    let mut c = claimed();
    at(&acc("anyone.near"), NOW + 5);
    c.withdraw(token(), taker_account());
    let used = b"\"1000\"".to_vec();
    at_with_results(&escrow_id(), NOW + 6, vec![PromiseResult::Successful(used)]);
    assert_eq!(
        c.on_withdraw(token(), taker_account(), Route::Intents, U128(AMT)),
        U128(1000)
    );
    assert_eq!(credit(&c, &taker_account()).intents, U128(AMT - 1000));
}

#[test]
fn on_withdraw_never_restores_more_than_sent() {
    // The token reports more used than sent: clamp, restore nothing.
    let mut c = claimed();
    at(&acc("anyone.near"), NOW + 5);
    c.withdraw(token(), taker_account());
    let used = format!("\"{}\"", AMT * 2).into_bytes();
    at_with_results(&escrow_id(), NOW + 6, vec![PromiseResult::Successful(used)]);
    assert_eq!(
        c.on_withdraw(token(), taker_account(), Route::Intents, U128(AMT)),
        U128(AMT)
    );
    assert_eq!(credit(&c, &taker_account()).intents, U128(0));
    // A success value that is not a U128: count it as delivered, so nothing is paid twice.
    for junk in [
        b"".to_vec(),
        b"12".to_vec(),
        b"\"x\"".to_vec(),
        vec![b'"'; 100],
    ] {
        let mut c = claimed();
        at(&acc("anyone.near"), NOW + 5);
        c.withdraw(token(), taker_account());
        at_with_results(&escrow_id(), NOW + 6, vec![PromiseResult::Successful(junk)]);
        assert_eq!(
            c.on_withdraw(token(), taker_account(), Route::Intents, U128(AMT)),
            U128(AMT)
        );
        assert_eq!(credit(&c, &taker_account()).intents, U128(0));
    }
}

#[test]
fn on_withdraw_restore_adds_to_newer_credit() {
    // A refund lands between withdraw and its callback; the restore must add, not set.
    let mut c = setup();
    let id1 = open(&mut c);
    at(&maker(), NOW + 1);
    c.refund(Hex32(id1), Hex32(le(B_SEC)));
    at(&acc("anyone.near"), NOW + 2);
    c.withdraw(token(), maker());
    let mut t = terms(2);
    t["t0"] = json!(T0);
    open_with(&mut c, t, 7);
    let id2 = compute_swap_id(&maker(), &[2; 32]);
    at(&maker(), NOW + 3);
    c.refund(Hex32(id2), Hex32(le(B_SEC)));
    assert_eq!(credit(&c, &maker()).direct, U128(7));
    at_with_results(&escrow_id(), NOW + 4, vec![PromiseResult::Failed]);
    c.on_withdraw(token(), maker(), Route::Direct, U128(AMT));
    assert_eq!(credit(&c, &maker()).direct, U128(AMT + 7));
}

#[test]
fn withdraw_pays_both_routes_for_same_owner() {
    // An owner that is both a maker (direct credit) and a taker (Intents credit).
    let mut c = setup();
    let id = open(&mut c);
    at(&maker(), NOW + 1);
    c.refund(Hex32(id), Hex32(le(B_SEC)));
    // Open a second swap whose taker account is the maker's account name. Only possible
    // when the maker is an implicit account; simulate it by keying the credit directly.
    let key = CreditKey {
        token: token(),
        owner: maker(),
        route: Route::Intents,
    };
    c.add_credit(key, 5);
    at(&acc("anyone.near"), NOW + 2);
    c.withdraw(token(), maker());
    assert_eq!(
        credit(&c, &maker()),
        CreditView {
            direct: U128(0),
            intents: U128(0)
        }
    );
    let r = get_created_receipts();
    assert_eq!(r.len(), 4);
}

#[test]
fn intents_account_is_configurable() {
    at(&maker(), NOW);
    let c = Escrow::new(acc("intents.testnet"));
    assert_eq!(c.get_intents_account(), acc("intents.testnet"));
}

#[test]
fn hex_inputs_parse_strictly() {
    let ok: Hex32 = serde_json::from_str(&format!("\"{}\"", "AB".repeat(32))).unwrap();
    assert_eq!(ok.0, [0xab; 32]);
    assert!(serde_json::from_str::<Hex32>(&format!("\"{}\"", "ab".repeat(31))).is_err());
    assert!(serde_json::from_str::<Hex32>(&format!("\"0x{}\"", "ab".repeat(31))).is_err());
    assert!(serde_json::from_str::<Hex64>(&format!("\"{}\"", "ab".repeat(32))).is_err());
    assert_eq!(
        serde_json::to_string(&Hex32([1; 32])).unwrap(),
        format!("\"{}\"", "01".repeat(32))
    );
}

// ================================================================ review-1 F-01
//
// Proof of concept from docs/security/poc/f01_swap_id_squat.rs, kept as a regression test.

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

/// F-01: `add_storage_funds` adds funds only for the caller. Naming another account is
/// refused; naming the caller itself is the same as naming no account.
#[test]
fn add_storage_funds_only_for_the_caller() {
    let mut c = setup();
    let attacker = acc("attacker.near");
    let fake = acc("fake-token.near");
    at_with_deposit(&attacker, NOW, ONE_NEAR / 10);
    assert_err("BadParams", || {
        c.add_storage_funds(fake.clone(), Some(maker()));
    });
    at_with_deposit(&attacker, NOW, ONE_NEAR / 10);
    assert_err("BadParams", || {
        c.add_storage_funds(token(), Some(maker()));
    });
    at_with_deposit(&attacker, NOW, ONE_NEAR / 10);
    let own = c.add_storage_funds(fake.clone(), Some(attacker.clone()));
    assert!(own.0 > 0);
    assert_eq!(c.get_storage_funds(attacker.clone(), fake.clone()), own);
    assert_eq!(c.get_storage_funds(maker(), fake), U128(0));
}
