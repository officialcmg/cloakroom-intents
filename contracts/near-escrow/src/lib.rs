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
//! The tokens are kept inside the NEAR Intents Verifier (`intents_account`), in the
//! escrow's own Verifier account. The maker opens a swap with `mt_transfer_call` on the
//! Verifier, so the swap exists only after the Verifier has moved the tokens to the
//! escrow. A payout is one `mt_transfer` inside the Verifier, from the escrow to the
//! taker (claim) or to the maker (refund). It is one receipt: it moves the whole amount
//! or nothing. No call of the escrow touches the token contract.
//!
//! States and windows (`t0 < t1 < t2`, `W = reveal_window`), the same as the Solidity escrow:
//!
//! ```text
//! (none)     --mt_on_transfer from the Verifier (maker's mt_transfer_call)--> Open
//! Open       --set_ready (maker, before t0)-->                   Ready
//! Ready      --commit_claim (taker-signed, before t1)-->         Locked
//! Open/Ready --commit_claim (taker-signed, from t2)-->           Locked   (punish path)
//! Locked     --claim(a) (before locked_at + W)-->                Claimed
//! Open       --refund(b) before t0-->                            Refunded
//! Open/Ready --refund(b) in [t1, t2)-->                          Refunded
//! Locked     --refund(b) from locked_at + W-->                   Refunded
//! ```
//!
//! Each swap also has a payout state. It changes only in Claimed and Refunded:
//!
//! ```text
//! None    --claim / refund-->                         Due
//! Due     --(same call, if it has the gas) / withdraw-->  Sending   (mt_transfer started)
//! Sending --on_payout: mt_transfer succeeded-->       Paid
//! Sending --on_payout: mt_transfer failed-->          Due
//! ```
//!
//! `claim` and `refund` store the key share and the new state first. They start the
//! payout only when enough gas is left for it; otherwise the payout stays Due and
//! `withdraw(id)` sends it. A failed payout never undoes a published key share.

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

/// Gas reserved for `mt_transfer` on the Verifier. It is one receipt with no further
/// calls. The live Verifier code burns about 4 Tgas for it. The `mt_transfer` also gets
/// all gas that the caller attached on top, so a payout can always be sent with more gas.
pub const GAS_MT_TRANSFER: Gas = Gas::from_tgas(7);
/// Gas reserved for the `on_payout` callback. It reads and writes one swap and one
/// counter, and burns about 2.3 Tgas. If it ran out of gas, the payout would stay
/// Sending forever, so the reservation is more than three times that.
pub const GAS_ON_PAYOUT: Gas = Gas::from_tgas(8);
/// Gas that must be left in `claim`, `refund` or `withdraw` when it starts a payout: the
/// two reservations above and the fees of the two receipts and of the data receipt.
pub const GAS_PAYOUT_START: Gas = GAS_MT_TRANSFER
    .saturating_add(GAS_ON_PAYOUT)
    .saturating_add(Gas::from_tgas(3));

/// NEP-297 standard name and version of the events.
pub const EVENT_STANDARD: &str = "near_escrow";
pub const EVENT_VERSION: &str = "2.0.0";

const ERR_BAD_PARAMS: &str = "BadParams";
const ERR_BAD_STATE: &str = "BadState";
const ERR_TOO_EARLY: &str = "TooEarly";
const ERR_TOO_LATE: &str = "TooLate";
const ERR_BAD_SECRET: &str = "BadSecret";
const ERR_BAD_SIGNATURE: &str = "BadSignature";
const ERR_NOT_MAKER: &str = "NotMaker";
const ERR_NOT_CLAIMABLE: &str = "NotClaimable";
const ERR_STORAGE: &str = "InsufficientStorage";
const ERR_NOT_VERIFIER: &str = "NotVerifier";
const ERR_GAS: &str = "NotEnoughGas";

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Ready,
    Locked,
    Claimed,
    Refunded,
}

