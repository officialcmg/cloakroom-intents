//! Zecret shared core.
//!
//! Everything both parties of a swap need, with no I/O of its own: it builds natively (the
//! maker daemon and the reference taker in `zecret-node`) and for `wasm32-unknown-unknown`
//! (the taker engine in a browser Web Worker). The outside world is reached through three
//! small interfaces; see `docs/ENGINE.md`:
//!
//! - [`net::Transport`]: one blocking HTTP POST. Zebra JSON-RPC, lightwalletd (gRPC-web)
//!   and NEAR JSON-RPC are thin clients on top of it.
//! - [`store::Store`]: a key-value store for the records a party must not lose.
//! - time: the engines never read a clock and never sleep. The caller passes the current
//!   Unix time to `tick(now)` and calls again when the returned [`engine::Tick`] says.
//!   Protocol deadlines use NEAR block time, read from final blocks.
//!
//! Modules:
//! - [`keys`]: Pallas key shares, proofs of knowledge, the joint Ironwood key.
//! - [`msg`]: the three encrypted memo messages (request, quote, acceptance).
//! - [`wallet`], [`zcash`]: the Ironwood light wallet and its two backends.
//! - [`near`]: NEAR transactions (borsh, ed25519), JSON-RPC and the escrow calls.
//! - [`config`]: protocol timing bounds and each party's settings.
//! - [`taker`], [`maker`]: the two state machines and the wallet rules of the paper.

pub mod config;
pub mod engine;
pub mod keys;
pub mod maker;
pub mod msg;
pub mod near;
pub mod net;
pub mod store;
pub mod taker;
pub mod wallet;
pub mod zcash;

use anyhow::{anyhow, Result};
use orchard::keys::{FullViewingKey, OutgoingViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use rand::{rand_core::UnwrapErr, rngs::SysRng};

/// The operating system's (or the browser's) random source.
#[allow(non_upper_case_globals)]
pub const OsRng: UnwrapErr<SysRng> = UnwrapErr(SysRng);

/// `N` random bytes from [`OsRng`].
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::Rng::fill_bytes(&mut { OsRng }, &mut b);
    b
}

/// A party's own shielded wallet key and its derived keys. The caller stores
/// [`Me::sk_bytes`] where its platform stores secrets.
pub struct Me {
    pub sk: SpendingKey,
    pub fvk: FullViewingKey,
    pub ask: SpendAuthorizingKey,
    pub ovk: OutgoingViewingKey,
}

impl Me {
    pub fn generate() -> Me {
        loop {
            if let Ok(me) = Me::from_bytes(random_bytes()) {
                return me;
            }
        }
    }
    pub fn from_bytes(b: [u8; 32]) -> Result<Me> {
        let sk = Option::<SpendingKey>::from(SpendingKey::from_bytes(b)).ok_or_else(|| anyhow!("bad sk"))?;
        let fvk = FullViewingKey::from(&sk);
        let ovk = fvk.to_ovk(Scope::External);
        Ok(Me { ask: SpendAuthorizingKey::from(&sk), fvk, ovk, sk })
    }
    pub fn from_hex(h: &str) -> Result<Me> {
        let b: [u8; 32] = hex::decode(h.trim())?.try_into().map_err(|_| anyhow!("sk len"))?;
        Me::from_bytes(b)
    }
    pub fn sk_bytes(&self) -> [u8; 32] {
        *self.sk.to_bytes()
    }
    pub fn ua(&self) -> String {
        wallet::encode_ua(&self.address())
    }
    pub fn address(&self) -> orchard::Address {
        self.fvk.address_at(0u32, Scope::External)
    }
    pub fn change(&self) -> orchard::Address {
        self.fvk.address_at(0u32, Scope::Internal)
    }
}

/// Memo bytes as text: UTF-8 up to the first zero byte (ZIP 302), or hex if not text.
pub fn memo_text(m: &[u8; 512]) -> String {
    if m[0] <= 0xF4 {
        let end = m.iter().position(|b| *b == 0).unwrap_or(512);
        if let Ok(s) = std::str::from_utf8(&m[..end]) {
            return s.to_string();
        }
    }
    format!("hex:{}", hex::encode(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_round_trip_and_ua() {
        let me = Me::generate();
        let again = Me::from_hex(&hex::encode(me.sk_bytes())).unwrap();
        assert_eq!(me.ua(), again.ua());
        assert!(me.ua().starts_with("uregtest1"));
        assert_eq!(wallet::encode_ua(&wallet::decode_ua(&me.ua()).unwrap()), me.ua());
    }

    #[test]
    fn memo_text_stops_at_zero_and_falls_back_to_hex() {
        let mut m = [0u8; 512];
        m[..5].copy_from_slice(b"hello");
        assert_eq!(memo_text(&m), "hello");
        m[0] = 0xF6;
        assert!(memo_text(&m).starts_with("hex:f6"));
    }
}
