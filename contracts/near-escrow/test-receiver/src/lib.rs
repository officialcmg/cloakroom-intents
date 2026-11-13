//! Test only. A stand-in for the NEAR Intents Verifier, used by `tests/usdc.rs` to place a
//! USDC restriction exactly between the receiver call and USDC's resolve step.
//!
//! It keeps multi-token balances like the Verifier, for the methods the escrow uses:
//!
//! - `ft_on_transfer(sender_id, amount, msg)`: credits `msg` (an account id), or
//!   `sender_id` if `msg` is empty, with `nep141:<token>`. If a restriction is armed, it
//!   then calls the token (it holds the USDC role for it: blocklister or pauser) and
//!   returns `"0"` only after that call. USDC's `ft_resolve_transfer` waits for this whole
//!   chain, so the restriction is in place before the resolve step runs, every time.
//!   With `reject_amount` armed, a deposit of exactly that amount panics instead (the
//!   token then returns the tokens to the sender).
//! - `mt_transfer`, `mt_balance_of`, `ft_withdraw` with the Verifier's arguments.
//!   `mt_transfer` and `ft_withdraw` move only the caller's own balance, and need
//!   exactly 1 yoctoNEAR.
//!
//! It is not a Verifier: no intents, no keys, no fees.

use near_sdk::json_types::U128;
use near_sdk::store::LookupMap;
use near_sdk::{
    env, ext_contract, near, require, AccountId, Gas, NearToken, PanicOnDefault, PromiseOrValue,
};

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restriction {
    /// `token.blocklist(account_id)`; the receiver must be the token's blocklister.
    Blocklist { account_id: AccountId },
    /// `token.execute_multisig_request(request_id)`; the receiver must be a pauser and the
    /// request (for example a Pause) must already have its approvals.
    ExecuteMultisig { request_id: u32 },
}

#[ext_contract(ext_usdc)]
#[allow(dead_code)]
trait Usdc {
    fn blocklist(&mut self, account_id: AccountId);
    fn execute_multisig_request(&mut self, request_id: u32);
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Receiver {
    balances: LookupMap<(AccountId, String), u128>,
    armed: Option<Restriction>,
    reject_amount: Option<u128>,
}

#[near]
impl Receiver {
    #[init]
    pub fn new() -> Self {
        Self {
            balances: LookupMap::new(b"b"),
            armed: None,
            reject_amount: None,
        }
    }

    /// Arms one restriction for the next deposit. Test setup; anyone may call it.
    pub fn arm(&mut self, restriction: Option<Restriction>) {
        self.armed = restriction;
    }

    /// Makes every deposit of exactly `amount` fail. Test setup; anyone may call it.
    pub fn reject_deposits_of(&mut self, amount: Option<U128>) {
        self.reject_amount = amount.map(|a| a.0);
    }

    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        require!(
            self.reject_amount != Some(amount.0),
            "deposit rejected (test)"
        );
        let token = env::predecessor_account_id();
        let owner: AccountId = if msg.is_empty() {
            sender_id
        } else {
            msg.parse()
                .unwrap_or_else(|_| env::panic_str("msg is not an account id"))
        };
        self.add(&owner, &format!("nep141:{token}"), amount.0);
        match self.armed.take() {
            None => PromiseOrValue::Value(U128(0)),
            Some(r) => {
                let call = ext_usdc::ext(token).with_static_gas(Gas::from_tgas(10));
                let p = match r {
                    Restriction::Blocklist { account_id } => call.blocklist(account_id),
                    Restriction::ExecuteMultisig { request_id } => {
                        call.execute_multisig_request(request_id)
                    }
                };
                p.then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(5))
                        .keep_all(),
                )
                .into()
            }
        }
    }

    /// Returns "0" (all tokens kept) after the restriction call. Panics if that call
    /// failed, so a broken test setup cannot pass silently.
    #[private]
    pub fn keep_all(&mut self) -> U128 {
        require!(
            env::promise_result_checked(0, 1024).is_ok(),
            "restriction call failed"
        );
        U128(0)
    }

    #[payable]
    pub fn mt_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: String,
        amount: U128,
        approval: Option<(AccountId, u64)>,
        memo: Option<String>,
    ) {
        let _ = memo;
        require!(approval.is_none(), "approvals are not supported");
        require!(
            env::attached_deposit() == NearToken::from_yoctonear(1),
            "requires 1 yoctoNEAR"
        );
        require!(amount.0 > 0, "zero amount");
        let sender = env::predecessor_account_id();
        self.sub(&sender, &token_id, amount.0);
        self.add(&receiver_id, &token_id, amount.0);
    }

    pub fn mt_balance_of(&self, account_id: AccountId, token_id: String) -> U128 {
        U128(
            self.balances
                .get(&(account_id, token_id))
                .copied()
                .unwrap_or(0),
        )
    }

    #[payable]
    pub fn ft_withdraw(
        &mut self,
        token: AccountId,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: Option<String>,
    ) -> PromiseOrValue<U128> {
        require!(msg.is_none(), "msg is not supported");
        require!(
            env::attached_deposit() == NearToken::from_yoctonear(1),
            "requires 1 yoctoNEAR"
        );
        let sender = env::predecessor_account_id();
        let token_id = format!("nep141:{token}");
        self.sub(&sender, &token_id, amount.0);
        ext_usdc::ext(token)
            .with_attached_deposit(NearToken::from_yoctonear(1))
            .with_static_gas(Gas::from_tgas(15))
            .ft_transfer(receiver_id, amount, memo)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(10))
                    .resolve_withdraw(sender, token_id, amount),
            )
            .into()
    }

    /// Like the Verifier: on a failed `ft_transfer`, the balance goes back to the sender.
    #[private]
    pub fn resolve_withdraw(&mut self, sender: AccountId, token_id: String, amount: U128) -> U128 {
        if env::promise_result_checked(0, 16).is_ok() {
            amount
        } else {
            self.add(&sender, &token_id, amount.0);
            U128(0)
        }
    }
}

impl Receiver {
    fn add(&mut self, owner: &AccountId, token_id: &str, amount: u128) {
        let key = (owner.clone(), token_id.to_string());
        let b = self.balances.get(&key).copied().unwrap_or(0);
        self.balances.insert(key, b + amount);
    }

    fn sub(&mut self, owner: &AccountId, token_id: &str, amount: u128) {
        let key = (owner.clone(), token_id.to_string());
        let b = self.balances.get(&key).copied().unwrap_or(0);
        require!(b >= amount, "insufficient balance");
        self.balances.insert(key, b - amount);
    }
}
