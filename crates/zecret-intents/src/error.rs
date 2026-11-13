//! Errors of this crate.

use core::fmt;

use crate::check::Refusal;

/// An error from building a request, parsing a response or checking an intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The caller's transport got no HTTP response (connection refused, timeout).
    Transport(String),
    /// The 1Click API answered with an HTTP error status.
    Api(ApiError),
    /// A NEAR JSON-RPC call failed, or the contract call inside it failed.
    Rpc(String),
    /// A response did not have the expected shape.
    Parse(String),
    /// An input to a builder was not valid.
    Invalid(String),
    /// The pre-signing check refused to sign. Nothing was signed.
    Refused(Refusal),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(m) => write!(f, "no HTTP response: {m}"),
            Self::Api(e) => write!(f, "{e}"),
            Self::Rpc(m) => write!(f, "NEAR RPC: {m}"),
            Self::Parse(m) => write!(f, "unexpected response: {m}"),
            Self::Invalid(m) => write!(f, "invalid input: {m}"),
            Self::Refused(r) => write!(f, "refused to sign: {r}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Refusal> for Error {
    fn from(r: Refusal) -> Self {
        Self::Refused(r)
    }
}

pub type Result<T, E = Error> = core::result::Result<T, E>;

/// An HTTP error answer from the 1Click API, classified so that the app can react.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    /// The `message` field of the answer, or the start of the body if it has none.
    pub message: String,
    pub kind: ApiErrorKind,
}

/// What an [`ApiError`] means for the app. The classification uses the HTTP status and
/// message texts recorded from the live API on 2026-11-13 (see `docs/INTENTS.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiErrorKind {
    /// 401: the `X-API-Key` (or bearer JWT) was rejected.
    Unauthorized,
    /// 429: rate limited. Unauthenticated clients get 1200 requests per 60 s.
    RateLimited,
    /// `deadline is not valid`: the deadline is in the past or not ISO 8601.
    DeadlineInvalid,
    /// `recipient is not valid` (for example the 0x…dEaD burn address).
    RecipientInvalid,
    /// `Amount is too low for bridge, try at least N`.
    AmountTooLow { min: Option<u128> },
    /// The quote is in a state that no longer accepts a deposit, for example
    /// `Quote is in FAILED status, expected PENDING_DEPOSIT or INCOMPLETE_DEPOSIT`.
    QuoteNotPending,
    /// 404 from `/v0/status`: the deposit address is unknown.
    NotFound,
    /// `Cannot parse signed intent`.
    BadSignedIntent,
    /// `Quoting for this pair is not available`: no solver quoted. Seen twice in 120
    /// dry quotes over 15 minutes, for SOL and ETH, and gone 20 s later. Retry.
    NoQuote,
    /// 5xx: a server-side failure. Retry later.
    Server,
    /// Any other 4xx answer.
    Other,
}

impl ApiError {
    /// Builds the error from an HTTP status and body.
    pub fn from_response(status: u16, body: &[u8]) -> Self {
        let message = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
            .unwrap_or_else(|| {
                let text = String::from_utf8_lossy(body);
                text.chars().take(300).collect()
            });
        let kind = classify(status, &message);
        Self {
            status,
            message,
            kind,
        }
    }
}

fn classify(status: u16, message: &str) -> ApiErrorKind {
    let lower = message.to_ascii_lowercase();
    if status == 401 {
        ApiErrorKind::Unauthorized
    } else if status == 429 {
        ApiErrorKind::RateLimited
    } else if status == 404 {
        ApiErrorKind::NotFound
    } else if status >= 500 {
        ApiErrorKind::Server
    } else if lower.contains("deadline") {
        ApiErrorKind::DeadlineInvalid
    } else if lower.contains("recipient is not valid") {
        ApiErrorKind::RecipientInvalid
    } else if lower.contains("amount is too low") {
        let min = message
            .rsplit(' ')
            .next()
            .and_then(|n| n.trim_end_matches('.').parse::<u128>().ok());
        ApiErrorKind::AmountTooLow { min }
    } else if lower.contains("expected pending_deposit") || lower.starts_with("quote is in ") {
        ApiErrorKind::QuoteNotPending
    } else if lower.contains("quoting for this pair is not available") {
        ApiErrorKind::NoQuote
    } else if lower.contains("cannot parse signed intent") {
        ApiErrorKind::BadSignedIntent
    } else {
        ApiErrorKind::Other
    }
}

impl ApiError {
    /// Whether the same request may succeed if sent again later.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ApiErrorKind::RateLimited | ApiErrorKind::NoQuote | ApiErrorKind::Server
        )
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "1Click API HTTP {}: {}", self.status, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_messages() {
        let k = |status, msg: &str| {
            ApiError::from_response(status, format!(r#"{{"message":"{msg}"}}"#).as_bytes()).kind
        };
        assert_eq!(
            k(400, "Quoting for this pair is not available"),
            ApiErrorKind::NoQuote
        );
        assert_eq!(k(429, "Too Many Requests"), ApiErrorKind::RateLimited);
        assert_eq!(
            k(400, "deadline must be a valid ISO 8601 date string"),
            ApiErrorKind::DeadlineInvalid
        );
        assert_eq!(
            k(400, "Amount is too low for bridge, try at least 5909617"),
            ApiErrorKind::AmountTooLow {
                min: Some(5_909_617)
            }
        );
        assert_eq!(k(400, "something new"), ApiErrorKind::Other);
        let plain = ApiError::from_response(502, b"<html>bad gateway</html>");
        assert_eq!(plain.kind, ApiErrorKind::Server);
        assert_eq!(plain.message, "<html>bad gateway</html>");
        assert!(plain.is_retryable());
        assert!(!ApiError::from_response(401, b"{}").is_retryable());
    }
}
