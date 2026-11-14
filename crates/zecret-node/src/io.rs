//! The native implementations of the core's interfaces: an HTTP transport (blocking
//! reqwest) and a store of files.

use anyhow::{Context, Result};
use std::{
    fs,
    io::Write,
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
        // A server that does not accept the connection within 10 s is treated as down, so
        // the NEAR RPC client moves on to its next endpoint (incident 2026-11-14).
        Ok(Http { client: reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(120)).build()? })
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

    fn now_ms(&self) -> Option<u64> {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
    }
}

/// A store of files under one directory. Key `a/b` is the file `<dir>/a/b.json`.
///
/// The records hold secrets (wallet key, key shares) and decisions the maker must not
/// forget (for example that b is public). So (security review 2, H4):
/// - a write goes to a temporary file, which is flushed to disk (`fsync`), renamed over
///   the record, and then the directory is flushed too, so a power loss cannot bring
///   back an older record;
/// - files are created with mode 0600 and directories with mode 0700 (Unix). The store
///   directory itself is set to 0700 when it is opened.
pub struct FileStore {
    dir: PathBuf,
}

/// Creates `dir` and its missing parents with mode 0700 (Unix).
fn create_private_dir(dir: &Path) -> Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir).with_context(|| format!("create {}", dir.display()))
}

/// Flushes a directory's entries (a rename in it) to disk. No-op where directories
/// cannot be opened as files.
fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(dir).and_then(|d| d.sync_all()).with_context(|| format!("fsync {}", dir.display()))?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

impl FileStore {
    pub fn new(dir: &Path) -> Result<FileStore> {
        create_private_dir(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir)?.permissions().mode() & 0o777;
            if mode != 0o700 {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                    .with_context(|| format!("set mode 0700 on the store directory {} (it holds keys)", dir.display()))?;
            }
        }
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
        let parent = p.parent().map(Path::to_path_buf).unwrap_or_else(|| self.dir.clone());
        if !parent.is_dir() {
            create_private_dir(&parent)?;
            if let Some(up) = parent.parent() {
                sync_dir(up)?;
            }
        }
        let tmp = p.with_extension("tmp");
        let mut o = fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let write = || -> std::io::Result<()> {
            let mut f = o.open(&tmp)?;
            // A temporary file left by an older version may have a wider mode.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            f.write_all(value)?;
            f.sync_all()
        };
        write().with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, &p).with_context(|| format!("rename {} to {}", tmp.display(), p.display()))?;
        sync_dir(&parent)?;
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
                if let Some(k) = name.strip_suffix(".json")
                    && k.starts_with(stem) {
                        out.push(if sub.is_empty() { k.to_string() } else { format!("{sub}/{k}") });
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

    /// Review 2, H4: secret files are 0600, directories 0700, no temporary file is left,
    /// and an existing wider store directory is narrowed.
    #[cfg(unix)]
    #[test]
    fn file_store_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = std::env::temp_dir().join(format!("zecret-filestore-mode-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let mut s = FileStore::new(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        s.put("wallet", b"secret").unwrap();
        s.put("swap/aa", b"1").unwrap();
        s.put("swap/aa", b"2").unwrap();
        assert_eq!(mode(&dir.join("wallet.json")), 0o600);
        assert_eq!(mode(&dir.join("swap/aa.json")), 0o600);
        assert_eq!(mode(&dir.join("swap")), 0o700);
        assert_eq!(s.get("swap/aa").unwrap().unwrap(), b"2");
        let left: Vec<_> = fs::read_dir(dir.join("swap")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("aa.json")]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
