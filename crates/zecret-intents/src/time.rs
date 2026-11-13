//! Time values. The crate never reads a clock (`SystemTime::now` panics on
//! `wasm32-unknown-unknown`); every function that needs "now" takes it as an argument.

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

use crate::error::{Error, Result};

/// A UTC instant.
pub type Time = DateTime<Utc>;

/// Formats an instant the way the 1Click API and the Verifier expect:
/// RFC 3339 with milliseconds and `Z`, e.g. `2026-11-16T03:33:00.000Z`.
pub fn format(t: Time) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Parses an RFC 3339 instant.
pub fn parse(s: &str) -> Result<Time> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Parse(format!("timestamp {s:?}: {e}")))
}

/// An instant from Unix milliseconds (for callers that hold `Date.now()`).
pub fn from_unix_millis(ms: i64) -> Result<Time> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| Error::Invalid(format!("unix millis {ms} out of range")))
}

/// Nanoseconds since the Unix epoch, as the Verifier stores a nonce deadline.
pub(crate) fn unix_nanos(t: Time) -> Result<i64> {
    t.timestamp_nanos_opt()
        .ok_or_else(|| Error::Invalid(format!("{t} out of nanosecond range")))
}

pub(crate) fn from_unix_nanos(n: i64) -> Time {
    Utc.timestamp_nanos(n)
}
