//! NEAR escrow for one leg of a swap between shielded ZEC and a NEP-141 token.
//!
//! A port of `contracts/src/CloakroomEscrow.sol` (v2). The taker and the maker each hold
//! one share of the spend-authorizing key of a shared Orchard address: `ask = a + b`,
//! `A = a·G`, `B = b·G` on Pallas, with G the RedPallas SpendAuth basepoint. The escrow holds
//! the maker's tokens and has two exits, and each exit publishes one share:
//!
//! - `claim(a)` pays the taker and publishes `a`, so the maker can take the ZEC with `a + b`;
//! - `refund(b)` pays the maker back and publishes `b`, so the taker can take the ZEC back.
//!
//! States and windows (`t0 < t1 < t2`, `W = reveal_window`), the same as the Solidity escrow:
//!
//! ```text
//! Open       --set_ready (maker, before t0)-->                   Ready
//! Ready      --commit_claim (taker-signed, before t1)-->         Locked
//! Open/Ready --commit_claim (taker-signed, from t2)-->           Locked   (punish path)
//! Locked     --claim(a) (before locked_at + W)-->                Claimed
//! Open       --refund(b) before t0-->                            Refunded
//! Open/Ready --refund(b) in [t1, t2)-->                          Refunded
//! Locked     --refund(b) from locked_at + W-->                   Refunded
//! ```
//!
//! `claim` and `refund` make no cross-contract calls. They only record a credit. Tokens
//! move in `withdraw`, so a problem on the token side can never make a call that publishes
//! a key share fail.

use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
use near_sdk::{
    env, ext_contract, near, require, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault,
    Promise, PromiseError,
};

pub mod curve;
pub mod hexbytes;

#[cfg(test)]
mod tests;

pub use hexbytes::{Hex32, Hex64, HexBytes};

/// Domain of the swap id hash.
pub const DOMAIN_SWAP_ID: &[u8] = b"near-escrow/v1/swap-id";
/// Domain of the claim commitment hash.
pub const DOMAIN_COMMITMENT: &[u8] = b"near-escrow/v1/commitment";
/// Domain of the message that the taker key signs for `commit_claim`.
pub const DOMAIN_COMMIT_CLAIM: &[u8] = b"near-escrow/v1/commit-claim";

/// Gas reserved for `ft_transfer` on the token.
pub const GAS_FT_TRANSFER: Gas = Gas::from_tgas(15);
/// Gas reserved for `ft_transfer_call` on the token. The token passes most of it on to
/// the Intents contract's `ft_on_transfer` and keeps some for `ft_resolve_transfer`.
pub const GAS_FT_TRANSFER_CALL: Gas = Gas::from_tgas(60);
/// Gas reserved for the `on_withdraw` callback. It reads and writes one credit entry.
pub const GAS_ON_WITHDRAW: Gas = Gas::from_tgas(10);

/// NEP-297 standard name and version of the events.
pub const EVENT_STANDARD: &str = "near_escrow";
pub const EVENT_VERSION: &str = "1.0.0";

const ERR_BAD_PARAMS: &str = "BadParams";
const ERR_BAD_STATE: &str = "BadState";
const ERR_TOO_EARLY: &str = "TooEarly";
const ERR_TOO_LATE: &str = "TooLate";
const ERR_BAD_SECRET: &str = "BadSecret";
const ERR_BAD_SIGNATURE: &str = "BadSignature";
const ERR_NOT_MAKER: &str = "NotMaker";
const ERR_NOT_CLAIMABLE: &str = "NotClaimable";
const ERR_STORAGE: &str = "InsufficientStorage";

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Ready,
    Locked,
    Claimed,
    Refunded,
}

/// How a credit leaves the escrow.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// `ft_transfer` to the owner. Used for the maker's refund.
    Direct,
    /// `ft_transfer_call` to the Intents contract with `msg` = owner. Used for the taker.
    Intents,
}

