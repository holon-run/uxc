//! Supported in-process MCP client API. No UXC CLI or daemon is required.
//!
//! Use one mutable session serially on a Tokio runtime. Dropping the client is
//! best effort; await [`McpClient::close`] for explicit cleanup.

mod events;

pub use events::{
    EventDescriptor, EventItem, ListEventsParams, ListEventsResult, PollEventsParams,
    PollEventsResult,
};

use crate::adapters::mcp::types::{ServerCapabilities, ServerInfo, Tool};
use crate::adapters::mcp::{McpHttpTransport, McpStdioClient, StdioSpawnOptions, TransportFailure};
use crate::error::StructuredError;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
use tokio::time::timeout;

/// Endpoint configuration. Stdio executes a program directly, without a shell.
/// Deliberately not `Debug`: arguments, environment values and URLs may be secret.
#[non_exhaustive]
pub enum McpEndpoint {
    /// Local child process; environment overrides are added to the inherited environment.
    Stdio {
        /// Executable name or path.
        program: String,
        /// Individual arguments, not a shell command line.
        args: Vec<String>,
        /// Overrides applied only to the child.
        env_overrides: HashMap<String, String>,
    },
    /// Explicit Streamable HTTP endpoint. No legacy SSE transport or redirects.
    StreamableHttp {
        /// Complete `http` or `https` endpoint URL.
        url: String,
        /// Explicit authentication selection.
        auth: McpHttpAuth,
    },
}

/// Embedded HTTP authentication. There is no implicit host binding or login.
#[derive(Default)]
#[non_exhaustive]
pub enum McpHttpAuth {
    /// Do not read any UXC authentication configuration.
    #[default]
    Anonymous,
    /// Load and materialize the named UXC credential profile.
    Profile(String),
}

/// Independent, nonzero operation budgets.
#[derive(Debug, Clone, Copy)]
pub struct McpTimeouts {
    /// Endpoint setup and protocol negotiation.
    pub connect: Duration,
    /// One API request, including all pages or supporting RPCs.
    pub request: Duration,
    /// Explicit session cleanup.
    pub close: Duration,
}

impl Default for McpTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(30),
            request: Duration::from_secs(30),
            close: Duration::from_secs(5),
        }
    }
}

/// Connection options.
pub struct McpConnectOptions {
    /// Transport and its configuration.
    pub endpoint: McpEndpoint,
    /// Connect, request and close deadlines.
    pub timeouts: McpTimeouts,
}

impl McpConnectOptions {
    /// Configure an endpoint with the default timeouts.
    pub fn new(endpoint: McpEndpoint) -> Self {
        Self {
            endpoint,
            timeouts: McpTimeouts::default(),
        }
    }
}

/// Redacted client error. No raw adapter error, URL, or response is retained.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum McpError {
    /// Invalid settings or a local program that could not be started.
    #[error("invalid MCP client configuration")]
    Configuration,
    /// Missing or unusable credentials, failed refresh or rejected authorization.
    #[error("MCP authentication failed")]
    Authentication,
    /// The configured operation budget was exceeded.
    #[error("MCP operation timed out")]
    Timeout,
    /// The connection or session is no longer usable.
    #[error("MCP session disconnected")]
    Disconnected,
    /// MCP handshake, response, or HTTP status failed.
    #[error("MCP protocol error (JSON-RPC code: {json_rpc_code:?})")]
    Protocol {
        /// Numeric JSON-RPC error code, if available.
        json_rpc_code: Option<i64>,
    },
    /// The server did not negotiate a required capability.
    #[error("MCP capability is not supported")]
    UnsupportedCapability,
}

impl McpError {
    fn protocol() -> Self {
        Self::Protocol {
            json_rpc_code: None,
        }
    }

    fn from_transport(err: anyhow::Error) -> Self {
        if let Some(failure) = err.downcast_ref::<TransportFailure>() {
            return match failure {
                TransportFailure::Authentication => Self::Authentication,
                TransportFailure::StdioTimeout(..) => Self::Timeout,
                TransportFailure::StdioDisconnected(_) | TransportFailure::SessionExpired => {
                    Self::Disconnected
                }
                TransportFailure::HttpStatus(status) => match status.as_u16() {
                    401 | 403 => Self::Authentication,
                    _ => Self::protocol(),
                },
                TransportFailure::Rpc(code) => Self::Protocol {
                    json_rpc_code: Some(*code),
                },
            };
        }
        if let Some(err) = err.downcast_ref::<reqwest::Error>() {
            return if err.is_timeout() {
                Self::Timeout
            } else if err.is_builder() {
                Self::Configuration
            } else {
                Self::Disconnected
            };
        }
        if let Some(err) = err.downcast_ref::<StructuredError>() {
            return Self::Protocol {
                json_rpc_code: err
                    .details
                    .as_ref()
                    .and_then(|d| d["jsonrpc_code"].as_i64()),
            };
        }
        Self::protocol()
    }
}

