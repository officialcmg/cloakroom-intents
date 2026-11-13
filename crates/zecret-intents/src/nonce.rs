//! Verifier nonces.
//!
//! A nonce is 32 bytes, sent as base64. The Verifier keeps one bit for each used nonce
//! of each account, so a nonce works once. Rules, from `contracts/defuse/core/src/
//! engine/mod.rs` and `nonce/` in `near/intents` (commit fce0936, 2026-11-10):
//!
//! - A **versioned** nonce starts with the magic bytes `5628f6c6`, followed by the
//!   borsh encoding of `VersionedNonce::V1(SaltedNonce { salt, ExpirableNonce {
//!   deadline, nonce } })`: one variant byte `0`, a 4-byte salt, the deadline as `i64`
//!   nanoseconds little-endian, and 15 random bytes. The Verifier accepts it only if
//!   the salt is valid (`is_valid_salt`; `current_salt` gives the current one), the
//!   intent deadline is not later than the nonce deadline, and the nonce deadline has
//!   not passed. Expired versioned nonces can be cleaned up by anyone.
//! - Any other 32 bytes is a **legacy** nonce. The Verifier still accepts it, but the
//!   source says legacy nonces "will be prohibited in the near future".
//!
//! 1Click's `/v0/generate-intent` returns versioned nonces whose deadline equals the
//! intent deadline. This crate does the same.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::error::{Error, Result};
use crate::time::{self, Time};

pub const VERSIONED_MAGIC: [u8; 4] = [0x56, 0x28, 0xf6, 0xc6];

/// A Verifier salt: 4 bytes, shown as 8 hex characters (the JSON result of
/// `current_salt`, e.g. `"252812b3"` on `intents.near` on 2026-11-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Salt(pub [u8; 4]);

impl Salt {
    pub fn from_hex(s: &str) -> Result<Self> {
        let mut out = [0u8; 4];
        hex::decode_to_slice(s, &mut out)
            .map_err(|e| Error::Parse(format!("salt {s:?}: {e}")))?;
        Ok(Self(out))
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

/// A 32-byte nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nonce(pub [u8; 32]);

/// The parts of a versioned nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionedNonce {
    pub salt: Salt,
    pub deadline: Time,
    pub random: [u8; 15],
}

impl Nonce {
    /// A versioned nonce (V1).
    pub fn versioned(salt: Salt, deadline: Time, random: [u8; 15]) -> Result<Self> {
        let mut n = [0u8; 32];
        n[..4].copy_from_slice(&VERSIONED_MAGIC);
        n[4] = 0; // VersionedNonce::V1
        n[5..9].copy_from_slice(&salt.0);
        n[9..17].copy_from_slice(&time::unix_nanos(deadline)?.to_le_bytes());
        n[17..].copy_from_slice(&random);
        Ok(Self(n))
    }

    /// A versioned nonce with random bytes from the platform's random source.
    pub fn versioned_random(salt: Salt, deadline: Time) -> Result<Self> {
        let mut random = [0u8; 15];
        crate::random_bytes(&mut random)?;
        Self::versioned(salt, deadline, random)
    }

    /// A legacy nonce: 32 bytes that do not start with the versioned magic.
    pub fn legacy(bytes: [u8; 32]) -> Result<Self> {
        if bytes[..4] == VERSIONED_MAGIC {
            return Err(Error::Invalid(
                "legacy nonce must not start with the versioned magic".into(),
            ));
        }
        Ok(Self(bytes))
    }

    /// The parts of a versioned nonce, or `None` for a legacy nonce. A nonce that
    /// starts with the magic but does not decode is also `None`; the Verifier treats it
    /// as legacy too (`VersionedNonce::maybe_from` returns `None`).
    pub fn as_versioned(&self) -> Option<VersionedNonce> {
        if self.0[..4] != VERSIONED_MAGIC || self.0[4] != 0 {
            return None;
        }
        let mut salt = [0u8; 4];
        salt.copy_from_slice(&self.0[5..9]);
        let mut nanos = [0u8; 8];
        nanos.copy_from_slice(&self.0[9..17]);
        let mut random = [0u8; 15];
        random.copy_from_slice(&self.0[17..]);
        Some(VersionedNonce {
            salt: Salt(salt),
            deadline: time::from_unix_nanos(i64::from_le_bytes(nanos)),
            random,
        })
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }

    pub fn from_base64(s: &str) -> Result<Self> {
        let v = STANDARD
            .decode(s)
            .map_err(|e| Error::Parse(format!("nonce {s:?}: {e}")))?;
        let n: [u8; 32] = v
            .try_into()
            .map_err(|v: Vec<u8>| Error::Parse(format!("nonce has {} bytes, not 32", v.len())))?;
        Ok(Self(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_nonce_from_live_generate_intent() {
        // Returned by POST /v0/generate-intent on 2026-11-13 for a quote whose deposit
        // address deadline was 2026-11-16T03:33:00.000Z.
        let n = Nonce::from_base64("Vij2xgAlKBKzADjHkhJp6BgCJ5mygKii98kQRBEIC/k=").unwrap();
        let v = n.as_versioned().expect("versioned");
        assert_eq!(v.salt.to_hex(), "252812b3");
        assert_eq!(time::format(v.deadline), "2026-11-16T03:33:00.000Z");
        assert_eq!(hex::encode(v.random), "022799b280a8a2f7c9104411080bf9");
    }

    #[test]
    fn versioned_roundtrip() {
        let deadline = time::parse("2026-11-16T03:33:00.000Z").unwrap();
        let salt = Salt::from_hex("252812b3").unwrap();
        let random: [u8; 15] = hex::decode("022799b280a8a2f7c9104411080bf9")
            .unwrap()
            .try_into()
            .unwrap();
        let n = Nonce::versioned(salt, deadline, random).unwrap();
        assert_eq!(
            n.to_base64(),
            "Vij2xgAlKBKzADjHkhJp6BgCJ5mygKii98kQRBEIC/k="
        );
        assert_eq!(Nonce::from_base64(&n.to_base64()).unwrap(), n);
    }

    #[test]
    fn legacy_is_not_versioned() {
        let n = Nonce::legacy([9u8; 32]).unwrap();
        assert!(n.as_versioned().is_none());
        assert!(Nonce::legacy({
            let mut b = [0u8; 32];
            b[..4].copy_from_slice(&VERSIONED_MAGIC);
            b
        })
        .is_err());
    }

    #[test]
    fn rejects_wrong_lengths_and_bad_salt() {
        assert!(Nonce::from_base64("AAAA").is_err());
        assert!(Nonce::from_base64("not base64!").is_err());
        assert!(Salt::from_hex("2528").is_err());
        assert!(Salt::from_hex("zz2812b3").is_err());
    }

    #[test]
    fn random_versioned_nonces_differ() {
        let deadline = time::parse("2026-11-16T03:33:00.000Z").unwrap();
        let salt = Salt([1, 2, 3, 4]);
        let a = Nonce::versioned_random(salt, deadline).unwrap();
        let b = Nonce::versioned_random(salt, deadline).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.as_versioned().unwrap().deadline, deadline);
    }
}