/// One swap as stored. Every field has a fixed size after `open`, so later transitions
/// never grow the contract's storage and can never fail for lack of storage balance.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Swap {
    pub maker: AccountId,
    pub token: AccountId,
    pub amount: u128,
    /// The taker's ed25519 public key. It signs `commit_claim`.
    pub taker_pk: [u8; 32],
    /// A = a·G, compressed.
    pub a_pub: [u8; 32],
    /// B = b·G, compressed.
    pub b_pub: [u8; 32],
    pub t0: u64,
    pub t1: u64,
    pub t2: u64,
    pub reveal_window: u64,
    pub state: State,
    /// Set by `commit_claim`. When false, `locked_at` and `commitment` are zero.
    pub locked: bool,
    pub locked_at: u64,
    pub commitment: [u8; 32],
    /// The taker's share, stored by `claim`. Zero until then.
    pub a: [u8; 32],
    /// The maker's share, stored by `refund`. Zero until then.
    pub b: [u8; 32],
}

/// The swap terms in the `msg` of the maker's `ft_transfer_call`.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct OpenTerms {
    pub salt: Hex32,
    pub taker_pk: Hex32,
    pub a: Hex32,
    pub b: Hex32,
    pub t0: u64,
    pub t1: u64,
    pub t2: u64,
    pub reveal_window: u64,
}

/// A swap as returned by `get_swap`.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapView {
    pub id: Hex32,
    pub maker: AccountId,
    pub token: AccountId,
    pub amount: U128,
    pub taker_pk: Hex32,
    /// The taker's NEAR and Intents account: the lowercase hex of `taker_pk`.
    pub taker_account: AccountId,
    pub a_pub: Hex32,
    pub b_pub: Hex32,
    pub t0: u64,
    pub t1: u64,
    pub t2: u64,
    pub reveal_window: u64,
    pub state: State,
    pub locked_at: Option<u64>,
    pub commitment: Option<Hex32>,
    /// The taker's share, once claimed.
    pub a: Option<Hex32>,
    /// The maker's share, once refunded.
    pub b: Option<Hex32>,
}

/// Withdrawable credit of one owner in one token, by route.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditView {
    /// Paid by `ft_transfer` to the owner (maker refunds).
    pub direct: U128,
    /// Paid into the Intents contract for the owner (taker claims).
    pub intents: U128,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreditKey {
    pub token: AccountId,
    pub owner: AccountId,
    pub route: Route,
}

/// Storage funds are kept per (owner, token). `ft_on_transfer` trusts the calling token
/// for `sender_id`, so a fake token could name any maker. Keying the funds by token means a
/// fake token can only spend funds deposited for that fake token.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FundsKey {
    pub owner: AccountId,
    pub token: AccountId,
}

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum Prefix {
    Swaps,
    Credits,
    StorageFunds,
}

