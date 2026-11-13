//! The Zecret taker engine for a browser Web Worker.
//!
//! This crate puts `zecret-core`'s taker and the exit through NEAR Intents
//! (`zecret-intents`) behind a small interface that a Web Worker calls through
//! wasm-bindgen (`web`, built for `wasm32-unknown-unknown` only). The worker
//! (`web/src/engine/real/`) owns the clock, IndexedDB and the schedule; every call here
//! blocks the worker until it is done (network calls are synchronous `XMLHttpRequest`s,
//! and one shielded send proves for about 25 s).
//!
//! - [`session`]: the taker over a `MemStore` whose changed records the worker writes to
//!   IndexedDB after every call, before it calls again.
//! - [`exit`]: the exit through 1Click (firm quote, safety check, signature, submission,
//!   status) and the direct exit through the Verifier (`ft_withdraw` signed by the taker
//!   key and sent with `execute_intents` by the taker's own NEAR account).
//! - [`transport`]: an adapter from the core's POST-only transport to the HTTP interface
//!   of `zecret-intents`.
//!
//! Everything except `web` builds natively too, so it is tested without a browser.

pub mod exit;
pub mod session;
pub mod transport;

#[cfg(target_arch = "wasm32")]
mod web;