/// Where the tokens of a swap are, after the swap ended.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Payout {
    /// The swap has not ended. The tokens back the swap.
    None,
    /// The swap ended (Claimed or Refunded). The tokens are still in the escrow's
    /// Verifier account. `withdraw(id)` sends them.
    Due,
    /// `mt_transfer` to the recipient has started; `on_payout` has not run yet.
    Sending,
    /// `mt_transfer` to the recipient succeeded. Final.
    Paid,
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
    pub payout: Payout,
}

/// The swap terms in the `msg` of the maker's `mt_transfer_call`.
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
    pub payout: Payout,
    /// Who receives the payout: the taker account after a claim, the maker after a
    /// refund, none before.
    pub recipient: Option<AccountId>,
}

/// Storage funds are kept per (owner, token). Only the owner can deposit funds for
/// itself (review-1 F-01).
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
    StorageFunds,
    Owed,
}

/// The methods of the Verifier (NEAR Intents) that the escrow calls.
#[ext_contract(ext_intents)]
#[allow(dead_code)]
trait Intents {
    fn mt_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: String,
        amount: U128,
        approval: Option<(AccountId, u64)>,
        memo: Option<String>,
    );
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Escrow {
    intents_account: AccountId,
    swaps: LookupMap<[u8; 32], Swap>,
    /// Prepaid NEAR (yocto) per (maker, token) for the storage that new swaps use.
    storage_funds: LookupMap<FundsKey, u128>,
    /// Per token: the sum of the amounts of all swaps whose payout is not Paid. These
    /// tokens must be in the escrow's Verifier account. The entry for a token is created
    /// by the first open in that token.
    owed: LookupMap<AccountId, u128>,
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

/// The token id of a NEP-141 token inside the Verifier.
pub fn nep141_id(token: &AccountId) -> String {
    format!("nep141:{token}")
}

/// The NEP-141 token of a Verifier token id `nep141:<account>`. None for other kinds.
pub fn token_of_nep141_id(token_id: &str) -> Option<AccountId> {
    token_id.strip_prefix("nep141:")?.parse().ok()
}

fn storage_cost(before: u64, after: u64) -> u128 {
    u128::from(after.saturating_sub(before)) * env::storage_byte_cost().as_yoctonear()
}

fn recipient_of(swap: &Swap) -> Option<AccountId> {
    match swap.state {
        State::Claimed => Some(taker_account_of(&swap.taker_pk)),
        State::Refunded => Some(swap.maker.clone()),
        _ => None,
    }
}

#[near]
impl Escrow {
    /// `intents_account` is the NEAR Intents Verifier contract (`intents.near` on
    /// mainnet).
    ///
    /// Private: only the escrow account itself can call it, so nobody else can initialise
    /// a deployed escrow with their own `intents_account`. Deploy the code, call `new` and
    /// delete the account's access key in one transaction that the escrow account sends to
    /// itself (see the README, "Deployment").
    #[init]
    #[private]
    pub fn new(intents_account: AccountId) -> Self {
        Self {
            intents_account,
            swaps: LookupMap::new(Prefix::Swaps),
            storage_funds: LookupMap::new(Prefix::StorageFunds),
            owed: LookupMap::new(Prefix::Owed),
        }
    }

    // ------------------------------------------------------------------ storage funds

    /// Adds the attached NEAR to the caller's storage funds for swaps in `token`. `open`
    /// takes the storage cost of each new swap from the maker's funds for the token of
    /// that swap. Returns the new balance.
    ///
    /// `account_id` is optional and, if given, must be the caller (review-1 F-01).
    #[payable]
    pub fn add_storage_funds(&mut self, token: AccountId, account_id: Option<AccountId>) -> U128 {
        let owner = env::predecessor_account_id();
        if account_id.is_some_and(|a| a != owner) {
            fail(
                ERR_BAD_PARAMS,
                "storage funds can only be added for the caller",
            );
        }
        let key = FundsKey { owner, token };
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

    /// NEP-245 receiver, called only by the Verifier. The maker opens a swap with
    /// `mt_transfer_call` on the Verifier: `receiver_id` = this contract, `token_id` =
    /// `nep141:<token>`, `msg` = JSON [`OpenTerms`]. The Verifier has already moved the
    /// tokens to the escrow's Verifier account when it calls this method. `sender_id` is
    /// the account that the Verifier authenticated as the sender; it becomes the maker.
    ///
    /// Returns `["0"]` (all tokens kept) on success and panics on bad input. After a
    /// panic the Verifier's `mt_resolve_transfer` returns the whole amount to the maker
    /// and no swap exists.
    pub fn mt_on_transfer(
        &mut self,
        sender_id: AccountId,
        previous_owner_ids: Vec<AccountId>,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        msg: String,
    ) -> Vec<U128> {
        if env::predecessor_account_id() != self.intents_account {
            fail(ERR_NOT_VERIFIER, "only the Verifier can open a swap");
        }
        if token_ids.len() != 1 || amounts.len() != 1 || previous_owner_ids.len() != 1 {
            fail(ERR_BAD_PARAMS, "exactly one token per swap");
        }
        if previous_owner_ids[0] != sender_id {
            fail(ERR_BAD_PARAMS, "the sender must own the tokens");
        }
        let token = token_of_nep141_id(&token_ids[0])
            .unwrap_or_else(|| fail(ERR_BAD_PARAMS, "token is not a nep141 token"));
        let amount = amounts[0].0;
        self.open_swap(sender_id, token, amount, &msg);
        vec![U128(0)]
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

    /// Publishes the taker's share `a` (32 bytes, little-endian) and pays the taker.
    /// Anyone may send it. State Locked, before locked_at + W.
    ///
    /// The payout (`mt_transfer` of the amount to the taker's account inside the
    /// Verifier) starts in the same call if at least [`GAS_PAYOUT_START`] is left after
    /// the checks. Otherwise the payout stays Due and `withdraw(id)` sends it.
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
        swap.payout = Payout::Due;
        emit("claimed", json!({ "id": h(&id.0), "a": h(&a.0) }));
        self.start_payout_if_gas(&id.0);
    }

    /// Publishes the maker's share `b` (32 bytes, little-endian) and pays the maker back.
    /// Anyone may send it. Allowed in Open before t0, in Open or Ready in [t1, t2), and in
    /// Locked from locked_at + W. The payout starts as in `claim`.
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
        swap.payout = Payout::Due;
        emit("refunded", json!({ "id": h(&id.0), "b": h(&b.0) }));
        self.start_payout_if_gas(&id.0);
    }

    // ------------------------------------------------------------------ payout

    /// Sends a Due payout: `mt_transfer` of the swap's amount from the escrow's Verifier
    /// account to the recipient (the taker after a claim, the maker after a refund).
    /// Anyone may call it. The recipient is fixed by the swap. Fails if the payout is not
    /// Due or if less than [`GAS_PAYOUT_START`] is left. The gas attached on top goes to
    /// `mt_transfer`.
    pub fn withdraw(&mut self, id: Hex32) {
        let payout = self.swap_mut(&id.0).payout;
        if payout != Payout::Due {
            fail(ERR_BAD_STATE, "payout is not Due");
        }
        if !self.has_payout_gas() {
            fail(ERR_GAS, "attach more gas");
        }
        self.start_payout(&id.0);
    }

    /// Callback of the payout's `mt_transfer`. `mt_transfer` is one receipt in the
    /// Verifier with no further calls: a failed result means that nothing moved, a
    /// successful one that the whole amount moved.
    #[private]
    pub fn on_payout(&mut self, id: Hex32) -> bool {
        // mt_transfer returns nothing on success; 64 bytes is more than enough.
        let ok = !matches!(
            env::promise_result_checked(0, 64),
            Err(PromiseError::Failed)
        );
        let swap = self.swap_mut(&id.0);
        if swap.payout != Payout::Sending {
            // Cannot happen: only start_payout sets Sending, and only this callback
            // leaves it. Fail rather than count a payment twice.
            env::panic_str("payout is not Sending");
        }
        let token = swap.token.clone();
        let amount = swap.amount;
        let recipient = recipient_of(swap);
        if ok {
            swap.payout = Payout::Paid;
            let owed = self.owed.get(&token).copied().unwrap_or(0);
            // owed always includes this swap's amount until here.
            self.owed.insert(token.clone(), owed.saturating_sub(amount));
        } else {
            swap.payout = Payout::Due;
        }
        emit(
            if ok { "paid" } else { "payout_failed" },
            json!({
                "id": h(&id.0),
                "token": token,
                "recipient": recipient,
                "amount": U128(amount),
            }),
        );
        ok
    }

    // ------------------------------------------------------------------ views

    pub fn get_intents_account(&self) -> AccountId {
        self.intents_account.clone()
    }

    /// The sum of the amounts of all swaps in `token` whose payout is not Paid. The
    /// escrow's Verifier balance of `nep141:<token>` must be at least this much. Read both
    /// at the same final block to check that the escrow's tokens back its swaps.
    pub fn get_owed(&self, token: AccountId) -> U128 {
        U128(self.owed.get(&token).copied().unwrap_or(0))
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
            payout: s.payout,
            recipient: recipient_of(s),
        })
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

    /// Validates the terms, stores a new Open swap of `amount` `token` for `maker` and
    /// adds the amount to the token's owed sum. Panics on any bad input; then nothing is
    /// stored.
    fn open_swap(&mut self, maker: AccountId, token: AccountId, amount: u128, msg: &str) {
        let terms: OpenTerms = serde_json::from_str(msg)
            .unwrap_or_else(|e| fail(ERR_BAD_PARAMS, &format!("msg is not valid swap terms: {e}")));
        let now = now();
        if amount == 0 {
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
        let id = compute_swap_id(&maker, &terms.salt.0);
        if self.swaps.contains_key(&id) {
            fail(ERR_BAD_STATE, "swap id already used");
        }

        let before = env::storage_usage();
        let taker_account = taker_account_of(&terms.taker_pk.0);
        let swap = Swap {
            maker: maker.clone(),
            token: token.clone(),
            amount,
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
            payout: Payout::None,
        };
        self.swaps.insert(id, swap);
        let owed = self.owed.get(&token).copied().unwrap_or(0);
        self.owed.insert(
            token.clone(),
            owed.checked_add(amount)
                .unwrap_or_else(|| fail(ERR_BAD_PARAMS, "owed overflow")),
        );
        self.swaps.flush();
        self.owed.flush();
        let cost = storage_cost(before, env::storage_usage());
        let funds_key = FundsKey {
            owner: maker.clone(),
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
                "maker": maker,
                "token": token,
                "amount": U128(amount),
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
    }

    fn has_payout_gas(&self) -> bool {
        env::prepaid_gas().saturating_sub(env::used_gas()) >= GAS_PAYOUT_START
    }

    /// Starts the payout of a Due swap if the call has the gas for it. A call without
    /// that gas keeps its effect (the key share is published); the payout stays Due.
    fn start_payout_if_gas(&mut self, id: &[u8; 32]) {
        if self.has_payout_gas() {
            self.start_payout(id);
        }
    }

    /// Due -> Sending, then `mt_transfer(recipient, nep141:<token>, amount)` with 1
    /// yoctoNEAR and the callback `on_payout`. The `mt_transfer` also gets all gas that
    /// is left after the reservations; what it does not burn goes back to the signer.
    fn start_payout(&mut self, id: &[u8; 32]) {
        let intents = self.intents_account.clone();
        let swap = self.swap_mut(id);
        require!(swap.payout == Payout::Due, "payout is not Due");
        let recipient = recipient_of(swap).unwrap_or_else(|| env::panic_str("swap has not ended"));
        swap.payout = Payout::Sending;
        let token = swap.token.clone();
        let amount = swap.amount;
        ext_intents::ext(intents)
            .with_attached_deposit(NearToken::from_yoctonear(1))
            .with_static_gas(GAS_MT_TRANSFER)
            .with_unused_gas_weight(1)
            .mt_transfer(recipient, nep141_id(&token), U128(amount), None, None)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_PAYOUT)
                    .with_unused_gas_weight(0)
                    .on_payout(Hex32(*id)),
            )
            .detach();
    }
}
