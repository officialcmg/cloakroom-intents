//! Native parts of Zecret: the HTTP transport and file store for the core, and the setup
//! of a local NEAR sandbox. The `zecret` binary (src/main.rs) is built on these.

pub mod devnet;
pub mod io;
pub mod relay;

#[cfg(test)]
mod testnet_config;

#[cfg(test)]
mod mainnet_config;
