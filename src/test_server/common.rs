//! Common utilities for test servers

use anyhow::Result;
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// Test scenario types for controlling server behavior
#[derive(Debug, Clone, Copy)]
pub enum Scenario {
    /// Normal successful operation
    Ok,
    /// Require authentication (return 401/Unauthorized)
    AuthRequired,
    /// Return malformed/invalid response
    Malformed,
    /// Simulate timeout
    Timeout,
    /// GraphQL legacy websocket subscription protocol
    Legacy,
    /// Only tools/call sleeps then returns an error (used for daemon busy-session testing)
    ToolCallTimeout,
    /// tools/list succeeds once, then fails (used to verify cache-first help behavior)
    ToolsListFailAfterFirst,
    /// tools/call returns content + structuredContent payload
    StructuredContent,
    /// tools/call returns a JSON-RPC error with structured data
    ToolStructuredError,
    /// First resources/read fails, later reads succeed
    ResourceReadFailOnce,
    /// tools/list changes after navigate and emits tools/list_changed notification
    DynamicToolset,
    /// tools/call requires the arguments field to be present as an object, even when empty
    EmptyObjectRequired,
    /// JSON-RPC WebSocket pubsub shape without explicit unsubscribe method
    SuiPubSub,
    /// Resource state is scoped to a single MCP session/instance
    SessionScopedResource,
    /// uxc/can_reap returns can_reap=false with structured state
    CanReapKeepAlive,
    /// uxc/can_reap returns can_reap=true
    CanReapAllowReap,
    /// uxc/can_reap times out
    CanReapTimeout,
}

impl Scenario {
    /// Parse scenario from command-line argument
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "ok" => Ok(Self::Ok),
            "auth_required" => Ok(Self::AuthRequired),
            "malformed" => Ok(Self::Malformed),
            "timeout" => Ok(Self::Timeout),
            "legacy" => Ok(Self::Legacy),
            "tool_call_timeout" => Ok(Self::ToolCallTimeout),
            "tools_list_fail_after_first" => Ok(Self::ToolsListFailAfterFirst),
            "structured_content" => Ok(Self::StructuredContent),
            "tool_structured_error" => Ok(Self::ToolStructuredError),
            "resource_read_fail_once" => Ok(Self::ResourceReadFailOnce),
            "dynamic_toolset" => Ok(Self::DynamicToolset),
            "empty_object_required" => Ok(Self::EmptyObjectRequired),
            "sui_pubsub" => Ok(Self::SuiPubSub),
            "session_scoped_resource" => Ok(Self::SessionScopedResource),
            "can_reap_keep_alive" => Ok(Self::CanReapKeepAlive),
            "can_reap_allow_reap" => Ok(Self::CanReapAllowReap),
            "can_reap_timeout" => Ok(Self::CanReapTimeout),
            _ => anyhow::bail!(
                "Unknown scenario: {}. Use: ok, auth_required, malformed, timeout, legacy, tool_call_timeout, tools_list_fail_after_first, structured_content, tool_structured_error, resource_read_fail_once, dynamic_toolset, empty_object_required, sui_pubsub, session_scoped_resource, can_reap_keep_alive, can_reap_allow_reap, can_reap_timeout",
                s
            ),
        }
    }
}

/// Handle to a running test server
pub struct ServerHandle {
    pub addr: SocketAddr,
    pub shutdown: tokio::sync::oneshot::Sender<()>,
}

/// Bind to an available port on localhost
pub async fn bind_available() -> Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}

/// Write server address to a file for test discovery
pub fn write_addr_file(addr: SocketAddr, name: &str) -> Result<()> {
    let path = if let Ok(path) = std::env::var("UXC_TEST_SERVER_ADDR_FILE") {
        std::path::PathBuf::from(path)
    } else {
        let dir = std::env::var("UXC_TEST_SERVER_DIR")
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().to_string());
        std::fs::create_dir_all(&dir)?;
        std::path::PathBuf::from(dir).join(format!("{}.addr", name))
    };
    std::fs::write(path, addr.to_string())?;

    tracing::info!("Wrote server address for {} to {}", name, addr);

    Ok(())
}

/// Scenario timeout duration (milliseconds), configurable for tests
pub fn timeout_duration() -> std::time::Duration {
    let ms = std::env::var("UXC_TEST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30_000);
    std::time::Duration::from_millis(ms)
}
