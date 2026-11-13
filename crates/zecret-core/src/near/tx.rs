//! NEAR transactions: borsh serialization, sha256 hash and ed25519 signature.
//!
//! Written out by hand instead of using `near-primitives`, which does not build for the
//! browser. Only what Zecret sends is here: the actions CreateAccount, DeployContract,
//! FunctionCall, Transfer and AddKey, and ed25519 keys. The layout is nearcore's
//! `Transaction::V0` and `SignedTransaction` (borsh): strings and byte vectors carry a u32
//! little-endian length, integers are little-endian, enums carry a u8 variant index.

use anyhow::{anyhow, bail, Result};
use ed25519_dalek::{Signer as _, SigningKey, Verifier as _, VerifyingKey};
use sha2::{Digest, Sha256};

/// Gas units. One Tgas is 10^12 gas.
pub const TGAS: u64 = 1_000_000_000_000;
/// One NEAR in yoctoNEAR.
pub const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

/// An ed25519 key pair for a NEAR account.
#[derive(Clone)]
pub struct NearKey {
    sk: SigningKey,
}

impl std::fmt::Debug for NearKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NearKey({})", self.public_key_str())
    }
}

impl NearKey {
    pub fn generate() -> NearKey {
        NearKey::from_seed(crate::random_bytes())
    }
    /// The key from its 32-byte ed25519 seed.
    pub fn from_seed(seed: [u8; 32]) -> NearKey {
        NearKey { sk: SigningKey::from_bytes(&seed) }
    }
    pub fn seed(&self) -> [u8; 32] {
        self.sk.to_bytes()
    }
    /// Parses the NEAR CLI form `ed25519:<base58 of seed ‖ public key>` (64 bytes) or of
    /// the bare 32-byte seed.
    pub fn from_secret_str(s: &str) -> Result<NearKey> {
        let b58 = s.trim().strip_prefix("ed25519:").ok_or_else(|| anyhow!("secret key must start with ed25519:"))?;
        let b = bs58::decode(b58).into_vec().map_err(|e| anyhow!("bad base58 secret key: {e}"))?;
        let seed: [u8; 32] = b.get(..32).ok_or_else(|| anyhow!("secret key too short"))?.try_into()?;
        let k = NearKey::from_seed(seed);
        if b.len() == 64 && b[32..] != k.public_bytes() {
            bail!("secret key: the public half does not match the seed");
        }
        if b.len() != 64 && b.len() != 32 {
            bail!("secret key has {} bytes, expected 32 or 64", b.len());
        }
        Ok(k)
    }
    pub fn secret_str(&self) -> String {
        let mut b = self.seed().to_vec();
        b.extend_from_slice(&self.public_bytes());
        format!("ed25519:{}", bs58::encode(b).into_string())
    }
    pub fn public_bytes(&self) -> [u8; 32] {
        self.sk.verifying_key().to_bytes()
    }
    /// `ed25519:<base58>`, the form NEAR RPC uses.
    pub fn public_key_str(&self) -> String {
        public_key_str(&self.public_bytes())
    }
    /// The implicit account of this key: the lower-case hex of the public key.
    pub fn implicit_account(&self) -> String {
        hex::encode(self.public_bytes())
    }
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.sk.sign(msg).to_bytes()
    }
}

pub fn public_key_str(pk: &[u8; 32]) -> String {
    format!("ed25519:{}", bs58::encode(pk).into_string())
}

pub fn parse_public_key(s: &str) -> Result<[u8; 32]> {
    let b58 = s.trim().strip_prefix("ed25519:").ok_or_else(|| anyhow!("public key must start with ed25519:"))?;
    let b = bs58::decode(b58).into_vec().map_err(|e| anyhow!("bad base58 public key: {e}"))?;
    b.try_into().map_err(|_| anyhow!("public key must be 32 bytes"))
}

/// True if `sig` is a valid ed25519 signature of `msg` by `pk`.
pub fn verify(pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    match VerifyingKey::from_bytes(pk) {
        Ok(vk) => vk.verify(msg, &ed25519_dalek::Signature::from_bytes(sig)).is_ok(),
        Err(_) => false,
    }
}

pub fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

/// A contract's code hash as NEAR reports it (`view_account.code_hash`): base58 of
/// sha256 of the wasm.
pub fn code_hash(wasm: &[u8]) -> String {
    bs58::encode(sha256(wasm)).into_string()
}

