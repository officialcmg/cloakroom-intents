//! The one network interface of the core: a blocking HTTP POST.
//!
//! The core never opens a socket. Every chain client (Zebra JSON-RPC, lightwalletd over
//! gRPC-web, NEAR JSON-RPC) is a thin layer over [`Transport`]. A native program
//! implements it with a blocking HTTP client; a browser Web Worker implements it with a
//! synchronous `XMLHttpRequest` (allowed in workers, not on the page's main thread).

use anyhow::{bail, Result};
use std::rc::Rc;

/// An HTTP response.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// Header names in lower case. In a browser only CORS-exposed headers are visible.
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// One blocking HTTP POST. Implementations return `Err` only when no HTTP response was
/// received (connection refused, timeout); an HTTP error status is an `Ok(Response)`.
pub trait Transport {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response>;
}

/// The shared handle the chain clients hold.
pub type SharedTransport = Rc<dyn Transport>;

pub fn ensure_ok(r: &Response, what: &str) -> Result<()> {
    if r.status != 200 {
        bail!("{what}: HTTP {} {}", r.status, String::from_utf8_lossy(&r.body[..r.body.len().min(300)]));
    }
    Ok(())
}
