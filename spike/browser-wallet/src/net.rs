//! The network interface the wallet needs: one blocking POST, and a millisecond clock.
//!
//! Native builds use `reqwest::blocking`. WebAssembly builds use a synchronous
//! `XMLHttpRequest`, which browsers allow inside a Web Worker (not on the page's main
//! thread). That keeps the wallet's blocking control flow unchanged on both targets.

use anyhow::{bail, Result};

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// Lower-case header names. In a browser only CORS-exposed headers are visible.
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub trait Transport {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response>;
}

/// The platform's default transport.
pub fn default_transport() -> Box<dyn Transport> {
    Box::new(Http)
}

pub struct Http;

#[cfg(not(target_arch = "wasm32"))]
impl Transport for Http {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(120)).build()?;
        let mut req = client.post(url).body(body.to_vec());
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.send()?;
        let status = resp.status().as_u16();
        let hdrs = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        Ok(Response { status, headers: hdrs, body: resp.bytes()?.to_vec() })
    }
}

#[cfg(target_arch = "wasm32")]
impl Transport for Http {
    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response> {
        use anyhow::anyhow;
        use wasm_bindgen::JsCast;
        let js = |e: wasm_bindgen::JsValue| anyhow!("XMLHttpRequest {url}: {e:?}");
        let xhr = web_sys::XmlHttpRequest::new().map_err(js)?;
        // async = false: a synchronous request. Allowed in workers only.
        xhr.open_with_async("POST", url, false).map_err(js)?;
        xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
        for (k, v) in headers {
            xhr.set_request_header(k, v).map_err(js)?;
        }
        let mut copy = body.to_vec();
        xhr.send_with_opt_u8_array(Some(&mut copy[..])).map_err(js)?;
        let status = xhr.status().map_err(js)?;
        let buf = xhr.response().map_err(js)?;
        let body = if buf.is_null() || buf.is_undefined() {
            vec![]
        } else {
            js_sys::Uint8Array::new(buf.unchecked_ref::<js_sys::ArrayBuffer>()).to_vec()
        };
        let raw = xhr.get_all_response_headers().map_err(js)?;
        let headers = raw
            .split("\r\n")
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        Ok(Response { status, body, headers })
    }
}

/// Milliseconds from an arbitrary origin, for timings only.
pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
}

pub fn ensure_ok(r: &Response, what: &str) -> Result<()> {
    if r.status != 200 {
        bail!("{what}: HTTP {} {}", r.status, String::from_utf8_lossy(&r.body[..r.body.len().min(300)]));
    }
    Ok(())
}