/// The actions Zecret sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    CreateAccount,
    DeployContract { code: Vec<u8> },
    FunctionCall { method: String, args: Vec<u8>, gas: u64, deposit: u128 },
    Transfer { deposit: u128 },
    /// Adds a full-access key.
    AddFullAccessKey { public_key: [u8; 32] },
    DeleteKey { public_key: [u8; 32] },
}

impl Action {
    /// A function call with JSON arguments.
    pub fn call(method: &str, args: &serde_json::Value, gas: u64, deposit: u128) -> Action {
        Action::FunctionCall { method: method.to_string(), args: args.to_string().into_bytes(), gas, deposit }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub signer_id: String,
    pub public_key: [u8; 32],
    pub nonce: u64,
    pub receiver_id: String,
    pub block_hash: [u8; 32],
    pub actions: Vec<Action>,
}

/// A signed transaction ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedTx {
    /// borsh(SignedTransaction).
    pub bytes: Vec<u8>,
    /// sha256 of borsh(Transaction): the transaction hash.
    pub hash: [u8; 32],
}

impl SignedTx {
    /// The hash in base58, as explorers and RPC show it.
    pub fn hash_str(&self) -> String {
        bs58::encode(self.hash).into_string()
    }
    pub fn base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.bytes)
    }
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u128(&mut self, v: u128) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn fixed(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn ed25519(&mut self, pk: &[u8; 32]) {
        self.u8(0); // KeyType::ED25519
        self.fixed(pk);
    }
}

impl Transaction {
    /// borsh(Transaction::V0). V0 is written without a version byte.
    pub fn borsh(&self) -> Vec<u8> {
        let mut w = W(Vec::with_capacity(256));
        w.bytes(self.signer_id.as_bytes());
        w.ed25519(&self.public_key);
        w.u64(self.nonce);
        w.bytes(self.receiver_id.as_bytes());
        w.fixed(&self.block_hash);
        w.u32(self.actions.len() as u32);
        for a in &self.actions {
            match a {
                Action::CreateAccount => w.u8(0),
                Action::DeployContract { code } => {
                    w.u8(1);
                    w.bytes(code);
                }
                Action::FunctionCall { method, args, gas, deposit } => {
                    w.u8(2);
                    w.bytes(method.as_bytes());
                    w.bytes(args);
                    w.u64(*gas);
                    w.u128(*deposit);
                }
                Action::Transfer { deposit } => {
                    w.u8(3);
                    w.u128(*deposit);
                }
                Action::AddFullAccessKey { public_key } => {
                    w.u8(5);
                    w.ed25519(public_key);
                    w.u64(0); // AccessKey.nonce
                    w.u8(1); // AccessKeyPermission::FullAccess
                }
                Action::DeleteKey { public_key } => {
                    w.u8(6);
                    w.ed25519(public_key);
                }
            }
        }
        w.0
    }

    pub fn hash(&self) -> [u8; 32] {
        sha256(&self.borsh())
    }

