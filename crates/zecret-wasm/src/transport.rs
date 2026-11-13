//! `zecret-intents` speaks `HttpTransport` (GET and POST); the core speaks `Transport`
//! (POST only). [`CorePost`] lets the Verifier view calls and the NEAR RPC share the
//! core's transport. The 1Click status call needs GET, which the browser transport in
//! `web` provides directly.

use zecret_core::net::SharedTransport;
use zecret_intents::{HttpRequest, HttpResponse, HttpTransport, Method};

/// The core's transport as an `HttpTransport` for POST requests (NEAR JSON-RPC).
pub struct CorePost(pub SharedTransport);

impl HttpTransport for CorePost {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        if r.method != Method::Post {
            return Err(format!("{} {}: only POST is available on this transport", r.method.as_str(), r.url));
        }
        let headers: Vec<(&str, &str)> = r.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let resp = self.0.post(&r.url, &headers, r.body.as_deref().unwrap_or_default()).map_err(|e| format!("{e:#}"))?;
        Ok(HttpResponse::new(resp.status, resp.body))
    }
}
