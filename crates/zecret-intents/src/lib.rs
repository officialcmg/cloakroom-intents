//! The exit through NEAR Intents.
//!
//! After a swap, the taker's USDC is inside the Verifier (`intents.near`), owned by the
//! taker's fresh ed25519 key. This crate moves it to the token and chain the user
//! selected, with one signature and no NEAR gas, through the 1Click API:
//!
//! 1. [`oneclick`]: build `/v0/quote`, `/v0/generate-intent`, `/v0/submit-intent`,
//!    `/v0/status` and `/v0/tokens` requests and parse their answers.
//! 2. [`check`]: before signing, check that the payload 1Click returned moves exactly
//!    the approved amount of the approved token to the deposit address of a quote that
//!    1Click signed for the approved destination. Refuse otherwise.
//! 3. [`signer`]: sign with the `raw_ed25519` or `nep413` standard.
//! 4. [`direct`]: build and sign `transfer` and `ft_withdraw` intents without 1Click,
//!    for anyone to submit with `execute_intents`.
//!
//! The crate does no I/O. Every function builds an [`http::HttpRequest`] or parses an
//! [`http::HttpResponse`]; [`client`] chains them over a caller-provided
//! [`http::HttpTransport`]. No function reads the clock; "now" is an argument.

pub mod check;
pub mod client;
pub mod direct;
pub mod error;
pub mod http;
pub mod nonce;
pub mod oneclick;
pub mod payload;
pub mod quote_sig;
pub mod signer;
pub mod time;

pub use check::{
    ApprovedExit, CheckedIntent, Policy, Refusal, check_generated_intent, check_quote,
};
pub use client::{ExitClient, SubmittedExit, VerifierRpc};
pub use direct::IntentDraft;
pub use error::{ApiError, ApiErrorKind, Error, Result};
pub use http::{HttpRequest, HttpResponse, HttpTransport, Method};
pub use nonce::{Nonce, Salt};
pub use oneclick::{
    ExitQuoteParams, OneClick, QuoteRequest, QuoteResponse, StatusResponse, SwapStatus, TokenInfo,
};
pub use payload::{SignedPayload, Standard, UnsignedPayload};
pub use signer::IntentSigner;

/// Fills `buf` from the platform's random source (the OS natively, `crypto.
/// getRandomValues` in a browser).
pub fn random_bytes(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|e| Error::Invalid(format!("random source: {e}")))
}
