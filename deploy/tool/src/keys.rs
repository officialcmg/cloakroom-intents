//! Private keys of the accounts the tool creates or uses.
//!
//! Each key is one file `<dir>/<account_id>.json` in the format of near-cli's
//! credentials (`account_id`, `public_key`, `private_key`). The directory is created
//! with mode 0700 and each file with mode 0600. Keys are never printed.

use near_workspaces::types::{KeyType, SecretKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::Res;

#[derive(Serialize, Deserialize)]
struct KeyFile {
    account_id: String,
    public_key: String,
    private_key: String,
}

pub struct KeyStore {
    dir: PathBuf,
}

impl KeyStore {
    pub fn open(dir: &Path) -> Res<Self> {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    fn path(&self, account_id: &str) -> PathBuf {
        self.dir.join(format!("{account_id}.json"))
    }

    pub fn get(&self, account_id: &str) -> Res<Option<SecretKey>> {
        let p = self.path(account_id);
        if !p.exists() {
            return Ok(None);
        }
        let mode = fs::metadata(&p)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "key file {} has mode {mode:o}; it must not be readable by others (chmod 600)",
                p.display()
            )
            .into());
        }
        let kf: KeyFile = serde_json::from_slice(&fs::read(&p)?)?;
        if kf.account_id != account_id {
            return Err(format!("key file {} is for {}", p.display(), kf.account_id).into());
        }
        let sk: SecretKey = kf
            .private_key
            .parse()
            .map_err(|_| format!("key file {}: bad private_key", p.display()))?;
        if sk.public_key().to_string() != kf.public_key {
            return Err(format!("key file {}: public_key does not match", p.display()).into());
        }
        Ok(Some(sk))
    }

    pub fn put(&self, account_id: &str, sk: &SecretKey) -> Res<()> {
        let p = self.path(account_id);
        if p.exists() {
            return Err(format!("refusing to overwrite key file {}", p.display()).into());
        }
        let kf = KeyFile {
            account_id: account_id.to_string(),
            public_key: sk.public_key().to_string(),
            private_key: sk.to_string(),
        };
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&p)?;
        f.write_all(serde_json::to_string_pretty(&kf)?.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }

    /// The stored key of `account_id`, or a new one that is stored first. The key is
    /// saved before any transaction uses it, so a crash never loses a funded key.
    pub fn get_or_create(&self, account_id: &str) -> Res<SecretKey> {
        if let Some(sk) = self.get(account_id)? {
            return Ok(sk);
        }
        let sk = SecretKey::from_random(KeyType::ED25519);
        self.put(account_id, &sk)?;
        Ok(sk)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_with_private_modes() {
        let dir = std::env::temp_dir().join(format!("zecret-keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ks = KeyStore::open(&dir).unwrap();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(ks.get("a.testnet").unwrap().is_none());
        let sk = ks.get_or_create("a.testnet").unwrap();
        let again = ks.get_or_create("a.testnet").unwrap();
        assert_eq!(sk.public_key(), again.public_key());
        let mode = fs::metadata(dir.join("a.testnet.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert!(ks.put("a.testnet", &sk).is_err(), "no overwrite");
        fs::set_permissions(
            dir.join("a.testnet.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(ks.get("a.testnet").is_err(), "world-readable key refused");
        fs::remove_dir_all(&dir).unwrap();
    }
}
