//! Calls and views of the Zecret escrow contract (`contracts/near-escrow`), the NEP-141
//! token and `intents.near`.
//!
//! The hashes the escrow computes (swap id, commitment, commit message) are computed here
//! too, so a client never has to trust an RPC server for the bytes it signs.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::rpc::{At, NearRpc};
use super::tx::{sha256, Action, TGAS};

/// Domains of the escrow's hashes (`contracts/near-escrow/src/lib.rs`).
pub const DOMAIN_SWAP_ID: &[u8] = b"near-escrow/v1/swap-id";
pub const DOMAIN_COMMITMENT: &[u8] = b"near-escrow/v1/commitment";
pub const DOMAIN_COMMIT_CLAIM: &[u8] = b"near-escrow/v1/commit-claim";

/// Gas attached to each call (measured in `contracts/near-escrow/tests/sandbox.rs` and
/// docs/experiments/light-payout.md). `claim` and `refund` check a Pallas scalar
/// multiplication (16.6 Tgas burnt); `commit_claim` checks one ed25519 signature.
///
/// A signer must hold its attached gas at `max(gas price, min_gas_purchase_price)` =
/// 10^9 yoctoNEAR per gas today, 10 times the gas price: 25 Tgas needs 0.026 NEAR.
/// The taker's key sends `commit_claim` (15), `claim` (25) and `withdraw` (25) from the
/// stipend. The largest balance NEAR checks in that sequence is 0.0278 NEAR.
pub const GAS_OPEN: u64 = 100 * TGAS;
/// The maker's deposit of plain tokens into its own Verifier account.
pub const GAS_DEPOSIT: u64 = 100 * TGAS;
pub const GAS_SET_READY: u64 = 10 * TGAS;
pub const GAS_COMMIT: u64 = 15 * TGAS;
/// `claim`: 17 Tgas is the measured minimum; with 25 the payout stays Due (it needs 34).
pub const GAS_CLAIM: u64 = 25 * TGAS;
/// `refund` with its payout in the same call (34 Tgas minimum).
pub const GAS_REFUND: u64 = 50 * TGAS;
/// `withdraw(id)`: one `mt_transfer` inside the Verifier. 19 Tgas is the measured minimum.
pub const GAS_WITHDRAW: u64 = 25 * TGAS;
pub const GAS_STORAGE: u64 = 10 * TGAS;
/// The price at which NEAR sells attached gas today (`min_gas_purchase_price`, protocol
/// 85 and later). A signer must hold `attached gas × this` before NEAR accepts a call.
pub const GAS_PURCHASE_PRICE: u128 = 1_000_000_000;

/// `sha256(DOMAIN_SWAP_ID ‖ len(maker) as u8 ‖ maker ‖ salt)`.
pub fn swap_id(maker: &str, salt: &[u8; 32]) -> [u8; 32] {
    let mut b = DOMAIN_SWAP_ID.to_vec();
    b.push(maker.len() as u8);
    b.extend_from_slice(maker.as_bytes());
    b.extend_from_slice(salt);
    sha256(&b)
}

/// `sha256(DOMAIN_COMMITMENT ‖ id ‖ a)`, with `a` the 32-byte little-endian scalar.
pub fn commitment(id: &[u8; 32], a: &[u8; 32]) -> [u8; 32] {
    sha256(&[DOMAIN_COMMITMENT, id, a].concat())
}

/// The bytes the taker key signs for `commit_claim`:
/// `DOMAIN_COMMIT_CLAIM ‖ len(escrow) as u8 ‖ escrow ‖ id ‖ commitment`.
pub fn commit_message(escrow: &str, id: &[u8; 32], commitment: &[u8; 32]) -> Vec<u8> {
    let mut b = DOMAIN_COMMIT_CLAIM.to_vec();
    b.push(escrow.len() as u8);
    b.extend_from_slice(escrow.as_bytes());
    b.extend_from_slice(id);
    b.extend_from_slice(commitment);
    b
}

/// Checks that the escrow's tokens back everything it owes in `token`, at block `at`: its
/// balance of `nep141:<token>` in the Verifier is at least its `get_owed(token)`. Every
/// swap's amount is in `get_owed` until its payout succeeds, and only the escrow can move
/// tokens out of its Verifier account, so a taker who sees this at a final block knows
/// that her swap's tokens are there and are not promised to another swap. Returns
/// (balance, owed).
pub fn check_backing(rpc: &NearRpc, escrow: &str, token: &str, intents: &str, at: &At) -> Result<(u128, u128)> {
    let owed = rpc.view(escrow, "get_owed", &json!({"token": token}), at)?;
    let owed: u128 = owed.as_str().ok_or_else(|| anyhow!("get_owed: {owed}"))?.parse()?;
    let bal = rpc.view(intents, "mt_balance_of", &json!({"account_id": escrow, "token_id": format!("nep141:{token}")}), at)?;
    let bal: u128 = bal.as_str().ok_or_else(|| anyhow!("mt_balance_of: {bal}"))?.parse()?;
    if bal < owed {
        anyhow::bail!("escrow {escrow} holds {bal} of {token} in the Verifier but owes {owed}");
    }
    Ok((bal, owed))
}

