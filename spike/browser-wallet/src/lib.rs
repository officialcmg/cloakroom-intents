//! The Cloakroom light wallet, built for a browser Web Worker (wasm32) and natively.
//!
//! - `wallet`: copied from `node/src/wallet.rs`, adapted for the browser.
//! - `keys`: copied unchanged from `node/src/keys.rs`.
//! - `backend`: Zebra JSON-RPC and lightwalletd gRPC-web over `net::Transport`.
//! - `net`: blocking POST (reqwest natively, synchronous XMLHttpRequest in a worker).
//! - `web`: the wasm-bindgen interface the worker calls.

pub mod backend;
pub mod keys;
pub mod net;
pub mod wallet;
#[cfg(target_arch = "wasm32")]
pub mod web;

use anyhow::{anyhow, Result};
use orchard::keys::{FullViewingKey, OutgoingViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use rand::{rand_core::UnwrapErr, rngs::SysRng};

#[allow(non_upper_case_globals)]
pub const OsRng: UnwrapErr<SysRng> = UnwrapErr(SysRng);

/// The party's own shielded wallet key and its derived keys (from `node/src/common.rs`,
/// without the file storage: the caller stores `sk_bytes()` where its platform stores data).
pub struct Me {
    pub sk: SpendingKey,
    pub fvk: FullViewingKey,
    pub ask: SpendAuthorizingKey,
    pub ovk: OutgoingViewingKey,
}

impl Me {
    pub fn generate() -> Me {
        loop {
            let mut s = [0u8; 32];
            rand::Rng::fill_bytes(&mut { OsRng }, &mut s);
            if let Ok(me) = Me::from_bytes(s) {
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
    /// The same JSON file format as `node/` (`{"sk": "<hex>"}`).
    pub fn from_wallet_json(s: &str) -> Result<Me> {
        let v: serde_json::Value = serde_json::from_str(s)?;
        Me::from_hex(v["sk"].as_str().ok_or_else(|| anyhow!("wallet.json has no sk"))?)
    }
    pub fn sk_bytes(&self) -> [u8; 32] {
        *self.sk.to_bytes()
    }
    pub fn ua(&self) -> String {
        wallet::encode_ua(&self.fvk.address_at(0u32, Scope::External))
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
    use crate::{
        backend::Rpc,
        net::{Response, Transport},
        wallet::{Backend, CoverPolicy, CoverWait, Net, Wallet},
    };

    struct NoNet;
    impl Transport for NoNet {
        fn post(&self, _: &str, _: &[(&str, &str)], _: &[u8]) -> Result<Response> {
            Err(anyhow!("no network in this test"))
        }
    }

    fn wallet() -> Wallet {
        Wallet::new(Backend::Zebra(Rpc::new(Box::new(NoNet), "http://unused")), Net::Local(wallet::regtest_params(Some(2))))
    }

    #[test]
    fn key_round_trip_and_ua() {
        let me = Me::generate();
        let again = Me::from_hex(&hex::encode(me.sk_bytes())).unwrap();
        assert_eq!(me.ua(), again.ua());
        assert!(me.ua().starts_with("uregtest1"));
        assert_eq!(wallet::encode_ua(&wallet::decode_ua(&me.ua()).unwrap()), me.ua());
        let json = format!("{{\"sk\": \"{}\"}}", hex::encode(me.sk_bytes()));
        assert_eq!(Me::from_wallet_json(&json).unwrap().ua(), me.ua());
    }

    #[test]
    fn memo_text_stops_at_zero_and_falls_back_to_hex() {
        let mut m = [0u8; 512];
        m[..5].copy_from_slice(b"hello");
        assert_eq!(memo_text(&m), "hello");
        m[0] = 0xF6;
        assert!(memo_text(&m).starts_with("hex:f6"));
        let full = [b'x'; 512];
        assert_eq!(memo_text(&full).len(), 512);
    }

    #[test]
    fn cover_wait_polls_without_sleeping() {
        let w = wallet();
        // no policy: go at once
        let mut c = CoverWait::new(CoverPolicy::default(), 0, None, 100);
        assert_eq!(c.poll(&w, 100).as_deref(), Some("no cover delay configured"));
        // fixed 10 s wait, k = 0: not before 110, then go
        let p = CoverPolicy { min_secs: 10, max_secs: 10, k: 0 };
        let mut c = CoverWait::new(p, 0, None, 100);
        assert!(c.poll(&w, 105).is_none());
        assert!(c.poll(&w, 110).unwrap().starts_with("cover: waited 10s over 1"));
        // k = 1 never met (no look-alikes): redraws until the deadline cuts it short
        let p = CoverPolicy { min_secs: 1, max_secs: 3, k: 1 };
        let mut c = CoverWait::new(p, 0, Some(150), 100);
        let mut now = 100;
        let out = loop {
            if let Some(o) = c.poll(&w, now) {
                break o;
            }
            now += 1;
        };
        assert_eq!(now, 150);
        assert!(out.starts_with("cover cut short by the protocol deadline after 50s"), "{out}");
    }
}