#[ext_contract(ext_ft)]
#[allow(dead_code)]
trait FungibleToken {
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> U128;
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Escrow {
    intents_account: AccountId,
    swaps: LookupMap<[u8; 32], Swap>,
    credits: LookupMap<CreditKey, u128>,
    /// Prepaid NEAR (yocto) per (maker, token) for the storage that new swaps use.
    storage_funds: LookupMap<FundsKey, u128>,
}

fn fail(code: &str, detail: &str) -> ! {
    env::panic_str(&format!("{code}: {detail}"))
}

fn now() -> u64 {
    env::block_timestamp() / 1_000_000_000
}

fn emit(event: &str, data: serde_json::Value) {
    let body = json!({
        "standard": EVENT_STANDARD,
        "version": EVENT_VERSION,
        "event": event,
        "data": [data],
    });
    env::log_str(&format!("EVENT_JSON:{body}"));
}

fn h(b: &[u8; 32]) -> String {
    hex::encode(b)
}

/// The taker's implicit account id: the lowercase hex of the ed25519 public key.
pub fn taker_account_of(taker_pk: &[u8; 32]) -> AccountId {
    // 64 lowercase hex characters are always a valid NEAR implicit account id.
    hex::encode(taker_pk)
        .parse()
        .unwrap_or_else(|_| env::panic_str("invalid implicit account id"))
}

/// `sha256(DOMAIN_SWAP_ID ‖ len(maker) as u8 ‖ maker ‖ salt)`.
pub fn compute_swap_id(maker: &AccountId, salt: &[u8; 32]) -> [u8; 32] {
    let m = maker.as_bytes();
    let mut buf = Vec::with_capacity(DOMAIN_SWAP_ID.len() + 1 + m.len() + 32);
    buf.extend_from_slice(DOMAIN_SWAP_ID);
    // Account ids are at most 64 bytes, so one length byte is enough.
    buf.push(m.len() as u8);
    buf.extend_from_slice(m);
    buf.extend_from_slice(salt);
    env::sha256_array(&buf)
}

/// `sha256(DOMAIN_COMMITMENT ‖ id ‖ a)`.
pub fn compute_commitment(id: &[u8; 32], a: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(DOMAIN_COMMITMENT.len() + 64);
    buf.extend_from_slice(DOMAIN_COMMITMENT);
    buf.extend_from_slice(id);
    buf.extend_from_slice(a);
    env::sha256_array(&buf)
}

/// `DOMAIN_COMMIT_CLAIM ‖ len(escrow) as u8 ‖ escrow ‖ id ‖ commitment`.
pub fn build_commit_message(escrow: &AccountId, id: &[u8; 32], commitment: &[u8; 32]) -> Vec<u8> {
    let e = escrow.as_bytes();
    let mut buf = Vec::with_capacity(DOMAIN_COMMIT_CLAIM.len() + 1 + e.len() + 64);
    buf.extend_from_slice(DOMAIN_COMMIT_CLAIM);
    buf.push(e.len() as u8);
    buf.extend_from_slice(e);
    buf.extend_from_slice(id);
    buf.extend_from_slice(commitment);
    buf
}

fn storage_cost(before: u64, after: u64) -> u128 {
    u128::from(after.saturating_sub(before)) * env::storage_byte_cost().as_yoctonear()
}

#[near]
impl Escrow {
    /// `intents_account` is the NEAR Intents verifier contract (`intents.near` on mainnet).
    #[init]
    pub fn new(intents_account: AccountId) -> Self {
        Self {
            intents_account,
            swaps: LookupMap::new(Prefix::Swaps),
            credits: LookupMap::new(Prefix::Credits),
            storage_funds: LookupMap::new(Prefix::StorageFunds),
        }
    }

    // ------------------------------------------------------------------ storage funds

    /// Adds the attached NEAR to the storage funds of `account_id` (default: the caller)
    /// for swaps in `token`. `open` takes the storage cost of each new swap from the
    /// maker's funds for the token of that swap. Returns the new balance.
    #[payable]
    pub fn add_storage_funds(&mut self, token: AccountId, account_id: Option<AccountId>) -> U128 {
        let key = FundsKey {
            owner: account_id.unwrap_or_else(env::predecessor_account_id),
            token,
        };
        let deposit = env::attached_deposit().as_yoctonear();
        let before = env::storage_usage();
        let balance = self.storage_funds.get(&key).copied().unwrap_or(0);
        self.storage_funds.insert(key.clone(), balance);
        self.storage_funds.flush();
        let entry_cost = storage_cost(before, env::storage_usage());
        let total = balance
            .checked_add(deposit)
            .unwrap_or_else(|| fail(ERR_BAD_PARAMS, "overflow"));
        if total < entry_cost {
            fail(
                ERR_STORAGE,
                &format!("attach at least {entry_cost} yoctoNEAR"),
            );
        }
        let new_balance = total - entry_cost;
        self.storage_funds.insert(key, new_balance);
        U128(new_balance)
    }

