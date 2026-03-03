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
    force_refresh_schema: bool,
    discovered_http_endpoints: Arc<RwLock<HashMap<String, String>>>,
    last_probe_diagnostics: Arc<RwLock<Option<String>>>,
}

impl McpAdapter {
    pub fn new() -> Self {
        Self {
            cache: None,
            auth_profile: None,
            force_refresh_schema: false,
            discovered_http_endpoints: Arc::new(RwLock::new(HashMap::new())),
            last_probe_diagnostics: Arc::new(RwLock::new(None)),
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

    pub fn with_refresh_schema(mut self, refresh: bool) -> Self {
        self.force_refresh_schema = refresh;
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

    fn tools_from_schema(schema: &Value) -> Option<Vec<types::Tool>> {
        let tools = schema.get("tools")?.as_array()?;
        Some(
            tools
                .iter()
                .filter_map(|tool| serde_json::from_value::<types::Tool>(tool.clone()).ok())
                .collect::<Vec<_>>(),
        )
    }

    fn validate_required_args(
        tool_name: &str,
        input_schema: Option<&Value>,
        args: &HashMap<String, Value>,
    ) -> Result<()> {
        let required = input_schema
            .and_then(|schema| schema.get("required"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let missing = required
            .into_iter()
            .filter(|key| !args.contains_key(key))
            .collect::<Vec<_>>();

        if missing.is_empty() {
            return Ok(());
        }

        Err(UxcError::InvalidArguments(format!(
            "Missing required arguments for MCP tool '{}': {}",
            tool_name,
            missing.join(", ")
        ))
        .into())
    }

    async fn validate_tool_call(
        &self,
        url: &str,
        operation: &str,
        args: &HashMap<String, Value>,
    ) -> Result<()> {
        let schema = self.fetch_schema(url).await?;
        let Some(tools) = Self::tools_from_schema(&schema) else {
            // Skip local validation when tool catalog is unavailable.
            return Ok(());
        };
        let tool = tools
            .iter()
            .find(|tool| tool.name == operation)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;

        Self::validate_required_args(operation, tool.inputSchema.as_ref(), args)
    }

    async fn tools_from_schema_or_refresh(&self, url: &str) -> Result<Vec<types::Tool>> {
        let schema = self.fetch_schema(url).await?;
        if let Some(tools) = Self::tools_from_schema(&schema) {
            return Ok(tools);
        }

        if !self.force_refresh_schema {
            let schema = self.fetch_schema_internal(url, false).await?;
            if let Some(tools) = Self::tools_from_schema(&schema) {
                return Ok(tools);
            }
        }

        bail!(
            "MCP tool catalog unavailable for endpoint '{}'; retry with --refresh-schema",
            url
        )
    }

    async fn fetch_schema_internal(&self, url: &str, allow_cache_read: bool) -> Result<Value> {
        if allow_cache_read {
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
        }

        // If it's a stdio command, connect and get server info
        if Self::is_stdio_command(url) {
            let (cmd, args) = Self::parse_stdio_command(url)?;
            let mut client = McpStdioClient::connect(&cmd, &args).await?;
            let server_info = client.server_info().cloned();
            let instructions = client.instructions().map(ToString::to_string);
            let tools = match client.list_tools().await {
                Ok(tools) => Some(tools),
                Err(err) => {
                    debug!("MCP stdio list_tools failed while building schema: {}", err);
                    None
                }
            };

            // Build schema from server capabilities
            let mut schema = serde_json::json!({
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
            if let Some(tools) = tools {
                schema["tools"] = serde_json::json!(tools);
            }

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
            let tools = match transport.list_tools().await {
                Ok(tools) => Some(tools),
                Err(err) => {
                    debug!("MCP HTTP list_tools failed while building schema: {}", err);
                    None
                }
            };

            let mut schema = serde_json::json!({
                "protocol": "MCP",
                "protocolVersion": "2024-11-05",
                "transport": "http",
                "url": url,
                "serverInfo": init_result.serverInfo,
                "instructions": init_result.instructions,
                "capabilities": init_result.capabilities
            });
            if let Some(tools) = tools {
                schema["tools"] = serde_json::json!(tools);
            }

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
        Ok(serde_json::json!({
            "protocol": "MCP",
            "protocolVersion": "2024-11-05",
            "transport": "stdio",
            "url": url
        }))
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
        self.fetch_schema_internal(url, !self.force_refresh_schema)
            .await
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        let tools = self.tools_from_schema_or_refresh(url).await?;
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
        Ok(operations)
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        let tools = self.tools_from_schema_or_refresh(url).await?;

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

    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        self.validate_tool_call(url, operation, &args).await?;

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

    #[test]
    fn validate_required_args_detects_missing_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["query", "limit"]
        });
        let mut args = HashMap::new();
        args.insert("query".to_string(), serde_json::json!("rust"));

        let err = McpAdapter::validate_required_args("search", Some(&schema), &args).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Missing required arguments"));
        assert!(message.contains("limit"));
    }

    #[test]
    fn tools_from_schema_extracts_catalog() {
        let schema = serde_json::json!({
            "protocol": "MCP",
            "tools": [
                {
                    "name": "search",
                    "description": "Search docs",
                    "inputSchema": {
                        "type": "object",
                        "required": ["query"]
                    }
                }
            ]
        });

        let tools = McpAdapter::tools_from_schema(&schema);
        assert_eq!(tools.as_ref().map(Vec::len), Some(1));
        assert_eq!(tools.unwrap()[0].name, "search");
    }
}
