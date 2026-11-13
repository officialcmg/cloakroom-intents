//! The native implementations of the core's interfaces: an HTTP transport (blocking
//! reqwest) and a store of files.

use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use zecret_core::{
    net::{Response, Transport},
    store::Store,
};

pub struct Http {
    client: reqwest::blocking::Client,
}

impl Http {
    pub fn new() -> Result<Http> {
        Ok(Http { client: reqwest::blocking::Client::builder().timeout(Duration::from_secs(120)).build()? })
    }
}

impl Transport for Http {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        let mut req = self.client.post(url).body(body.to_vec());
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.send().with_context(|| format!("POST {url}"))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().iter().map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or_default().to_string())).collect();
        Ok(Response { status, headers, body: resp.bytes()?.to_vec() })
    }
}

/// A store of files under one directory. Key `a/b` is the file `<dir>/a/b.json`.
/// Writes go to a temporary file first and are renamed, so a record is never half written.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: &Path) -> Result<FileStore> {
        fs::create_dir_all(dir)?;
        Ok(FileStore { dir: dir.to_path_buf() })
    }
    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }
}

impl Store for FileStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match fs::read(self.path(key)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", self.path(key).display())),
        }
    }

    fn put(&mut self, key: &str, value: &[u8]) -> Result<()> {
        let p = self.path(key);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, value).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, &p)?;
        Ok(())
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        // Prefixes name a directory ("swap/") or a file stem ("wallet").
        let (sub, stem) = match prefix.rfind('/') {
            Some(i) => (&prefix[..i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        let d = self.dir.join(sub);
        let mut out = vec![];
        if let Ok(rd) = fs::read_dir(&d) {
            for e in rd {
                let name = e?.file_name().to_string_lossy().to_string();
                if let Some(k) = name.strip_suffix(".json") {
                    if k.starts_with(stem) {
                        out.push(if sub.is_empty() { k.to_string() } else { format!("{sub}/{k}") });
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trip() {
        let dir = std::env::temp_dir().join(format!("zecret-filestore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut s = FileStore::new(&dir).unwrap();
        s.put("wallet", b"k").unwrap();
        s.put("swap/aa", b"1").unwrap();
        s.put("swap/bb", b"2").unwrap();
        assert_eq!(s.get("swap/bb").unwrap().unwrap(), b"2");
        assert!(s.get("nope").unwrap().is_none());
        assert_eq!(s.keys("swap/").unwrap(), vec!["swap/aa", "swap/bb"]);
        assert_eq!(s.keys("wal").unwrap(), vec!["wallet"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