    /// Sends unused storage funds for `token` back to the caller. Requires exactly
    /// 1 yoctoNEAR. `amount` defaults to the whole balance. Returns the remaining balance.
    #[payable]
    pub fn withdraw_storage_funds(&mut self, token: AccountId, amount: Option<U128>) -> U128 {
        require!(
            env::attached_deposit() == NearToken::from_yoctonear(1),
            "Requires attached deposit of exactly 1 yoctoNEAR"
        );
        let owner = env::predecessor_account_id();
        let key = FundsKey {
            owner: owner.clone(),
            token,
        };
        let balance = self.storage_funds.get(&key).copied().unwrap_or(0);
        let amount = amount.map(|a| a.0).unwrap_or(balance);
        if amount > balance {
            fail(ERR_STORAGE, "amount exceeds storage funds");
        }
        let rest = balance - amount;
        if self.storage_funds.contains_key(&key) {
            self.storage_funds.insert(key, rest);
        }
        if amount > 0 {
            Promise::new(owner)
                .transfer(NearToken::from_yoctonear(amount))
                .detach();
        }
        U128(rest)
    }

    pub fn get_storage_funds(&self, account_id: AccountId, token: AccountId) -> U128 {
        let key = FundsKey {
            owner: account_id,
            token,
        };
        U128(self.storage_funds.get(&key).copied().unwrap_or(0))
    }

    // ------------------------------------------------------------------ open

    /// NEP-141 receiver. The maker opens a swap with `ft_transfer_call` on the token, with
    /// `receiver_id` = this contract and `msg` = JSON [`OpenTerms`]. The token is the caller.
    /// Returns "0" (all tokens kept) on success and panics on bad input, so the token
    /// returns the whole amount to the maker.
    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> U128 {
        let token = env::predecessor_account_id();
        let terms: OpenTerms = serde_json::from_str(&msg)
            .unwrap_or_else(|e| fail(ERR_BAD_PARAMS, &format!("msg is not valid swap terms: {e}")));
        let now = now();
        if amount.0 == 0 {
            fail(ERR_BAD_PARAMS, "amount is zero");
        }
        if terms.t0 <= now || terms.t1 <= terms.t0 || terms.t2 <= terms.t1 {
            fail(ERR_BAD_PARAMS, "need now < t0 < t1 < t2");
        }
        if terms.reveal_window == 0 {
            fail(ERR_BAD_PARAMS, "reveal_window is zero");
        }
        if terms.taker_pk.0 == [0u8; 32] {
            fail(ERR_BAD_PARAMS, "taker_pk is zero");
        }
        if curve::parse_point(&terms.a.0).is_none() {
            fail(ERR_BAD_PARAMS, "a is not a valid Pallas point");
        }
        if curve::parse_point(&terms.b.0).is_none() {
            fail(ERR_BAD_PARAMS, "b is not a valid Pallas point");
        }
        let id = compute_swap_id(&sender_id, &terms.salt.0);
        if self.swaps.contains_key(&id) {
            fail(ERR_BAD_STATE, "swap id already used");
        }

        let before = env::storage_usage();
        let taker_account = taker_account_of(&terms.taker_pk.0);
        let swap = Swap {
            maker: sender_id.clone(),
            token: token.clone(),
            amount: amount.0,
            taker_pk: terms.taker_pk.0,
            a_pub: terms.a.0,
            b_pub: terms.b.0,
            t0: terms.t0,
            t1: terms.t1,
            t2: terms.t2,
            reveal_window: terms.reveal_window,
            state: State::Open,
            locked: false,
            locked_at: 0,
            commitment: [0; 32],
            a: [0; 32],
            b: [0; 32],
        };
        self.swaps.insert(id, swap);
        // Create both credit entries now, paid by the maker, so that claim and refund only
        // update existing entries and never need new storage.
        for key in [
            CreditKey {
                token: token.clone(),
                owner: sender_id.clone(),
                route: Route::Direct,
            },
            CreditKey {
                token: token.clone(),
                owner: taker_account.clone(),
                route: Route::Intents,
            },
        ] {
            if !self.credits.contains_key(&key) {
                self.credits.insert(key, 0);
            }
        }
        self.swaps.flush();
        self.credits.flush();
        let cost = storage_cost(before, env::storage_usage());
        let funds_key = FundsKey {
            owner: sender_id.clone(),
            token: token.clone(),
        };
        let funds = self.storage_funds.get(&funds_key).copied();
        match funds {
            Some(f) if f >= cost => {
                self.storage_funds.insert(funds_key, f - cost);
            }
            _ => fail(
                ERR_STORAGE,
                &format!(
                    "maker storage funds for this token {} yoctoNEAR, swap needs {cost}; \
                     call add_storage_funds",
                    funds.unwrap_or(0)
                ),
            ),
        }

        emit(
            "opened",
            json!({
                "id": h(&id),
                "maker": sender_id,
                "token": token,
                "amount": amount,
                "taker_pk": h(&terms.taker_pk.0),
                "taker_account": taker_account,
                "a_pub": h(&terms.a.0),
                "b_pub": h(&terms.b.0),
                "t0": terms.t0,
                "t1": terms.t1,
                "t2": terms.t2,
                "reveal_window": terms.reveal_window,
            }),
        );
        U128(0)
    }