/// Checks that the escrow cannot change under the taker and pays out where it should: at
/// block `at`, the escrow account runs exactly the code with hash `code_hash` (base58),
/// has no access keys (so nobody can redeploy it), and its configured Intents account is
/// `intents`. A taker runs this before it sends ZEC (security review 1, NV-01).
pub fn check_escrow(rpc: &NearRpc, escrow: &str, code_hash: &str, intents: &str, at: &At) -> Result<()> {
    let acct = rpc.account(escrow, at)?.ok_or_else(|| anyhow!("escrow account {escrow} does not exist"))?;
    if acct.code_hash != code_hash {
        anyhow::bail!("escrow {escrow} runs code {}, expected {code_hash}", acct.code_hash);
    }
    let keys = rpc.access_keys(escrow, at)?;
    if !keys.is_empty() {
        anyhow::bail!("escrow {escrow} has {} access key(s) ({}); its code could be replaced", keys.len(), keys.join(", "));
    }
    let v = rpc.view(escrow, "get_intents_account", &json!({}), at)?;
    if v.as_str() != Some(intents) {
        anyhow::bail!("escrow {escrow} pays takers into {v}, expected {intents}");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
pub enum SwapState {
    Open,
    Ready,
    Locked,
    Claimed,
    Refunded,
}

/// Where the tokens of a swap are after it ended (`get_swap(id).payout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Payout {
    /// The swap has not ended.
    None,
    /// Ended; the tokens are still in the escrow's Verifier account. `withdraw(id)` pays.
    Due,
    /// The payout's `mt_transfer` is in flight.
    Sending,
    /// Paid to the recipient's Verifier account. Final.
    Paid,
}

fn de_u128<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u128, D::Error> {
    let s = String::deserialize(d)?;
    s.parse().map_err(serde::de::Error::custom)
}

/// One swap as `get_swap` returns it. Byte fields are lower-case hex.
#[derive(Clone, Debug, Deserialize)]
pub struct SwapView {
    pub id: String,
    pub maker: String,
    pub token: String,
    #[serde(deserialize_with = "de_u128")]
    pub amount: u128,
    pub taker_pk: String,
    pub taker_account: String,
    pub a_pub: String,
    pub b_pub: String,
    pub t0: u64,
    pub t1: u64,
    pub t2: u64,
    pub reveal_window: u64,
    pub state: SwapState,
    pub locked_at: Option<u64>,
    pub commitment: Option<String>,
    /// The taker's share, once claimed.
    pub a: Option<String>,
    /// The maker's share, once refunded.
    pub b: Option<String>,
    pub payout: Payout,
    /// The taker's account after a claim, the maker after a refund.
    pub recipient: Option<String>,
}

/// The escrow, its token and the Intents contract, as one client.
pub struct Escrow<'a> {
    pub rpc: &'a NearRpc,
    pub escrow: String,
    pub token: String,
    pub intents: String,
}