/// Server identity reported during negotiation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct McpServerInfo {
    /// Server implementation name.
    pub name: String,
    /// Server implementation version.
    pub version: String,
}

/// Explicitly modeled server capabilities. These are advertisements, not grants.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct McpCapabilities {
    /// The server advertises tools.
    pub tools: bool,
    /// The server advertises resources.
    pub resources: bool,
    /// The server advertises prompts.
    pub prompts: bool,
    /// The server advertises the MCP Events extension.
    pub events: bool,
}

impl McpCapabilities {
    fn from_adapter(value: &ServerCapabilities) -> Self {
        Self {
            tools: value.tools.is_some(),
            resources: value.resources.is_some(),
            prompts: value.prompts.is_some(),
            events: value.supports_events(),
        }
    }
}

/// Discovery snapshot captured during connection.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct McpDiscovery {
    /// Identity, if supplied by the server.
    pub server_info: Option<McpServerInfo>,
    /// Negotiated MCP protocol version.
    pub protocol_version: String,
    /// Server capabilities advertised at handshake.
    pub capabilities: McpCapabilities,
    /// Optional server instructions. Treat them as untrusted content.
    pub instructions: Option<String>,
}

impl McpDiscovery {
    fn new(
        version: &str,
        server: Option<&ServerInfo>,
        capabilities: McpCapabilities,
        instructions: Option<&str>,
    ) -> Self {
        Self {
            server_info: server.map(|server| McpServerInfo {
                name: server.name.clone(),
                version: server.version.clone(),
            }),
            protocol_version: version.to_owned(),
            capabilities,
            instructions: instructions.map(str::to_owned),
        }
    }
}

/// A discovered tool. Calling it is outside this API's scope.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct McpTool {
    /// Stable tool name within this server's catalog.
    pub name: String,
    /// Optional display title.
    pub title: Option<String>,
    /// Optional untrusted server description.
    pub description: Option<String>,
    /// Dynamic input JSON Schema.
    pub input_schema: Option<Value>,
    /// Dynamic output JSON Schema, if provided.
    pub output_schema: Option<Value>,
}

impl McpTool {
    fn from_adapter(tool: Tool) -> Self {
        Self {
            name: tool.name,
            title: tool.title,
            description: tool.description,
            input_schema: tool.inputSchema,
            output_schema: tool.outputSchema,
        }
    }
}

enum Session {
    Stdio(Box<McpStdioClient>),
    Http(Box<McpHttpTransport>),
}

/// A negotiated MCP session. No requests are retried or replayed.
///
/// Reconnect explicitly after timeout, disconnect, or cancellation of a request
/// future; its server-side outcome may be unknown. Close the old client first
/// if possible. Discovery is a snapshot, not a liveness check.
pub struct McpClient {
    session: Session,
    discovery: McpDiscovery,
    timeouts: McpTimeouts,
    usable: bool,
}

impl McpClient {
    /// Connect and negotiate one stdio or Streamable HTTP session.
    pub async fn connect(options: McpConnectOptions) -> Result<Self, McpError> {
        let timeouts = options.timeouts;
        if timeouts.connect.is_zero() || timeouts.request.is_zero() || timeouts.close.is_zero() {
            return Err(McpError::Configuration);
        }
        timeout(timeouts.connect, Self::connect_inner(options))
            .await
            .map_err(|_| McpError::Timeout)?
    }