    // ------------------------------------------------------------------ transitions

    /// The maker confirms that the ZEC note is at the shared address. Maker only, state
    /// Open, before t0.
    pub fn set_ready(&mut self, id: Hex32) {
        let maker = env::predecessor_account_id();
        let swap = self.swap_mut(&id.0);
        if maker != swap.maker {
            fail(ERR_NOT_MAKER, "only the maker can set Ready");
        }
        if swap.state != State::Open {
            fail(ERR_BAD_STATE, "swap is not Open");
        }
        if now() >= swap.t0 {
            fail(ERR_TOO_LATE, "t0 has passed");
        }
        swap.state = State::Ready;
        emit("ready", json!({ "id": h(&id.0) }));
    }

    /// Locks the swap for the taker and stores only `commitment = commitment_of(id, a)`.
    /// Anyone may send it. `signature` is the taker key's ed25519 signature over
    /// `commit_message(id, commitment)`. Allowed in Ready before t1, or (punish path) in
    /// Open or Ready from t2.
    pub fn commit_claim(&mut self, id: Hex32, commitment: Hex32, signature: Hex64) {
        let escrow = env::current_account_id();
        let now = now();
        let swap = self.swap_mut(&id.0);
        let st = swap.state;
        if st != State::Ready && st != State::Open {
            fail(ERR_BAD_STATE, "swap is not Open or Ready");
        }
        if !((st == State::Ready && now < swap.t1) || now >= swap.t2) {
            fail(
                ERR_NOT_CLAIMABLE,
                "commit_claim needs Ready before t1, or any time from t2",
            );
        }
        let message = build_commit_message(&escrow, &id.0, &commitment.0);
        if !env::ed25519_verify(&signature.0, &message, &swap.taker_pk) {
            fail(ERR_BAD_SIGNATURE, "not signed by the taker key");
        }
        swap.state = State::Locked;
        swap.locked = true;
        swap.locked_at = now;
        swap.commitment = commitment.0;
        emit(
            "locked",
            json!({ "id": h(&id.0), "commitment": h(&commitment.0), "locked_at": now }),
        );
    }

    /// Publishes the taker's share `a` (32 bytes, little-endian) and credits the taker.
    /// Anyone may send it. State Locked, before locked_at + W. No token moves here.
    pub fn claim(&mut self, id: Hex32, a: Hex32) {
        let now = now();
        let swap = self.swap_mut(&id.0);
        if swap.state != State::Locked {
            fail(ERR_BAD_STATE, "swap is not Locked");
        }
        if u128::from(now) >= u128::from(swap.locked_at) + u128::from(swap.reveal_window) {
            fail(ERR_TOO_LATE, "reveal window has closed");
        }
        if compute_commitment(&id.0, &a.0) != swap.commitment {
            fail(ERR_BAD_SECRET, "a does not open the commitment");
        }
        if !curve::is_discrete_log_of(&a.0, &swap.a_pub) {
            fail(ERR_BAD_SECRET, "a·G != A");
        }
        swap.state = State::Claimed;
        swap.a = a.0;
        let key = CreditKey {
            token: swap.token.clone(),
            owner: taker_account_of(&swap.taker_pk),
            route: Route::Intents,
        };
        let amount = swap.amount;
        self.add_credit(key, amount);
        emit("claimed", json!({ "id": h(&id.0), "a": h(&a.0) }));
    }

