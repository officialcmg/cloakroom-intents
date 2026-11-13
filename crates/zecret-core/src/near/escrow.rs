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

/// Gas attached to each call. `claim` and `refund` check a Pallas scalar multiplication
/// (about 16 Tgas measured); `commit_claim` checks one ed25519 signature (about 2 Tgas).
/// The taker's stipend must cover the prepaid gas of its calls.
pub const GAS_OPEN: u64 = 100 * TGAS;
pub const GAS_SET_READY: u64 = 10 * TGAS;
pub const GAS_COMMIT: u64 = 15 * TGAS;
pub const GAS_REVEAL: u64 = 40 * TGAS;
pub const GAS_WITHDRAW: u64 = 120 * TGAS;
pub const GAS_STORAGE: u64 = 10 * TGAS;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
pub enum SwapState {
    Open,
    Ready,
    Locked,
    Claimed,
    Refunded,
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Credit {
    pub direct: u128,
    pub intents: u128,
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

    pub fn credit(&self, owner: &str, at: &At) -> Result<Credit> {
        let v = self.rpc.view(&self.escrow, "get_credit", &json!({"token": self.token, "owner": owner}), at)?;
        let get = |k: &str| -> Result<u128> { Ok(v[k].as_str().ok_or_else(|| anyhow!("credit without {k}"))?.parse()?) };
        Ok(Credit { direct: get("direct")?, intents: get("intents")? })
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

    /// `open`: `ft_transfer_call` on the token with the swap terms in `msg`.
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
        let args = json!({"receiver_id": self.escrow, "amount": amount.to_string(), "msg": terms.to_string()});
        (self.token.clone(), vec![Action::call("ft_transfer_call", &args, GAS_OPEN, 1)])
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
        self.call("claim", json!({"id": hex::encode(id), "a": hex::encode(a)}), GAS_REVEAL)
    }

    pub fn refund(&self, id: &[u8; 32], b: &[u8; 32]) -> (String, Vec<Action>) {
        self.call("refund", json!({"id": hex::encode(id), "b": hex::encode(b)}), GAS_REVEAL)
    }

    pub fn withdraw(&self, owner: &str) -> (String, Vec<Action>) {
        self.call("withdraw", json!({"token": self.token, "owner": owner}), GAS_WITHDRAW)
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
            "locked_at": 5, "commitment": "cc", "a": null, "b": null
        });
        let s: SwapView = serde_json::from_value(v).unwrap();
        assert_eq!(s.amount, 3_000_000);
        assert_eq!(s.state, SwapState::Locked);
        assert_eq!(s.locked_at, Some(5));
    }
}
