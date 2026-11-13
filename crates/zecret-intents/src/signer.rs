//! The taker's ed25519 key as a Verifier signer.
//!
//! The key's Verifier account id is the lowercase hex of its 32-byte public key (a
//! NEAR implicit account id). The Verifier accepts that key for that account without
//! any `add_public_key` call, and the account does not need to exist on NEAR.

use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::payload::{SignedPayload, UnsignedPayload};

/// An ed25519 signing key. The secret never leaves this value except through
/// [`IntentSigner::seed`], which the app needs for the recovery file.
pub struct IntentSigner {
    key: SigningKey,
}

impl IntentSigner {
    /// From a 32-byte ed25519 seed.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            key: SigningKey::from_bytes(seed),
        }
    }

    /// A new random key from the platform's random source. The seed buffer is zeroed
    /// when this returns. (`SigningKey` zeroes its own copy on drop.)
    pub fn generate() -> Result<Self> {
        let mut seed = Zeroizing::new([0u8; 32]);
        crate::random_bytes(&mut seed[..])?;
        Ok(Self::from_seed(&seed))
    }

    /// A copy of the 32-byte seed, zeroed when the returned value is dropped.
    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.key.to_bytes())
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// `ed25519:<base58>`.
    pub fn public_key_str(&self) -> String {
        format!("ed25519:{}", bs58::encode(self.public_key()).into_string())
    }

    /// The Verifier account id: lowercase hex of the public key.
    pub fn account_id(&self) -> String {
        hex::encode(self.public_key())
    }

    /// Signs a payload. Only this crate can make an [`UnsignedPayload`], so a 1Click
    /// payload reaches this point only after [`crate::check::check_generated_intent`].
    pub fn sign(&self, payload: &UnsignedPayload) -> Result<SignedPayload> {
        let bytes = payload.bytes_to_sign()?;
        let signature = self.key.sign(&bytes).to_bytes();
        let signed = SignedPayload {
            unsigned: payload.clone(),
            public_key: self.public_key(),
            signature,
        };
        verify(&signed)?;
        Ok(signed)
    }
}

impl core::fmt::Debug for IntentSigner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IntentSigner")
            .field("account_id", &self.account_id())
            .finish_non_exhaustive()
    }
}

/// Verifies a signed payload the way the Verifier does (`verify_strict`).
pub fn verify(signed: &SignedPayload) -> Result<()> {
    let pk = VerifyingKey::from_bytes(&signed.public_key)
        .map_err(|e| Error::Invalid(format!("public key: {e}")))?;
    let sig = ed25519_dalek::Signature::from_bytes(&signed.signature);
    pk.verify_strict(&signed.unsigned.bytes_to_sign()?, &sig)
        .map_err(|e| Error::Invalid(format!("signature does not verify: {e}")))
}