    /// Publishes the maker's share `b` (32 bytes, little-endian) and credits the maker.
    /// Anyone may send it. Allowed in Open before t0, in Open or Ready in [t1, t2), and in
    /// Locked from locked_at + W. No token moves here.
    pub fn refund(&mut self, id: Hex32, b: Hex32) {
        let now = now();
        let swap = self.swap_mut(&id.0);
        let st = swap.state;
        let ok = (st == State::Open && now < swap.t0)
            || ((st == State::Open || st == State::Ready) && now >= swap.t1 && now < swap.t2)
            || (st == State::Locked
                && u128::from(now) >= u128::from(swap.locked_at) + u128::from(swap.reveal_window));
        if !ok {
            fail(ERR_TOO_EARLY, "no refund window is open");
        }
        if !curve::is_discrete_log_of(&b.0, &swap.b_pub) {
            fail(ERR_BAD_SECRET, "b·G != B");
        }
        swap.state = State::Refunded;
        swap.b = b.0;
        let key = CreditKey {
            token: swap.token.clone(),
            owner: swap.maker.clone(),
            route: Route::Direct,
        };
        let amount = swap.amount;
        self.add_credit(key, amount);
        emit("refunded", json!({ "id": h(&id.0), "b": h(&b.0) }));
    }

    // ------------------------------------------------------------------ withdraw

    /// Pays out the credit of `owner` in `token`. Anyone may call it. Each credit is set to
    /// zero before its transfer starts. A direct credit goes out with `ft_transfer` to the
    /// owner. An Intents credit goes out with `ft_transfer_call` to the Intents contract,
    /// with `msg` = owner, which credits that account inside Intents. `on_withdraw` puts back
    /// whatever the token did not deliver.
    pub fn withdraw(&mut self, token: AccountId, owner: AccountId) {
        for route in [Route::Direct, Route::Intents] {
            let key = CreditKey {
                token: token.clone(),
                owner: owner.clone(),
                route,
            };
            let amount = self.credits.get(&key).copied().unwrap_or(0);
            if amount == 0 {
                continue;
            }
            self.credits.insert(key, 0);
            let one_yocto = NearToken::from_yoctonear(1);
            let transfer = match route {
                Route::Direct => ext_ft::ext(token.clone())
                    .with_attached_deposit(one_yocto)
                    .with_static_gas(GAS_FT_TRANSFER)
                    .ft_transfer(owner.clone(), U128(amount), None),
                Route::Intents => ext_ft::ext(token.clone())
                    .with_attached_deposit(one_yocto)
                    .with_static_gas(GAS_FT_TRANSFER_CALL)
                    .ft_transfer_call(
                        self.intents_account.clone(),
                        U128(amount),
                        None,
                        owner.to_string(),
                    ),
            };
            transfer
                .then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(GAS_ON_WITHDRAW)
                        .with_unused_gas_weight(0)
                        .on_withdraw(token.clone(), owner.clone(), route, U128(amount)),
                )
                .detach();
        }
    }

