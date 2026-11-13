//! The storage interface of the core: a small key-value store of byte strings.
//!
//! The state machines write every record they must not lose (wallet key, swap record) with
//! [`Store::put`] before they act on it. A native program implements the store with files.
//! In a browser, IndexedDB is asynchronous and blocking Rust code cannot wait for it, so a
//! worker uses [`MemStore`]: it loads the records into memory before it creates an engine,
//! and after every `tick` it writes [`MemStore::take_dirty`] to IndexedDB before it calls
//! `tick` again. The engines never use a secret in an outgoing message in the same tick
//! that created it, so a record is always durable before the world can depend on it.

use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

pub trait Store {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn put(&mut self, key: &str, value: &[u8]) -> Result<()>;
    /// Keys that start with `prefix`, in byte order.
    fn keys(&self, prefix: &str) -> Result<Vec<String>>;
}

/// An in-memory store that remembers which keys changed.
#[derive(Default, Debug, Clone)]
pub struct MemStore {
    map: BTreeMap<String, Vec<u8>>,
    dirty: BTreeSet<String>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads a record without marking it changed (use it to fill the store from disk or
    /// IndexedDB).
    pub fn load(&mut self, key: &str, value: Vec<u8>) {
        self.map.insert(key.to_string(), value);
    }

    /// The records changed since the last call, with their current values.
    pub fn take_dirty(&mut self) -> Vec<(String, Vec<u8>)> {
        let keys = std::mem::take(&mut self.dirty);
        keys.into_iter().filter_map(|k| self.map.get(&k).map(|v| (k.clone(), v.clone()))).collect()
    }
}

impl Store for MemStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.map.get(key).cloned())
    }
    fn put(&mut self, key: &str, value: &[u8]) -> Result<()> {
        self.map.insert(key.to_string(), value.to_vec());
        self.dirty.insert(key.to_string());
        Ok(())
    }
    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self.map.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)).map(|(k, _)| k.clone()).collect())
    }
}

/// JSON helpers on top of a store.
pub fn get_json<T: serde::de::DeserializeOwned>(s: &dyn Store, key: &str) -> Result<Option<T>> {
    match s.get(key)? {
        Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
        None => Ok(None),
    }
}

pub fn put_json<T: serde::Serialize>(s: &mut dyn Store, key: &str, v: &T) -> Result<()> {
    s.put(key, &serde_json::to_vec_pretty(v)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_tracking_and_prefix_scan() {
        let mut s = MemStore::new();
        s.load("swap/a", b"1".to_vec());
        assert!(s.take_dirty().is_empty(), "loaded records are not dirty");
        s.put("swap/b", b"2").unwrap();
        s.put("wallet", b"k").unwrap();
        s.put("swap/b", b"3").unwrap();
        let d = s.take_dirty();
        assert_eq!(d, vec![("swap/b".to_string(), b"3".to_vec()), ("wallet".to_string(), b"k".to_vec())]);
        assert!(s.take_dirty().is_empty());
        assert_eq!(s.keys("swap/").unwrap(), vec!["swap/a", "swap/b"]);
        assert_eq!(s.get("swap/a").unwrap().unwrap(), b"1");
        assert!(s.get("nope").unwrap().is_none());
    }
}