    async fn connect_inner(options: McpConnectOptions) -> Result<Self, McpError> {
        let timeouts = options.timeouts;
        let (session, discovery) = match options.endpoint {
            McpEndpoint::Stdio {
                program,
                args,
                env_overrides,
            } => {
                if program.is_empty() {
                    return Err(McpError::Configuration);
                }
                let transport = McpStdioClient::connect_embedded(
                    &program,
                    &args,
                    StdioSpawnOptions {
                        env_overrides: env_overrides.into_iter().collect(),
                        cwd: None,
                    },
                    timeouts.connect,
                )
                .await
                .map_err(|err| {
                    if err.downcast_ref::<std::io::Error>().is_some() {
                        McpError::Configuration
                    } else {
                        McpError::from_transport(err)
                    }
                })?;
                let discovery = McpDiscovery::new(
                    transport.protocol_version(),
                    transport.server_info(),
                    transport
                        .server_capabilities()
                        .map(McpCapabilities::from_adapter)
                        .unwrap_or_default(),
                    transport.instructions(),
                );
                (Session::Stdio(Box::new(transport)), discovery)
            }
            McpEndpoint::StreamableHttp { url, auth } => {
                let parsed = url::Url::parse(&url).map_err(|_| McpError::Configuration)?;
                if !matches!(parsed.scheme(), "http" | "https")
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || parsed.fragment().is_some()
                {
                    return Err(McpError::Configuration);
                }
                let profile = match auth {
                    McpHttpAuth::Anonymous => None,
                    McpHttpAuth::Profile(name) => {
                        // Loading/materializing may involve blocking local secret providers.
                        tokio::task::spawn_blocking(move || {
                            crate::auth::resolve_auth_for_endpoint("", Some(name))
                        })
                        .await
                        .map_err(|_| McpError::Authentication)?
                        .map_err(|_| McpError::Authentication)?
                    }
                };
                let mut transport = McpHttpTransport::embedded(
                    url,
                    profile,
                    timeouts.connect.max(timeouts.request).max(timeouts.close),
                )
                .map_err(|_| McpError::Configuration)?;
                let result = transport
                    .initialize_embedded()
                    .await
                    .map_err(McpError::from_transport)?;
                let discovery = McpDiscovery::new(
                    &result.protocolVersion,
                    result.serverInfo.as_ref(),
                    McpCapabilities::from_adapter(&result.capabilities),
                    result.instructions.as_deref(),
                );
                (Session::Http(Box::new(transport)), discovery)
            }
        };
        Ok(Self {
            session,
            discovery,
            timeouts,
            usable: true,
        })
    }

    /// Return the discovery snapshot without performing any RPC.
    pub async fn discover(&mut self) -> Result<McpDiscovery, McpError> {
        Ok(self.discovery.clone())
    }

    /// List tools, transparently following catalog pages within one request budget.
    /// Returns an empty catalog if the server does not advertise tools.
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>, McpError> {
        let budget = self.start_request()?;
        if !self.discovery.capabilities.tools {
            self.usable = true;
            return Ok(Vec::new());
        }
        let result = timeout(budget, async {
            match &mut self.session {
                Session::Stdio(transport) => transport.list_tools_with_timeout(budget).await,
                Session::Http(transport) => transport.list_tools().await,
            }
        })
        .await;
        self.finish_request(result)
            .map(|tools| tools.into_iter().map(McpTool::from_adapter).collect())
    }

    /// Consume the session. Stdio closes stdin, then kills and reaps the direct
    /// child if necessary. Stateful HTTP attempts a single DELETE; 404 and 405
    /// are accepted. Local resources are released even if close fails.
    pub async fn close(mut self) -> Result<(), McpError> {
        let budget = self.timeouts.close;
        timeout(budget, async {
            match &mut self.session {
                Session::Stdio(transport) => transport.kill_and_wait(budget / 2).await,
                Session::Http(transport) => transport.close_embedded().await,
            }
        })
        .await
        .map_err(|_| McpError::Timeout)?
        .map_err(McpError::from_transport)
    }

    fn start_request(&mut self) -> Result<Duration, McpError> {
        if !self.usable {
            return Err(McpError::Disconnected);
        }
        // If the caller drops the future, do not reuse the session implicitly.
        self.usable = false;
        Ok(self.timeouts.request)
    }

    fn finish_request<T>(
        &mut self,
        value: Result<anyhow::Result<T>, tokio::time::error::Elapsed>,
    ) -> Result<T, McpError> {
        let result = value
            .map_err(|_| McpError::Timeout)
            .and_then(|result| result.map_err(McpError::from_transport));
        self.usable = !matches!(result, Err(McpError::Timeout | McpError::Disconnected));
        result
    }
}
