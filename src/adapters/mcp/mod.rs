//! MCP (Model Context Protocol) adapter
//!
//! This module provides support for MCP servers via both stdio and HTTP transports.

pub mod client;
pub mod http_transport;
pub mod transport;
pub mod types;

use super::{Adapter, ExecutionResult, Operation, OperationDetail, ProtocolType};
use crate::auth::Profile;
use crate::error::UxcError;
use anyhow::{bail, Result};
use async_trait::async_trait;
pub use client::McpStdioClient;
pub use http_transport::McpHttpTransport;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};
#[cfg(test)]
pub use transport::MockStdioExecutor;

pub struct McpAdapter {
    cache: Option<Arc<dyn crate::cache::Cache>>,
    auth_profile: Option<Profile>,
    discovered_http_endpoints: Arc<RwLock<HashMap<String, String>>>,
    last_probe_diagnostics: Arc<RwLock<Option<String>>>,
    service_metadata: Arc<RwLock<HashMap<String, McpServiceMetadata>>>,
}

#[derive(Debug, Clone, Default)]
pub struct McpServiceMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
}

impl McpAdapter {
    pub fn new() -> Self {
        Self {
            cache: None,
            auth_profile: None,
            discovered_http_endpoints: Arc::new(RwLock::new(HashMap::new())),
            last_probe_diagnostics: Arc::new(RwLock::new(None)),
            service_metadata: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_cache(mut self, cache: Arc<dyn crate::cache::Cache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn with_auth(mut self, profile: Profile) -> Self {
        self.auth_profile = Some(profile);
        self
    }

    /// Check if a URL/command looks like an MCP stdio command
    pub fn is_stdio_command(url: &str) -> bool {
        // Check if it looks like a command (not a URL)
        // URLs have schemes like http://, https://, etc.
        // Commands start with executable names or paths
        let lower = url.to_lowercase();

        // HTTP(S) URLs use HTTP transport, not stdio
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return false;
        }

        // mcp:// URLs use stdio transport (backward compatibility)
        if lower.starts_with("mcp://") {
            return true;
        }

        // Check for common command patterns
        // - Contains spaces (command with args)
        // - Starts with common shell metacharacters
        // - Contains executable patterns
        url.contains(' ')
            || url.starts_with("./")
            || url.starts_with('/')
            || url.starts_with("npx ")
            || url.starts_with("node ")
            || url.starts_with("python ")
            || url.starts_with("python3 ")
            || url.contains("\\") // Windows path
    }

    /// Check if a URL is an HTTP MCP endpoint
    pub fn is_http_url(url: &str) -> bool {
        let lower = url.to_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    }

    /// Parse a stdio command into the command and arguments
    pub fn parse_stdio_command(url: &str) -> Result<(String, Vec<String>)> {
        let parts = self::transport::parse_command(url);
        if parts.is_empty() {
            bail!("Empty command");
        }

        let (cmd, args) = parts.split_first().unwrap();
        Ok((cmd.clone(), args.to_vec()))
    }

    fn normalize_http_url(url: &str) -> String {
        url.trim_end_matches('/').to_string()
    }

    fn http_endpoint_candidates(url: &str) -> Vec<String> {
        let normalized = Self::normalize_http_url(url);
        let mut candidates = vec![normalized.clone()];

        if let Ok(parsed) = url::Url::parse(&normalized) {
            let path = parsed.path();
            if path.is_empty() || path == "/" {
                candidates.push(format!("{}/mcp", normalized));
                candidates.push(format!("{}/.well-known/mcp", normalized));
            }
        }

        candidates.sort();
        candidates.dedup();
        candidates
    }

    async fn resolve_http_endpoint(&self, url: &str) -> Result<Option<String>> {
        let normalized = Self::normalize_http_url(url);
        {
            let mut diag = self.last_probe_diagnostics.write().await;
            *diag = None;
        }
        {
            let cache = self.discovered_http_endpoints.read().await;
            if let Some(endpoint) = cache.get(&normalized) {
                return Ok(Some(endpoint.clone()));
            }
        }

        let mut reasons = Vec::new();
        for candidate in Self::http_endpoint_candidates(url) {
            match McpHttpTransport::probe_initialize_with_reason(
                &candidate,
                self.auth_profile.clone(),
            )
            .await
            {
                Ok(http_transport::ProbeInitializeOutcome::Success) => {
                    let mut cache = self.discovered_http_endpoints.write().await;
                    cache.insert(normalized, candidate.clone());
                    return Ok(Some(candidate));
                }
                Ok(http_transport::ProbeInitializeOutcome::AuthFailed(failure)) => {
                    let detail = format!(
                        "MCP authentication probe failed for {}: {}",
                        candidate, failure.message
                    );
                    return match failure.code {
                        http_transport::ProbeAuthFailureCode::OAuthRequired => {
                            Err(UxcError::OAuthRequired(detail).into())
                        }
                        http_transport::ProbeAuthFailureCode::OAuthRefreshFailed => {
                            Err(UxcError::OAuthRefreshFailed(detail).into())
                        }
                    };
                }
                Ok(http_transport::ProbeInitializeOutcome::NotMcp(reason)) => {
                    reasons.push(format!("{} => {}", candidate, reason));
                }
                Err(err) => reasons.push(format!("{} => {}", candidate, err)),
            }
        }

        if !reasons.is_empty() {
            let mut diag = self.last_probe_diagnostics.write().await;
            *diag = Some(reasons.join("; "));
        }

        Ok(None)
    }

    pub async fn latest_probe_diagnostics(&self) -> Option<String> {
        self.last_probe_diagnostics.read().await.clone()
    }

    async fn cache_service_metadata(
        &self,
        url: &str,
        server_info: Option<&types::ServerInfo>,
        instructions: Option<&str>,
    ) {
        let metadata = McpServiceMetadata {
            name: server_info.map(|info| info.name.clone()),
            description: instructions.map(ToString::to_string),
        };
        let mut map = self.service_metadata.write().await;
        map.insert(url.to_string(), metadata);
    }

    pub async fn service_metadata_for(&self, url: &str) -> Option<McpServiceMetadata> {
        self.service_metadata.read().await.get(url).cloned()
    }
}

impl Default for McpAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for McpAdapter {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::Mcp
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        // First, check if it's a stdio command
        if Self::is_stdio_command(url) {
            return Ok(true);
        }

        if Self::is_http_url(url) {
            return Ok(self.resolve_http_endpoint(url).await?.is_some());
        }

        Ok(false)
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        // Try cache first if available
        if let Some(cache) = &self.cache {
            match cache.get(url)? {
                crate::cache::CacheResult::Hit(schema) => {
                    debug!("MCP cache hit for: {}", url);
                    return Ok(schema);
                }
                crate::cache::CacheResult::Bypassed => {
                    debug!("MCP cache bypassed for: {}", url);
                }
                crate::cache::CacheResult::Miss => {
                    debug!("MCP cache miss for: {}", url);
                }
            }
        }

        // If it's a stdio command, connect and get server info
        if Self::is_stdio_command(url) {
            let (cmd, args) = Self::parse_stdio_command(url)?;
            let client = McpStdioClient::connect(&cmd, &args).await?;
            let server_info = client.server_info().cloned();
            let instructions = client.instructions().map(ToString::to_string);

            // Build schema from server capabilities
            let schema = serde_json::json!({
                "protocol": "MCP",
                "protocolVersion": "2024-11-05",
                "transport": "stdio",
                "command": cmd,
                "serverInfo": server_info,
                "instructions": instructions,
                "capabilities": {
                    "tools": client.supports_tools(),
                    "resources": client.supports_resources(),
                    "prompts": client.supports_prompts(),
                }
            });

            // Store in cache if available
            if let Some(cache) = &self.cache {
                if let Err(e) = cache.put(url, &schema) {
                    debug!("Failed to cache MCP schema: {}", e);
                } else {
                    info!("Cached MCP schema for: {}", url);
                }
            }

            return Ok(schema);
        }

        // For HTTP-based MCP, connect and get server info
        if Self::is_http_url(url) {
            let endpoint = self.resolve_http_endpoint(url).await?.ok_or_else(|| {
                anyhow::anyhow!("Unable to discover MCP HTTP endpoint for {}", url)
            })?;
            let transport = McpHttpTransport::with_auth(endpoint, self.auth_profile.clone())?;
            let init_result = transport.initialize().await?;

            let schema = serde_json::json!({
                "protocol": "MCP",
                "protocolVersion": "2024-11-05",
                "transport": "http",
                "url": url,
                "serverInfo": init_result.serverInfo,
                "instructions": init_result.instructions,
                "capabilities": init_result.capabilities
            });

            // Store in cache if available
            if let Some(cache) = &self.cache {
                if let Err(e) = cache.put(url, &schema) {
                    debug!("Failed to cache MCP schema: {}", e);
                } else {
                    info!("Cached MCP schema for: {}", url);
                }
            }

            return Ok(schema);
        }

        // Default fallback for mcp:// URLs
        let schema = serde_json::json!({
            "protocol": "MCP",
            "protocolVersion": "2024-11-05",
            "transport": "stdio",
            "url": url
        });

        Ok(schema)
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        if Self::is_stdio_command(url) {
            let (cmd, args) = Self::parse_stdio_command(url)?;
            let mut client = McpStdioClient::connect(&cmd, &args).await?;
            self.cache_service_metadata(url, client.server_info(), client.instructions())
                .await;

            // List tools as operations
            let tools = client.list_tools().await?;

            let operations = tools
                .into_iter()
                .map(|tool| {
                    let parameters = if let Some(schema) = tool.inputSchema {
                        // Convert JSON Schema to our Parameter format
                        parse_schema_to_parameters(&schema)
                    } else {
                        Vec::new()
                    };

                    Operation {
                        operation_id: tool.name.clone(),
                        display_name: tool.name.clone(),
                        description: Some(tool.description),
                        parameters,
                        return_type: Some("ToolContent".to_string()),
                    }
                })
                .collect();

            return Ok(operations);
        }

        // For HTTP-based MCP
        if Self::is_http_url(url) {
            let endpoint = self.resolve_http_endpoint(url).await?.ok_or_else(|| {
                anyhow::anyhow!("Unable to discover MCP HTTP endpoint for {}", url)
            })?;
            let transport = McpHttpTransport::with_auth(endpoint, self.auth_profile.clone())?;
            let init_result = transport.initialize().await?;
            self.cache_service_metadata(
                url,
                init_result.serverInfo.as_ref(),
                init_result.instructions.as_deref(),
            )
            .await;
            let tools = transport.list_tools().await?;

            let operations = tools
                .into_iter()
                .map(|tool| {
                    let parameters = if let Some(schema) = tool.inputSchema {
                        parse_schema_to_parameters(&schema)
                    } else {
                        Vec::new()
                    };

                    Operation {
                        operation_id: tool.name.clone(),
                        display_name: tool.name.clone(),
                        description: Some(tool.description),
                        parameters,
                        return_type: Some("ToolContent".to_string()),
                    }
                })
                .collect();

            return Ok(operations);
        }

        // Default fallback
        Ok(Vec::new())
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        if Self::is_stdio_command(url) {
            let (cmd, args) = Self::parse_stdio_command(url)?;
            let mut client = McpStdioClient::connect(&cmd, &args).await?;

            let tools = client.list_tools().await?;

            for tool in tools {
                if tool.name == operation {
                    return Ok(OperationDetail {
                        operation_id: tool.name.clone(),
                        display_name: tool.name,
                        description: Some(tool.description),
                        parameters: tool
                            .inputSchema
                            .as_ref()
                            .map(parse_schema_to_parameters)
                            .unwrap_or_default(),
                        return_type: Some("ToolContent".to_string()),
                        input_schema: tool.inputSchema,
                    });
                }
            }

            bail!("Tool '{}' not found", operation);
        }

        // For HTTP-based MCP
        if Self::is_http_url(url) {
            let endpoint = self.resolve_http_endpoint(url).await?.ok_or_else(|| {
                anyhow::anyhow!("Unable to discover MCP HTTP endpoint for {}", url)
            })?;
            let transport = McpHttpTransport::with_auth(endpoint, self.auth_profile.clone())?;
            transport.initialize().await?;
            let tools = transport.list_tools().await?;

            for tool in tools {
                if tool.name == operation {
                    return Ok(OperationDetail {
                        operation_id: tool.name.clone(),
                        display_name: tool.name,
                        description: Some(tool.description),
                        parameters: tool
                            .inputSchema
                            .as_ref()
                            .map(parse_schema_to_parameters)
                            .unwrap_or_default(),
                        return_type: Some("ToolContent".to_string()),
                        input_schema: tool.inputSchema,
                    });
                }
            }

            bail!("Tool '{}' not found", operation);
        }

        bail!("Operation '{}' not found", operation);
    }

    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();

        if Self::is_stdio_command(url) {
            let (cmd, args_list) = Self::parse_stdio_command(url)?;
            let mut client = McpStdioClient::connect(&cmd, &args_list).await?;

            // Build arguments JSON
            let arguments = if args.is_empty() {
                None
            } else {
                Some(Value::Object(args.into_iter().collect()))
            };

            let result = client.call_tool(operation, arguments).await?;

            // Convert tool content to a simple JSON output
            let output = convert_tool_content_to_value(&result.content);

            return Ok(ExecutionResult {
                data: output,
                metadata: super::ExecutionMetadata {
                    duration_ms: start.elapsed().as_millis() as u64,
                    operation: operation.to_string(),
                },
            });
        }

        // For HTTP-based MCP
        if Self::is_http_url(url) {
            let endpoint = self.resolve_http_endpoint(url).await?.ok_or_else(|| {
                anyhow::anyhow!("Unable to discover MCP HTTP endpoint for {}", url)
            })?;
            let transport = McpHttpTransport::with_auth(endpoint, self.auth_profile.clone())?;
            transport.initialize().await?;

            // Build arguments JSON
            let arguments = if args.is_empty() {
                None
            } else {
                Some(Value::Object(args.into_iter().collect()))
            };

            let result = transport.call_tool(operation, arguments).await?;

            // Convert tool content to a simple JSON output
            let output = convert_tool_content_to_value(&result.content);

            return Ok(ExecutionResult {
                data: output,
                metadata: super::ExecutionMetadata {
                    duration_ms: start.elapsed().as_millis() as u64,
                    operation: operation.to_string(),
                },
            });
        }

        bail!("Unsupported MCP URL format: {}", url)
    }
}