    /// Callback of `withdraw`. Restores the part of `amount` that was not delivered and
    /// returns the delivered amount.
    ///
    /// - Direct: delivered = amount if `ft_transfer` succeeded, else 0.
    /// - Intents: `ft_transfer_call` returns the amount the receiver kept. Delivered = that
    ///   amount (at most `amount`). If the call failed, delivered = 0. If it succeeded but
    ///   its result cannot be read, delivered = amount, so a credit is never paid twice.
    #[private]
    pub fn on_withdraw(
        &mut self,
        token: AccountId,
        owner: AccountId,
        route: Route,
        amount: U128,
    ) -> U128 {
        // A U128 result is a short JSON string; 64 bytes is more than enough.
        let result = env::promise_result_checked(0, 64);
        let delivered = match (route, result) {
            (_, Err(PromiseError::Failed)) => 0,
            (Route::Direct, _) => amount.0,
            (Route::Intents, Ok(bytes)) => match parse_u128_json(&bytes) {
                Some(used) => used.min(amount.0),
                None => amount.0,
            },
            (Route::Intents, Err(_)) => amount.0,
        };
        let restored = amount.0 - delivered;
        if restored > 0 {
            let key = CreditKey {
                token: token.clone(),
                owner: owner.clone(),
                route,
            };
            self.add_credit(key, restored);
        }
        emit(
            "withdrawn",
            json!({
                "token": token,
                "owner": owner,
                "route": route,
                "amount": U128(delivered),
                "restored": U128(restored),
            }),
        );
        U128(delivered)
    }

    // ------------------------------------------------------------------ views

    pub fn get_intents_account(&self) -> AccountId {
        self.intents_account.clone()
    }

    pub fn get_swap(&self, id: Hex32) -> Option<SwapView> {
        self.swaps.get(&id.0).map(|s| SwapView {
            id,
            maker: s.maker.clone(),
            token: s.token.clone(),
            amount: U128(s.amount),
            taker_pk: Hex32(s.taker_pk),
            taker_account: taker_account_of(&s.taker_pk),
            a_pub: Hex32(s.a_pub),
            b_pub: Hex32(s.b_pub),
            t0: s.t0,
            t1: s.t1,
            t2: s.t2,
            reveal_window: s.reveal_window,
            state: s.state,
            locked_at: s.locked.then_some(s.locked_at),
            commitment: s.locked.then_some(Hex32(s.commitment)),
            a: (s.state == State::Claimed).then_some(Hex32(s.a)),
            b: (s.state == State::Refunded).then_some(Hex32(s.b)),
        })
    }

    pub fn get_credit(&self, token: AccountId, owner: AccountId) -> CreditView {
        let get = |route| {
            let key = CreditKey {
                token: token.clone(),
                owner: owner.clone(),
                route,
            };
            U128(self.credits.get(&key).copied().unwrap_or(0))
        };
        CreditView {
            direct: get(Route::Direct),
            intents: get(Route::Intents),
        }
    }

    pub fn swap_id(&self, maker: AccountId, salt: Hex32) -> Hex32 {
        Hex32(compute_swap_id(&maker, &salt.0))
    }

    /// The exact bytes the taker key signs for `commit_claim` on this escrow.
    pub fn commit_message(&self, id: Hex32, commitment: Hex32) -> HexBytes {
        HexBytes(build_commit_message(
            &env::current_account_id(),
            &id.0,
            &commitment.0,
        ))
    }

    pub fn commitment_of(&self, id: Hex32, a: Hex32) -> Hex32 {
        Hex32(compute_commitment(&id.0, &a.0))
    }
}

impl Escrow {
    fn swap_mut(&mut self, id: &[u8; 32]) -> &mut Swap {
        self.swaps
            .get_mut(id)
            .unwrap_or_else(|| fail(ERR_BAD_STATE, "unknown swap"))
    }

    fn add_credit(&mut self, key: CreditKey, amount: u128) {
        let current = self.credits.get(&key).copied().unwrap_or(0);
        let next = current
            .checked_add(amount)
            .unwrap_or_else(|| env::panic_str("credit overflow"));
        self.credits.insert(key, next);
    }
}

/// Reads a JSON string holding a u128, the result of `ft_transfer_call`.
pub fn parse_u128_json(bytes: &[u8]) -> Option<u128> {
    match serde_json::from_slice::<serde_json::Value>(bytes).ok()? {
        serde_json::Value::String(s) => s.parse::<u128>().ok(),
        _ => None,
    }
}