    /// Signs the transaction hash with `key`, which must be the transaction's `public_key`.
    pub fn sign(&self, key: &NearKey) -> Result<SignedTx> {
        if key.public_bytes() != self.public_key {
            bail!("signing key does not match the transaction's public key");
        }
        let body = self.borsh();
        let hash = sha256(&body);
        let sig = key.sign(&hash);
        let mut w = W(body);
        w.u8(0); // Signature::ED25519
        w.fixed(&sig);
        Ok(SignedTx { bytes: w.0, hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transfer signed by a fixed key, checked byte by byte against the borsh layout of
    /// nearcore's SignedTransaction. The same code path (with fresh nonces and block hashes)
    /// is accepted by a NEAR 2.14.1 sandbox in `crates/zecret-node/tests/sandbox.rs`.
    #[test]
    fn transfer_layout_and_signature() {
        let key = NearKey::from_seed([7u8; 32]);
        let tx = Transaction {
            signer_id: "alice.near".into(),
            public_key: key.public_bytes(),
            nonce: 5,
            receiver_id: "bob.near".into(),
            block_hash: [9u8; 32],
            actions: vec![Action::Transfer { deposit: 10u128.pow(22) }],
        };
        let b = tx.borsh();
        let mut expect = vec![];
        expect.extend_from_slice(&10u32.to_le_bytes());
        expect.extend_from_slice(b"alice.near");
        expect.push(0);
        expect.extend_from_slice(&key.public_bytes());
        expect.extend_from_slice(&5u64.to_le_bytes());
        expect.extend_from_slice(&8u32.to_le_bytes());
        expect.extend_from_slice(b"bob.near");
        expect.extend_from_slice(&[9u8; 32]);
        expect.extend_from_slice(&1u32.to_le_bytes());
        expect.push(3);
        expect.extend_from_slice(&10u128.pow(22).to_le_bytes());
        assert_eq!(b, expect);
        let s = tx.sign(&key).unwrap();
        assert_eq!(s.hash, sha256(&expect));
        assert_eq!(&s.bytes[..expect.len()], &expect[..]);
        assert_eq!(s.bytes[expect.len()], 0);
        let sig: [u8; 64] = s.bytes[expect.len() + 1..].try_into().unwrap();
        assert!(verify(&key.public_bytes(), &s.hash, &sig));
        assert_eq!(s.bytes.len(), expect.len() + 65);
        // ed25519 is deterministic: this transaction always has the same hash and bytes.
        assert_eq!(s, tx.sign(&key).unwrap());
        // a different key cannot sign it
        assert!(tx.sign(&NearKey::from_seed([8u8; 32])).is_err());
    }

    #[test]
    fn function_call_and_add_key_layout() {
        let key = NearKey::from_seed([1u8; 32]);
        let tx = Transaction {
            signer_id: "a".into(),
            public_key: key.public_bytes(),
            nonce: 1,
            receiver_id: "b".into(),
            block_hash: [0u8; 32],
            actions: vec![
                Action::call("m", &serde_json::json!({}), 30 * TGAS, 1),
                Action::AddFullAccessKey { public_key: [2u8; 32] },
                Action::CreateAccount,
                Action::DeployContract { code: vec![0, 97, 115, 109] },
                Action::DeleteKey { public_key: [3u8; 32] },
            ],
        };
        let b = tx.borsh();
        let head = 4 + 1 + 1 + 32 + 8 + 4 + 1 + 32;
        let mut a = vec![];
        a.extend_from_slice(&5u32.to_le_bytes());
        a.push(2);
        a.extend_from_slice(&1u32.to_le_bytes());
        a.push(b'm');
        a.extend_from_slice(&2u32.to_le_bytes());
        a.extend_from_slice(b"{}");
        a.extend_from_slice(&(30 * TGAS).to_le_bytes());
        a.extend_from_slice(&1u128.to_le_bytes());
        a.push(5);
        a.push(0);
        a.extend_from_slice(&[2u8; 32]);
        a.extend_from_slice(&0u64.to_le_bytes());
        a.push(1);
        a.push(0);
        a.push(1);
        a.extend_from_slice(&4u32.to_le_bytes());
        a.extend_from_slice(&[0, 97, 115, 109]);
        a.push(6);
        a.push(0);
        a.extend_from_slice(&[3u8; 32]);
        assert_eq!(&b[head..], &a[..]);
    }

    #[test]
    fn key_strings_round_trip() {
        let k = NearKey::generate();
        let again = NearKey::from_secret_str(&k.secret_str()).unwrap();
        assert_eq!(again.public_bytes(), k.public_bytes());
        assert_eq!(parse_public_key(&k.public_key_str()).unwrap(), k.public_bytes());
        assert_eq!(k.implicit_account().len(), 64);
        // a 64-byte secret whose halves do not match is refused
        let other = NearKey::generate();
        let mut b = k.seed().to_vec();
        b.extend_from_slice(&other.public_bytes());
        assert!(NearKey::from_secret_str(&format!("ed25519:{}", bs58::encode(b).into_string())).is_err());
        // the sandbox validator key format
        let v = NearKey::from_secret_str("ed25519:4kvpP9iksTVNaTxEg8V8TzApGhksCV64DXMWhKfxzyAJVfMUbxh8RcBozRTmLNTVnVThi5nXkGJgFBa7LXDvvEqb").unwrap();
        assert_eq!(v.public_key_str(), "ed25519:6DN64ZDTzsd9gpp7ppzXRhb4BHvGnDccGvZM9Jd4oSZB");
    }
}