impl Escrow<'_> {
    pub fn get_swap(&self, id: &[u8; 32], at: &At) -> Result<Option<SwapView>> {
        let v = self.rpc.view(&self.escrow, "get_swap", &json!({"id": hex::encode(id)}), at)?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(v)?))
    }

    /// The sum of the amounts of the escrow's swaps in the token whose payout is not Paid.
    pub fn owed(&self, at: &At) -> Result<u128> {
        let v = self.rpc.view(&self.escrow, "get_owed", &json!({"token": self.token}), at)?;
        Ok(v.as_str().ok_or_else(|| anyhow!("get_owed"))?.parse()?)
    }

    pub fn storage_funds(&self, owner: &str, at: &At) -> Result<u128> {
        let v = self.rpc.view(&self.escrow, "get_storage_funds", &json!({"account_id": owner, "token": self.token}), at)?;
        Ok(v.as_str().ok_or_else(|| anyhow!("storage funds"))?.parse()?)
    }

    /// The token balance of `owner` inside `intents.near` (`mt_balance_of`, token id
    /// `nep141:<token>`).
    pub fn intents_balance(&self, owner: &str, at: &At) -> Result<u128> {
        let v = self.rpc.view(&self.intents, "mt_balance_of", &json!({"account_id": owner, "token_id": format!("nep141:{}", self.token)}), at)?;
        Ok(v.as_str().ok_or_else(|| anyhow!("mt_balance_of"))?.parse()?)
    }

    pub fn ft_balance(&self, owner: &str, at: &At) -> Result<u128> {
        let v = self.rpc.view(&self.token, "ft_balance_of", &json!({"account_id": owner}), at)?;
        Ok(v.as_str().ok_or_else(|| anyhow!("ft_balance_of"))?.parse()?)
    }

    // ------------------------------------------------------------------ actions
    // Each returns (receiver, actions) for `NearRpc::sign`.

    /// `open`: `mt_transfer_call` on the Verifier, from the maker's Verifier account to the
    /// escrow, with the swap terms in `msg`. The escrow's `mt_on_transfer` stores the swap
    /// only after the Verifier moved the tokens; on bad terms the Verifier gives them back.
    #[allow(clippy::too_many_arguments)]
    pub fn open(&self, amount: u128, salt: &[u8; 32], taker_pk: &[u8; 32], a_pub: &[u8; 32], b_pub: &[u8; 32], t0: u64, t1: u64, t2: u64, reveal_window: u64) -> (String, Vec<Action>) {
        let terms = json!({
            "salt": hex::encode(salt),
            "taker_pk": hex::encode(taker_pk),
            "a": hex::encode(a_pub),
            "b": hex::encode(b_pub),
            "t0": t0,
            "t1": t1,
            "t2": t2,
            "reveal_window": reveal_window,
        });
        let args = json!({"receiver_id": self.escrow, "token_id": format!("nep141:{}", self.token), "amount": amount.to_string(), "msg": terms.to_string()});
        (self.intents.clone(), vec![Action::call("mt_transfer_call", &args, GAS_OPEN, 1)])
    }

    /// The maker's deposit of `amount` plain tokens into its own Verifier account
    /// (`ft_transfer_call` on the token, empty `msg`). With USDC the call's result can be
    /// a failure after the Verifier credited the maker (docs/security/nv02-usdc.md), so a
    /// caller reads its Verifier balance afterwards instead of trusting the result.
    pub fn deposit(&self, amount: u128) -> (String, Vec<Action>) {
        let args = json!({"receiver_id": self.intents, "amount": amount.to_string(), "msg": ""});
        (self.token.clone(), vec![Action::call("ft_transfer_call", &args, GAS_DEPOSIT, 1)])
    }

    pub fn add_storage_funds(&self, deposit: u128) -> (String, Vec<Action>) {
        (self.escrow.clone(), vec![Action::call("add_storage_funds", &json!({"token": self.token}), GAS_STORAGE, deposit)])
    }

    pub fn set_ready(&self, id: &[u8; 32]) -> (String, Vec<Action>) {
        self.call("set_ready", json!({"id": hex::encode(id)}), GAS_SET_READY)
    }

    pub fn commit_claim(&self, id: &[u8; 32], commitment: &[u8; 32], signature: &[u8; 64]) -> (String, Vec<Action>) {
        self.call("commit_claim", json!({"id": hex::encode(id), "commitment": hex::encode(commitment), "signature": hex::encode(signature)}), GAS_COMMIT)
    }

    pub fn claim(&self, id: &[u8; 32], a: &[u8; 32]) -> (String, Vec<Action>) {
        self.call("claim", json!({"id": hex::encode(id), "a": hex::encode(a)}), GAS_CLAIM)
    }

    pub fn refund(&self, id: &[u8; 32], b: &[u8; 32]) -> (String, Vec<Action>) {
        self.call("refund", json!({"id": hex::encode(id), "b": hex::encode(b)}), GAS_REFUND)
    }

    /// `withdraw(id)`: sends a Due payout (one `mt_transfer` inside the Verifier to the
    /// swap's recipient). Anyone may send it; the recipient is fixed by the swap.
    pub fn withdraw(&self, id: &[u8; 32]) -> (String, Vec<Action>) {
        self.call("withdraw", json!({"id": hex::encode(id)}), GAS_WITHDRAW)
    }

    fn call(&self, method: &str, args: Value, gas: u64) -> (String, Vec<Action>) {
        (self.escrow.clone(), vec![Action::call(method, &args, gas, 0)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_follow_the_contract_layout() {
        let salt = [7u8; 32];
        let mut b = b"near-escrow/v1/swap-id".to_vec();
        b.push(10);
        b.extend_from_slice(b"maker.near");
        b.extend_from_slice(&salt);
        assert_eq!(swap_id("maker.near", &salt), sha256(&b));
        assert_ne!(swap_id("maker.near", &salt), swap_id("maker2.near", &salt));
        let id = [1u8; 32];
        let a = [2u8; 32];
        assert_eq!(commitment(&id, &a), sha256(&[b"near-escrow/v1/commitment".as_slice(), &id, &a].concat()));
        let m = commit_message("e.near", &id, &a);
        assert_eq!(m.len(), 27 + 1 + 6 + 64);
        assert_eq!(m[27], 6);
    }

    #[test]
    fn swap_view_parses_contract_json() {
        let v = json!({
            "id": "00", "maker": "m.near", "token": "wrap.near", "amount": "3000000",
            "taker_pk": "ab", "taker_account": "ab", "a_pub": "01", "b_pub": "02",
            "t0": 1, "t1": 2, "t2": 3, "reveal_window": 4, "state": "Locked",
            "locked_at": 5, "commitment": "cc", "a": null, "b": null,
            "payout": "none", "recipient": null
        });
        let s: SwapView = serde_json::from_value(v).unwrap();
        assert_eq!(s.amount, 3_000_000);
        assert_eq!(s.state, SwapState::Locked);
        assert_eq!(s.locked_at, Some(5));
        assert_eq!(s.payout, Payout::None);
        for (j, p) in [("due", Payout::Due), ("sending", Payout::Sending), ("paid", Payout::Paid)] {
            assert_eq!(serde_json::from_value::<Payout>(json!(j)).unwrap(), p);
        }
    }
}
