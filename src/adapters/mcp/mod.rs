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
pub use client::{LifecycleReapPolicy, McpStdioClient};
pub use http_transport::{McpHttpTransport, McpRemoteTransport, ResolvedMcpHttpTransport};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};
pub use transport::StdioSpawnOptions;

pub const MCP_CAPABILITIES_ARG: &str = "__uxc_mcp_capabilities";
pub const MCP_CONTINUATION_ARG: &str = "__uxc_mcp_continuation";

#[derive(Debug, Clone, Default)]
pub struct McpExecutionOptions {
    pub capabilities: Option<Value>,
    pub continuation: Option<Value>,
}

pub struct McpAdapter {
    cache: Option<Arc<dyn crate::cache::Cache>>,
    auth_profile: Option<Profile>,
    force_refresh_schema: bool,
    discovered_http_endpoints: Arc<RwLock<HashMap<String, ResolvedMcpHttpTransport>>>,
    stdio_spawn_options: transport::StdioSpawnOptions,
    last_probe_diagnostics: Arc<RwLock<Option<String>>>,
    request_timeout: Option<Duration>,
}

impl McpAdapter {
    pub fn split_execution_options(
        mut args: HashMap<String, Value>,
    ) -> Result<(HashMap<String, Value>, McpExecutionOptions)> {
        let capabilities = args.remove(MCP_CAPABILITIES_ARG);
        if capabilities
            .as_ref()
            .is_some_and(|value| !value.is_object())
        {
            bail!("--mcp-capabilities must resolve to a JSON object");
        }
        let continuation = args.remove(MCP_CONTINUATION_ARG);
        if continuation
            .as_ref()
            .is_some_and(|value| !value.is_object())
        {
            bail!("--mcp-continuation must resolve to a JSON object");
        }
        Ok((
            args,
            McpExecutionOptions {
                capabilities,
                continuation,
            },
        ))
    }

    fn build_tool_arguments(args: HashMap<String, Value>) -> Option<Value> {
        Some(Value::Object(args.into_iter().collect()))
    }

