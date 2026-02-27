//! Local test servers for E2E testing
//!
//! This module provides standalone test servers that implement each protocol
//! with controllable scenarios (ok, auth_required, malformed, timeout).

pub mod common;
pub mod graphql;
pub mod grpc;
pub mod jsonrpc;
pub mod mcp_http;
pub mod mcp_stdio;
pub mod openapi;

pub use common::{Scenario, ServerHandle};
