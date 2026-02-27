//! UXC - Universal X-Protocol Call
//!
//! Schema-driven, multi-protocol RPC execution runtime.

#![allow(non_camel_case_types)]

pub mod adapters;
pub mod auth;
pub mod cache;
pub mod cli;
pub mod error;
pub mod http_client;
pub mod output;
pub mod protocol;
pub mod schema_mapping;

#[cfg(feature = "test-server")]
pub mod test_server;

pub use adapters::{Adapter, ProtocolType};
pub use cache::{create_cache, create_default_cache, Cache, CacheConfig, CacheResult};
pub use error::{Result, UxcError};
pub use output::OutputEnvelope;

/// UXC version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// UXC library initialization
pub fn init() {
    // Initialize logging, etc.
}
