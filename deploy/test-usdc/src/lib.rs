//! Stand-in USDC for NEAR testnet.
//!
//! A standard NEP-141 fungible token (with NEP-145 storage management and NEP-148
//! metadata) from `near-contract-standards`, with 6 decimals like USDC. It adds one
//! method, `mint`, that anyone can call. It exists so that the Zecret demo on testnet
//! has a token with a reliable faucet. It has no value.
//!
//! The transfer, `ft_transfer_call` and `ft_resolve_transfer` code is the unchanged
//! standard implementation, the same code family as `wrap.near`, which the escrow's
//! sandbox tests use.

use near_contract_standards::fungible_token::events::FtMint;
use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider, FT_METADATA_SPEC,
};
use near_contract_standards::fungible_token::{
    FungibleToken, FungibleTokenCore, FungibleTokenResolver,
};
use near_contract_standards::storage_management::{
    StorageBalance, StorageBalanceBounds, StorageManagement,
};
use near_sdk::borsh::BorshSerialize;
use near_sdk::collections::LazyOption;
use near_sdk::json_types::U128;
use near_sdk::{
    env, log, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise,
    PromiseOrValue,
};

/// 6 decimals, like USDC.
pub const DECIMALS: u8 = 6;
/// The most one `mint` call can create: 10 000 tokens.
pub const MAX_MINT: u128 = 10_000 * 10u128.pow(DECIMALS as u32);

const ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Ccircle cx='16' cy='16' r='16' fill='%23888'/%3E%3Ctext x='16' y='21' font-size='12' text-anchor='middle' fill='white' font-family='sans-serif'%3Et$%3C/text%3E%3C/svg%3E";

#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    Accounts,
    Metadata,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    token: FungibleToken,
    metadata: LazyOption<FungibleTokenMetadata>,
}

#[near]
impl Contract {
    /// Initialises the token with no supply. Private: only the token account itself can
    /// call it, so deploy and init go in one transaction from that account.
    #[init]
    #[private]
    pub fn new() -> Self {
        let metadata = FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: "Test USDC (testnet, no value)".to_string(),
            symbol: "tUSDC".to_string(),
            icon: Some(ICON.to_string()),
            reference: None,
            reference_hash: None,
            decimals: DECIMALS,
        };
        metadata.assert_valid();
        Self {
            token: FungibleToken::new(StorageKey::Accounts),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
        }
    }

    /// Creates `amount` new tokens for `account_id` (default: the caller). Anyone can call
    /// it; this is the faucet. At most `MAX_MINT` per call.
    ///
    /// If the account is not registered, attach at least `storage_balance_bounds().min`
    /// and the call registers it; the rest of the deposit is sent back. A registered
    /// account gets any attached deposit back.
    #[payable]
    pub fn mint(&mut self, account_id: Option<AccountId>, amount: U128) -> U128 {
        let amount = amount.0;
        require!(amount > 0, "mint: amount must be positive");
        require!(
            amount <= MAX_MINT,
            format!("mint: at most {MAX_MINT} per call")
        );
        let account_id = account_id.unwrap_or_else(env::predecessor_account_id);
        let mut refund = env::attached_deposit();
        if !self.token.accounts.contains_key(&account_id) {
            let min = self.token.storage_balance_bounds().min;
            require!(
                refund >= min,
                format!(
                    "mint: {account_id} is not registered; attach at least {} yoctoNEAR",
                    min.as_yoctonear()
                )
            );
            self.token.internal_register_account(&account_id);
            refund = refund.saturating_sub(min);
        }
        self.token.internal_deposit(&account_id, amount);
        FtMint {
            owner_id: &account_id,
            amount: U128(amount),
            memo: Some("faucet"),
        }
        .emit();
        if refund > NearToken::from_yoctonear(0) {
            Promise::new(env::predecessor_account_id())
                .transfer(refund)
                .detach();
        }
        U128(self.token.ft_balance_of(account_id).0)
    }
}

#[near]
impl FungibleTokenCore for Contract {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        self.token.ft_transfer(receiver_id, amount, memo)
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.token.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for Contract {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        let (used_amount, burned_amount) =
            self.token
                .internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        if burned_amount > 0 {
            log!("Account @{} burned {}", sender_id, burned_amount);
        }
        used_amount.into()
    }
}

#[near]
impl StorageManagement for Contract {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        if let Some((account_id, balance)) = self.token.internal_storage_unregister(force) {
            log!("Closed @{} with {}", account_id, balance);
            true
        } else {
            false
        }
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.get().expect("metadata is set in new")
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
    use near_sdk::{testing_env, Gas};

    fn acc(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    fn ctx(predecessor: &str, deposit: NearToken) -> VMContextBuilder {
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("usdc.testnet"))
            .predecessor_account_id(acc(predecessor))
            .signer_account_id(acc(predecessor))
            .attached_deposit(deposit)
            .prepaid_gas(Gas::from_tgas(300))
            .account_balance(NearToken::from_near(10));
        b
    }

    fn init() -> Contract {
        testing_env!(ctx("usdc.testnet", NearToken::from_yoctonear(0)).build());
        Contract::new()
    }

    fn min_storage(c: &Contract) -> NearToken {
        c.storage_balance_bounds().min
    }

