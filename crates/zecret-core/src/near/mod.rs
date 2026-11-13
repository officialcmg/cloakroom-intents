//! The NEAR chain adapter: what the Cloakroom client did with `cast` on an EVM chain.
//!
//! - [`tx`]: borsh transactions, ed25519 keys and signatures.
//! - [`rpc`]: JSON-RPC (`block`, `query`, `send_tx`, `tx`) over the core's `Transport`.
//! - [`escrow`]: the escrow's calls and views, the token and `intents.near`.
//!
//! Protocol decisions read state at a final block ([`rpc::At::Final`]) and use final block
//! time as "now".

pub mod escrow;
pub mod rpc;
pub mod tx;

pub use escrow::{Escrow, SwapState, SwapView};
pub use rpc::{At, NearRpc, Signer, TxOutcome, WaitUntil};
pub use tx::{Action, NearKey, ONE_NEAR, TGAS};
