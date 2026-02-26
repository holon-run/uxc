//! Local test servers for E2E testing
//!
//! This module provides standalone test servers that implement each protocol
//! with controllable scenarios (ok, auth_required, malformed, timeout).

pub mod common;
pub mod graphql;
pub mod jsonrpc;
pub mod openapi;

pub use common::{Scenario, ServerHandle};