    pub fn new() -> Self {
        Self {
            cache: None,
            auth_profile: None,
            force_refresh_schema: false,
            stdio_spawn_options: transport::StdioSpawnOptions::default(),
            discovered_http_endpoints: Arc::new(RwLock::new(HashMap::new())),
            last_probe_diagnostics: Arc::new(RwLock::new(None)),
            request_timeout: None,
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

    pub fn with_stdio_spawn_options(mut self, options: transport::StdioSpawnOptions) -> Self {
        self.stdio_spawn_options = options;
        self
    }

    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.request_timeout = timeout;
        self
    }

    fn request_timeout_or_default(&self) -> Duration {
        self.request_timeout
            .unwrap_or_else(transport::McpStdioTransport::default_request_timeout)
    }

    pub(crate) fn schema_cache_key_for(url: &str, auth_profile: Option<&Profile>) -> String {
        let mut hasher = Sha256::new();
        if let Some(profile) = auth_profile {
            match serde_json::to_value(profile) {
                Ok(value) => hash_canonical_json(&mut hasher, &value),
                Err(err) => {
                    debug!(error = %err, "Failed to serialize auth profile for MCP cache key");
                    hasher.update(b"serialization-error");
                }
            }
        } else {
            hasher.update(b"anonymous");
        }
        format!(
            "{}#uxc-mcp-schema=dual-era-2026&auth={:x}",
            url,
            hasher.finalize()
        )
    }

    fn schema_cache_key(&self, url: &str) -> String {
        Self::schema_cache_key_for(url, self.auth_profile.as_ref())
    }

    fn catalog_ttl_seconds(metadata: &types::McpListCatalogMetadata) -> Option<u64> {
        metadata
            .ttlMs
            .map(|ttl_ms| ttl_ms.saturating_add(999) / 1000)
    }

    fn cache_schema(
        &self,
        url: &str,
        schema: &Value,
        metadata: Option<&types::McpListCatalogMetadata>,
    ) {
        let Some(cache) = &self.cache else {
            return;
        };
        let cache_key = self.schema_cache_key(url);
        let result = match metadata.and_then(Self::catalog_ttl_seconds) {
            Some(ttl_seconds) => cache.put_with_ttl(&cache_key, schema, ttl_seconds),
            None => cache.put(&cache_key, schema),
        };
        if let Err(err) = result {
            debug!("Failed to cache MCP schema: {}", err);
        } else {
            info!("Cached MCP schema for: {}", url);
        }
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

    async fn resolve_http_transport(&self, url: &str) -> Result<Option<ResolvedMcpHttpTransport>> {
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
        if !self.force_refresh_schema {
            if let Some(cache) = &self.cache {
                if let crate::cache::CacheResult::Hit(schema) =
                    cache.get(&self.schema_cache_key(url))?
                {
                    if let Some(resolved) = Self::resolved_transport_from_schema(&schema) {
                        let mut discovered = self.discovered_http_endpoints.write().await;
                        discovered.insert(normalized, resolved.clone());
                        return Ok(Some(resolved));
                    }
                }
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
                Ok(http_transport::ProbeInitializeOutcome::Success(mode)) => {
                    let resolved = ResolvedMcpHttpTransport::new(mode, candidate.clone());
                    let mut cache = self.discovered_http_endpoints.write().await;
                    cache.insert(normalized, resolved.clone());
                    return Ok(Some(resolved));
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

    fn resolved_transport_from_schema(schema: &Value) -> Option<ResolvedMcpHttpTransport> {
        let resolved = schema.get("resolvedTransport")?;
        let mode = match resolved.get("mode")?.as_str()? {
            "modern_streamable_http" => http_transport::McpHttpMode::ModernStreamableHttp,
            "streamable_http" | "legacy_streamable_http" => {
                http_transport::McpHttpMode::LegacyStreamableHttp
            }
            "legacy_sse" => http_transport::McpHttpMode::LegacySse,
            _ => return None,
        };
        let connect_url = resolved.get("connect_url")?.as_str()?.to_string();
        Some(ResolvedMcpHttpTransport::new(mode, connect_url))
    }

    fn resolved_transport_json(resolved: &ResolvedMcpHttpTransport) -> Value {
        let mode = match resolved.mode {
            http_transport::McpHttpMode::ModernStreamableHttp => "modern_streamable_http",
            http_transport::McpHttpMode::LegacyStreamableHttp => "legacy_streamable_http",
            http_transport::McpHttpMode::LegacySse => "legacy_sse",
        };
        serde_json::json!({
            "mode": mode,
            "connect_url": resolved.connect_url,
        })
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
                match cache.get(&self.schema_cache_key(url))? {
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
            let mut client = McpStdioClient::connect_with_options_and_timeout(
                &cmd,
                &args,
                self.stdio_spawn_options.clone(),
                self.request_timeout_or_default(),
            )
            .await?;
            let protocol_version = client.protocol_version().to_string();
            let protocol_era = client.protocol_era();
            let server_info = client.server_info().cloned();
            let instructions = client.instructions().map(ToString::to_string);
            let tools = match client
                .list_tools_catalog_with_timeout(self.request_timeout_or_default())
                .await
            {
                Ok(catalog) => Some(catalog),
                Err(err) => {
                    debug!("MCP stdio list_tools failed while building schema: {}", err);
                    None
                }
            };

            // Build schema from server capabilities
            let mut schema = serde_json::json!({
                "protocol": "MCP",
                "protocolVersion": protocol_version,
                "protocolEra": protocol_era,
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
            if let Some(catalog) = &tools {
                schema["tools"] = serde_json::json!(catalog.items);
                schema["cacheMetadata"] = serde_json::to_value(&catalog.metadata)?;
            }

            self.cache_schema(
                url,
                &schema,
                tools.as_ref().map(|catalog| &catalog.metadata),
            );

            return Ok(schema);
        }

        // For HTTP-based MCP, connect and get server info
        if Self::is_http_url(url) {
            let resolved = self.resolve_http_transport(url).await?.ok_or_else(|| {
                anyhow::anyhow!("Unable to discover MCP HTTP endpoint for {}", url)
            })?;
            let transport = McpRemoteTransport::with_auth_and_timeout(
                resolved.clone(),
                self.auth_profile.clone(),
                self.request_timeout_or_default(),
            )?;
            let init_result = transport.initialize().await?;
            // MCP lifecycle (2025-03-26) requires the client to ack with
            // `notifications/initialized` before the server will service
            // subsequent requests on the session. Spec-compliant servers
            // (e.g., rmcp 0.15) otherwise hang on follow-up requests.
            transport.initialized().await?;
            let tools = match transport.list_tools_catalog().await {
                Ok(catalog) => Some(catalog),
                Err(err) => {
                    debug!("MCP HTTP list_tools failed while building schema: {}", err);
                    None
                }
            };

            let mut schema = serde_json::json!({
                "protocol": "MCP",
                "protocolVersion": init_result.protocolVersion,
                "protocolEra": match resolved.mode {
                    http_transport::McpHttpMode::ModernStreamableHttp => "modern",
                    _ => "legacy",
                },
                "transport": "http",
                "url": url,
                "resolvedTransport": Self::resolved_transport_json(&resolved),
                "serverInfo": init_result.serverInfo,
                "instructions": init_result.instructions,
                "capabilities": init_result.capabilities
            });
            if let Some(catalog) = &tools {
                schema["tools"] = serde_json::json!(catalog.items);
                schema["cacheMetadata"] = serde_json::to_value(&catalog.metadata)?;
            }

            self.cache_schema(
                url,
                &schema,
                tools.as_ref().map(|catalog| &catalog.metadata),
            );

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
            return Ok(self.resolve_http_transport(url).await?.is_some());
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
                let parameters = if let Some(schema) = tool.inputSchema.as_ref() {
                    parse_schema_to_parameters_for_daemon(schema)
                } else {
                    Vec::new()
                };

                Operation {
                    operation_id: tool.name.clone(),
                    display_name: tool.display_name().to_string(),
                    description: tool.description,
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
                    display_name: tool.display_name().to_string(),
                    description: tool.description,
                    parameters: tool
                        .inputSchema
                        .as_ref()
                        .map(parse_schema_to_parameters_for_daemon)
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
        let (args, execution_options) = Self::split_execution_options(args)?;
        self.validate_tool_call(url, operation, &args).await?;

        if Self::is_stdio_command(url) {
            let (cmd, args_list) = Self::parse_stdio_command(url)?;
            let mut client = McpStdioClient::connect_with_options_and_timeout(
                &cmd,
                &args_list,
                self.stdio_spawn_options.clone(),
                self.request_timeout_or_default(),
            )
            .await?;

            let arguments = Self::build_tool_arguments(args);

            let result = client
                .call_tool_with_options_and_timeout(
                    operation,
                    arguments,
                    &execution_options,
                    self.request_timeout_or_default(),
                )
                .await?;

            let output = convert_tool_result_to_value(&result);

            return Ok(ExecutionResult {
                data: output,
                metadata: super::ExecutionMetadata {
                    duration_ms: start.elapsed().as_millis() as u64,
                    operation: operation.to_string(),
                    response_status_code: None,
                    response_headers: std::collections::HashMap::new(),
                },
            });
        }

        // For HTTP-based MCP
        if Self::is_http_url(url) {
            let resolved = self.resolve_http_transport(url).await?.ok_or_else(|| {
                anyhow::anyhow!("Unable to discover MCP HTTP endpoint for {}", url)
            })?;
            let transport = McpRemoteTransport::with_auth_and_timeout(
                resolved,
                self.auth_profile.clone(),
                self.request_timeout_or_default(),
            )?;
            transport.initialize().await?;
            // See notification rationale above in `fetch_schema_for_url`.
            transport.initialized().await?;

            let arguments = Self::build_tool_arguments(args);

            let result = transport
                .call_tool_with_options(operation, arguments, &execution_options)
                .await?;

            let output = convert_tool_result_to_value(&result);

            return Ok(ExecutionResult {
                data: output,
                metadata: super::ExecutionMetadata {
                    duration_ms: start.elapsed().as_millis() as u64,
                    operation: operation.to_string(),
                    response_status_code: None,
                    response_headers: std::collections::HashMap::new(),
                },
            });
        }

        bail!("Unsupported MCP URL format: {}", url)
    }
}

/// Parse JSON Schema to our Parameter format
pub(crate) fn parse_schema_to_parameters_for_daemon(schema: &Value) -> Vec<super::Parameter> {
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

/// Convert MCP tool call result to a JSON value for output.
pub(crate) fn convert_tool_result_to_value(result: &types::ToolCallResult) -> Value {
    let mut output = serde_json::json!({
        "resultType": result.resultType,
        "content": convert_tool_content_to_json(&result.content)
    });

    if let Some(is_error) = result.isError {
        output["isError"] = serde_json::json!(is_error);
    }
    if let Some(structured) = &result.structuredContent {
        output["structuredContent"] = structured.clone();
    }
    if let Some(input_requests) = &result.inputRequests {
        output["inputRequests"] = input_requests.clone();
    }
    if let Some(request_state) = &result.requestState {
        output["requestState"] = serde_json::json!(request_state);
    }
    if let Some(meta) = &result.meta {
        output["_meta"] = meta.clone();
    }

    output
}

fn convert_tool_content_to_json(content: &[types::ToolContent]) -> Value {
    Value::Array(content.iter().map(|item| item.as_value().clone()).collect())
}

fn hash_canonical_json(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update(b"null"),
        Value::Bool(value) => hasher.update(if *value {
            b"true".as_slice()
        } else {
            b"false".as_slice()
        }),
        Value::Number(value) => hasher.update(value.to_string().as_bytes()),
        Value::String(value) => {
            hasher.update(b"\"");
            hasher.update(value.as_bytes());
            hasher.update(b"\"");
        }
        Value::Array(values) => {
            hasher.update(b"[");
            for value in values {
                hash_canonical_json(hasher, value);
                hasher.update(b",");
            }
            hasher.update(b"]");
        }
        Value::Object(values) => {
            hasher.update(b"{");
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                hasher.update(key.as_bytes());
                hasher.update(b":");
                hash_canonical_json(hasher, &values[key]);
                hasher.update(b",");
            }
            hasher.update(b"}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{Cache, CacheLookup, CacheReadPolicy, CacheResult, CacheStats};
    use serde_json::json;
    use std::collections::HashMap as StdHashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TestCache {
        entries: Mutex<StdHashMap<String, Value>>,
    }

    impl Cache for TestCache {
        fn get(&self, url: &str) -> Result<CacheResult> {
            Ok(match self.entries.lock().unwrap().get(url) {
                Some(v) => CacheResult::Hit(v.clone()),
                None => CacheResult::Miss,
            })
        }

        fn get_with_policy(&self, url: &str, _policy: CacheReadPolicy) -> Result<CacheLookup> {
            Ok(match self.entries.lock().unwrap().get(url) {
                Some(v) => CacheLookup::Hit(crate::cache::CacheHit {
                    schema: v.clone(),
                    fetched_at: 0,
                    stale: false,
                }),
                None => CacheLookup::Miss,
            })
        }

        fn put(&self, url: &str, schema: &Value) -> Result<()> {
            self.entries
                .lock()
                .unwrap()
                .insert(url.to_string(), schema.clone());
            Ok(())
        }

        fn invalidate(&self, _url: &str) -> Result<()> {
            Ok(())
        }

        fn invalidate_by_key(&self, _key: &str) -> Result<()> {
            Ok(())
        }

        fn clear(&self) -> Result<()> {
            self.entries.lock().unwrap().clear();
            Ok(())
        }

        fn list_entries(&self) -> Result<Vec<crate::cache::CacheListEntry>> {
            Ok(Vec::new())
        }

        fn stats(&self) -> Result<CacheStats> {
            Ok(CacheStats::default())
        }

        fn is_enabled(&self) -> bool {
            true
        }
    }

    #[test]
    fn convert_tool_result_includes_structured_content_and_error_flag() {
        let result = types::ToolCallResult {
            resultType: "complete".to_string(),
            content: vec![types::ToolContent(json!({
                "type": "text",
                "text": "hello"
            }))],
            isError: Some(true),
            structuredContent: Some(json!({ "message": "hello", "count": 1 })),
            inputRequests: None,
            requestState: None,
            meta: None,
        };

        let output = convert_tool_result_to_value(&result);
        assert_eq!(output["content"][0]["type"], "text");
        assert_eq!(output["content"][0]["text"], "hello");
        assert_eq!(output["resultType"], "complete");
        assert_eq!(output["isError"], true);
        assert_eq!(output["structuredContent"]["message"], "hello");
        assert_eq!(output["structuredContent"]["count"], 1);
    }

    #[test]
    fn split_execution_options_keeps_control_fields_out_of_tool_arguments() {
        let (args, options) = McpAdapter::split_execution_options(StdHashMap::from([
            ("message".to_string(), json!("hello")),
            (MCP_CAPABILITIES_ARG.to_string(), json!({"elicitation": {}})),
            (
                MCP_CONTINUATION_ARG.to_string(),
                json!({
                    "inputResponses": {"request-1": {"action": "accept"}},
                    "requestState": "opaque"
                }),
            ),
        ]))
        .unwrap();

        assert_eq!(
            args,
            StdHashMap::from([("message".to_string(), json!("hello"))])
        );
        assert_eq!(options.capabilities, Some(json!({"elicitation": {}})));
        assert_eq!(
            options.continuation,
            Some(json!({
                "inputResponses": {"request-1": {"action": "accept"}},
                "requestState": "opaque"
            }))
        );
    }

    #[test]
    fn schema_cache_key_is_stable_across_auth_field_insertion_order() {
        use crate::auth::{AuthType, SecretSource};

        let mut first = Profile::new("secret".to_string(), AuthType::Bearer);
        first.fields.insert(
            "alpha".to_string(),
            SecretSource::Literal {
                value: "one".to_string(),
            },
        );
        first.fields.insert(
            "beta".to_string(),
            SecretSource::Literal {
                value: "two".to_string(),
            },
        );

        let mut second = Profile::new("secret".to_string(), AuthType::Bearer);
        second.fields.insert(
            "beta".to_string(),
            SecretSource::Literal {
                value: "two".to_string(),
            },
        );
        second.fields.insert(
            "alpha".to_string(),
            SecretSource::Literal {
                value: "one".to_string(),
            },
        );

        assert_eq!(
            McpAdapter::schema_cache_key_for("https://example.com/mcp", Some(&first)),
            McpAdapter::schema_cache_key_for("https://example.com/mcp", Some(&second))
        );
    }

    #[test]
    fn convert_tool_result_preserves_local_artifact_paths() {
        let result = types::ToolCallResult {
            resultType: "complete".to_string(),
            content: vec![],
            isError: Some(false),
            structuredContent: Some(json!({
                "ok": true,
                "artifacts": [
                    {
                        "kind": "file",
                        "name": "report.csv",
                        "path": "/tmp/webmcp-artifacts/report.csv"
                    }
                ]
            })),
            inputRequests: None,
            requestState: None,
            meta: None,
        };

        let output = convert_tool_result_to_value(&result);
        assert_eq!(
            output["structuredContent"]["artifacts"][0]["path"],
            "/tmp/webmcp-artifacts/report.csv"
        );
    }

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
            .resolve_http_transport(&server.url())
            .await
            .unwrap()
            .unwrap();
        assert!(resolved.connect_url.ends_with("/mcp"));
    }

    #[tokio::test]
    async fn resolve_http_transport_reuses_cached_resolved_transport() {
        let cache = Arc::new(TestCache::default());
        let url = "https://example.com";
        let adapter = McpAdapter::new().with_cache(cache.clone());
        cache
            .put(
                &adapter.schema_cache_key(url),
                &json!({
                    "protocol": "MCP",
                    "transport": "http",
                    "resolvedTransport": {
                        "mode": "legacy_sse",
                        "connect_url": "https://example.com/mcp"
                    }
                }),
            )
            .unwrap();

        let resolved = adapter.resolve_http_transport(url).await.unwrap().unwrap();

        assert_eq!(resolved.mode, http_transport::McpHttpMode::LegacySse);
        assert_eq!(resolved.connect_url, "https://example.com/mcp");
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

    #[test]
    fn build_tool_arguments_preserves_empty_object() {
        let arguments = McpAdapter::build_tool_arguments(HashMap::new());
        assert_eq!(arguments, Some(json!({})));
    }
}
