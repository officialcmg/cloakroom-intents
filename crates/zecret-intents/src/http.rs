//! Plain HTTP request and response values. This crate never sends them; the caller
//! does, with whatever client the platform has (a synchronous `XMLHttpRequest` in a
//! Web Worker, a blocking client natively).

use crate::error::{ApiError, Error, Result};

/// HTTP method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// A request for the caller to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    /// `None` for GET.
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: vec![("accept".into(), "application/json".into())],
            body: None,
        }
    }

    pub fn post_json(url: impl Into<String>, body: &serde_json::Value) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body: Some(body.to_string().into_bytes()),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .push((name.to_ascii_lowercase(), value.to_owned()));
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The body parsed as JSON (for tests and logging).
    pub fn json_body(&self) -> Option<serde_json::Value> {
        self.body
            .as_deref()
            .and_then(|b| serde_json::from_slice(b).ok())
    }
}

/// A response the caller received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    /// Returns the body if the status is 2xx, else the classified 1Click error.
    pub fn ok_body(&self) -> Result<&[u8]> {
        if (200..300).contains(&self.status) {
            Ok(&self.body)
        } else {
            Err(Error::Api(ApiError::from_response(self.status, &self.body)))
        }
    }
}

/// One blocking HTTP exchange. Implementations return `Err` only when no HTTP response
/// was received; an HTTP error status is an `Ok(HttpResponse)`.
///
/// This is the same contract as `zecret_core::net::Transport::post`, plus GET, which
/// `/v0/tokens` and `/v0/status` need.
pub trait HttpTransport {
    fn send(&self, request: &HttpRequest) -> core::result::Result<HttpResponse, String>;
}

impl<T: HttpTransport + ?Sized> HttpTransport for &T {
    fn send(&self, request: &HttpRequest) -> core::result::Result<HttpResponse, String> {
        (**self).send(request)
    }
}

pub(crate) fn send<T: HttpTransport + ?Sized>(t: &T, r: &HttpRequest) -> Result<HttpResponse> {
    t.send(r).map_err(Error::Transport)
}

pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8], what: &str) -> Result<T> {
    serde_json::from_slice(body).map_err(|e| Error::Parse(format!("{what}: {e}")))
}