/// Parse JSON Schema to our Parameter format
fn parse_schema_to_parameters(schema: &Value) -> Vec<super::Parameter> {
    let mut parameters = Vec::new();

    if let Some(obj) = schema.as_object() {
        if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
            let required = obj
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<std::collections::HashSet<_>>()
                })
                .unwrap_or_default();

            for (name, prop_schema) in props {
                let param_type = prop_schema
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let description = prop_schema
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                parameters.push(super::Parameter {
                    name: name.clone(),
                    param_type,
                    required: required.contains(name.as_str()),
                    description,
                });
            }
        }
    }

    parameters
}

/// Convert tool content to a JSON value for output
fn convert_tool_content_to_value(content: &[types::ToolContent]) -> Value {
    let mut results = Vec::new();

    for item in content {
        let value = match item {
            types::ToolContent::Text { text } => serde_json::json!({
                "type": "text",
                "text": text
            }),
            types::ToolContent::Image { data, mimeType } => serde_json::json!({
                "type": "image",
                "data": data,
                "mimeType": mimeType
            }),
            types::ToolContent::Resource {
                uri,
                mimeType,
                text,
                blob,
            } => {
                let mut obj = serde_json::json!({
                    "type": "resource",
                    "uri": uri
                });
                if let Some(mt) = mimeType {
                    obj["mimeType"] = serde_json::json!(mt);
                }
                if let Some(t) = text {
                    obj["text"] = serde_json::json!(t);
                }
                if let Some(b) = blob {
                    obj["blob"] = serde_json::json!(b);
                }
                obj
            }
        };
        results.push(value);
    }

    serde_json::json!({ "content": results })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_response() -> &'static str {
        r#"{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {}
    },
    "serverInfo": {
      "name": "mock-mcp",
      "version": "1.0.0"
    }
  }
}"#
    }

    #[tokio::test]
    async fn can_handle_discovers_host_level_http_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _root = server
            .mock("POST", "/")
            .with_status(404)
            .create_async()
            .await;
        let _well_known = server
            .mock("POST", "/.well-known/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _mcp = server
            .mock("POST", "/mcp")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(initialize_response())
            .create_async()
            .await;

        let adapter = McpAdapter::new();
        assert!(adapter.can_handle(&server.url()).await.unwrap());

        let resolved = adapter
            .resolve_http_endpoint(&server.url())
            .await
            .unwrap()
            .unwrap();
        assert!(resolved.ends_with("/mcp"));
    }
}