    #[test]
    fn metadata_has_six_decimals_and_no_supply() {
        let c = init();
        let m = c.ft_metadata();
        assert_eq!(m.decimals, 6);
        assert_eq!(m.symbol, "tUSDC");
        assert_eq!(m.spec, "ft-1.0.0");
        assert_eq!(c.ft_total_supply().0, 0);
    }

    #[test]
    fn mint_registers_with_deposit_and_refunds_the_rest() {
        let mut c = init();
        let min = min_storage(&c);
        let attached = min.saturating_add(NearToken::from_millinear(5));
        testing_env!(ctx("alice.testnet", attached).build());
        let bal = c.mint(None, U128(5_000_000));
        assert_eq!(bal.0, 5_000_000);
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, 5_000_000);
        assert_eq!(c.ft_total_supply().0, 5_000_000);
        assert!(c.storage_balance_of(acc("alice.testnet")).is_some());
        let logs = get_logs();
        assert!(
            logs.iter().any(|l| l.contains("\"event\":\"ft_mint\"")),
            "{logs:?}"
        );
        // The refund of the excess deposit is one transfer receipt to the caller.
        let receipts = get_created_receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].receiver_id, acc("alice.testnet"));
        let s = format!("{:?}", receipts[0].actions);
        assert!(
            s.contains(&NearToken::from_millinear(5).as_yoctonear().to_string()),
            "{s}"
        );
    }

    #[test]
    fn mint_for_another_account() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(Some(acc("maker.testnet")), U128(MAX_MINT));
        assert_eq!(c.ft_balance_of(acc("maker.testnet")).0, MAX_MINT);
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, 0);
        assert!(get_created_receipts().is_empty(), "no refund when exact");
    }

    #[test]
    fn mint_to_registered_account_returns_the_deposit() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.storage_deposit(None, None);
        testing_env!(ctx("alice.testnet", NearToken::from_millinear(1)).build());
        assert_eq!(c.mint(None, U128(7)).0, 7);
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(0)).build());
        assert_eq!(c.mint(None, U128(3)).0, 10);
        assert_eq!(c.ft_total_supply().0, 10);
    }

    #[test]
    #[should_panic(expected = "is not registered; attach at least")]
    fn mint_to_unregistered_account_needs_storage() {
        let mut c = init();
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(1)).build());
        c.mint(None, U128(1));
    }

    #[test]
    #[should_panic(expected = "mint: at most 10000000000 per call")]
    fn mint_is_capped() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(MAX_MINT + 1));
    }

    #[test]
    #[should_panic(expected = "mint: amount must be positive")]
    fn mint_zero_is_rejected() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(0));
    }

    #[test]
    fn transfer_moves_tokens_between_registered_accounts() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(100));
        testing_env!(ctx("bob.testnet", min).build());
        c.storage_deposit(None, None);
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(1)).build());
        c.ft_transfer(acc("bob.testnet"), U128(40), None);
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, 60);
        assert_eq!(c.ft_balance_of(acc("bob.testnet")).0, 40);
        assert_eq!(c.ft_total_supply().0, 100);
    }

    #[test]
    #[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
    fn transfer_needs_one_yocto() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(100));
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(0)).build());
        c.ft_transfer(acc("bob.testnet"), U128(1), None);
    }

    #[test]
    #[should_panic(expected = "The account bob.testnet is not registered")]
    fn transfer_to_unregistered_receiver_fails() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(100));
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(1)).build());
        c.ft_transfer(acc("bob.testnet"), U128(1), None);
    }

    #[test]
    fn transfer_call_debits_and_schedules_receiver_then_resolve() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(100));
        testing_env!(ctx("escrow.testnet", min).build());
        c.storage_deposit(None, None);
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(1)).build());
        let _ = c.ft_transfer_call(acc("escrow.testnet"), U128(30), None, "{}".into());
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, 70);
        assert_eq!(c.ft_balance_of(acc("escrow.testnet")).0, 30);
        let methods: Vec<String> = get_created_receipts()
            .iter()
            .flat_map(|r| r.actions.iter())
            .filter_map(|a| match a {
                near_sdk::mock::MockAction::FunctionCallWeight { method_name, .. } => {
                    Some(String::from_utf8(method_name.clone()).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(methods, ["ft_on_transfer", "ft_resolve_transfer"]);
    }

    #[test]
    fn resolve_returns_unused_tokens_to_sender() {
        let mut c = init();
        let min = min_storage(&c);
        testing_env!(ctx("alice.testnet", min).build());
        c.mint(None, U128(100));
        testing_env!(ctx("escrow.testnet", min).build());
        c.storage_deposit(None, None);
        testing_env!(ctx("alice.testnet", NearToken::from_yoctonear(1)).build());
        let _ = c.ft_transfer_call(acc("escrow.testnet"), U128(30), None, "{}".into());
        // The receiver's ft_on_transfer returned "10" unused.
        let b = ctx("usdc.testnet", NearToken::from_yoctonear(0));
        testing_env!(
            b.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            Default::default(),
            vec![near_sdk::PromiseResult::Successful(b"\"10\"".to_vec())],
        );
        let used = c.ft_resolve_transfer(acc("alice.testnet"), acc("escrow.testnet"), U128(30));
        assert_eq!(used.0, 20);
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, 80);
        assert_eq!(c.ft_balance_of(acc("escrow.testnet")).0, 20);
    }
}
