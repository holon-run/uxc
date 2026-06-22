//! UXC - Universal X-Protocol CLI
//!
//! Schema-driven, multi-protocol RPC execution runtime.

#![allow(non_camel_case_types)]

pub mod adapters;
pub mod arg_coercion;
pub mod auth;
pub mod cache;
pub mod cli;
pub mod codegen;
pub mod config_import;
pub mod daemon;
pub mod daemon_client;
pub mod daemon_log;
pub mod email;
pub mod error;
pub mod http_client;
pub mod managed_source_streams;
pub mod output;
pub mod protocol;
pub mod schema_mapping;
pub mod subscription_discord;
pub mod subscription_email;
pub mod subscription_feishu;
pub mod subscription_graphql;
pub mod subscription_jsonrpc;
pub mod subscription_poll;
pub mod subscription_slack;
pub mod subscription_websocket;

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
