//! MCP stdio client implementation

use super::transport::{
    DefaultStdioProcessExecutor, McpStdioTransport, StdioProcessExecutor, StdioSpawnOptions,
};
use super::types::*;
use super::McpExecutionOptions;
use crate::error::{structured_error_from_anyhow, StructuredError};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleReapPolicy {
    SafeIdleReap,
    Stateful,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleContract {
    pub reap_policy: LifecycleReapPolicy,
}

/// MCP stdio client
pub struct McpStdioClient {
    transport: McpStdioTransport,
    protocol_context: McpProtocolContext,
    server_capabilities: Option<ServerCapabilities>,
    server_info: Option<ServerInfo>,
    instructions: Option<String>,
}

impl McpStdioClient {
    /// Create a new MCP stdio client by spawning a server process
    #[allow(dead_code)]
    pub async fn connect(command: &str, args: &[String]) -> Result<Self> {
        Self::connect_with_options(command, args, StdioSpawnOptions::default()).await
    }

    pub async fn connect_with_options(
        command: &str,
        args: &[String],
        options: StdioSpawnOptions,
    ) -> Result<Self> {
        Self::connect_with_options_and_timeout(
            command,
            args,
            options,
            McpStdioTransport::default_request_timeout(),
        )
        .await
    }

    pub async fn connect_with_options_and_timeout(
        command: &str,
        args: &[String],
        options: StdioSpawnOptions,
        timeout: Duration,
    ) -> Result<Self> {
        Self::connect_with_executor(
            command,
            args,
            options,
            Arc::new(DefaultStdioProcessExecutor),
            timeout,
        )
        .await
    }

    /// Create a new client with a custom executor (for testing)
    pub async fn connect_with_executor(
        command: &str,
        args: &[String],
        options: StdioSpawnOptions,
        executor: Arc<dyn StdioProcessExecutor>,
        timeout: Duration,
    ) -> Result<Self> {
        let client_info = ClientInfo {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: None,
            description: None,
            websiteUrl: None,
            icons: None,
        };

        let mut transport = McpStdioTransport::connect_with_executor(
            command,
            args,
            options.clone(),
            executor.clone(),
        )
        .await?;
        let modern_context = McpProtocolContext {
            era: McpProtocolEra::Modern,
            version: MCP_MODERN_PROTOCOL_VERSION.to_string(),
            client_capabilities: ClientCapabilities::default(),
            server_capabilities: ServerCapabilities::default(),
            client_info: client_info.clone(),
            server_info: None,
        };
        let discover_result = transport
            .send_request_with_timeout(
                "server/discover",
                Some(modern_context.modern_request_params(None)?),
                timeout.min(Duration::from_secs(1)),
            )
            .await;

        match discover_result {
            Ok(result) => match serde_json::from_value::<DiscoverResult>(result) {
                Ok(discover) => {
                    if !discover
                        .supportedVersions
                        .iter()
                        .any(|version| version == MCP_MODERN_PROTOCOL_VERSION)
                    {
                        return Err(StructuredError::new(
                            "UNSUPPORTED_PROTOCOL_VERSION",
                            format!(
                                "MCP server does not support protocol version {}",
                                MCP_MODERN_PROTOCOL_VERSION
                            ),
                            Some(json!({
                                "requested": MCP_MODERN_PROTOCOL_VERSION,
                                "supported": discover.supportedVersions,
                            })),
                        )
                        .into());
                    }

                    let server_info = discover.server_info();
                    tracing::info!(
                        "Connected to modern MCP server: {} v{}",
                        server_info
                            .as_ref()
                            .map(|server| server.name.as_str())
                            .unwrap_or("unknown"),
                        server_info
                            .as_ref()
                            .map(|server| server.version.as_str())
                            .unwrap_or("unknown")
                    );
                    let protocol_context = McpProtocolContext {
                        server_capabilities: discover.capabilities.clone(),
                        server_info: server_info.clone(),
                        ..modern_context
                    };
                    return Ok(Self {
                        transport,
                        protocol_context,
                        server_capabilities: Some(discover.capabilities),
                        server_info,
                        instructions: discover.instructions,
                    });
                }
                Err(err) => tracing::debug!(
                    "MCP server/discover returned a non-modern result; retrying legacy initialize: {}",
                    err
                ),
            },
            Err(err) if is_unsupported_protocol_version(&err) => return Err(err),
            Err(err) => {
                tracing::debug!(
                    "MCP server/discover probe did not identify a modern server; retrying legacy initialize: {}",
                    err
                );
            }
        }
        let _ = transport.kill_and_wait(Duration::from_millis(250)).await;

        // Never reuse a failed probe process: a late response could otherwise
        // be mistaken for the legacy initialize response.
        let mut transport =
            McpStdioTransport::connect_with_executor(command, args, options, executor).await?;
        let init_result = transport
            .initialize_with_timeout(client_info.clone(), timeout)
            .await?;
        tracing::info!(
            "Connected to MCP server: {} v{}",
            init_result
                .serverInfo
                .as_ref()
                .map(|s| s.name.as_str())
                .unwrap_or("unknown"),
            init_result
                .serverInfo
                .as_ref()
                .map(|s| s.version.as_str())
                .unwrap_or("unknown")
        );

        // Send initialized notification
        transport.initialized().await?;

        let protocol_context = McpProtocolContext {
            era: McpProtocolEra::Legacy,
            version: init_result.protocolVersion.clone(),
            client_capabilities: ClientCapabilities::default(),
            server_capabilities: init_result.capabilities.clone(),
            client_info,
            server_info: init_result.serverInfo.clone(),
        };
        Ok(Self {
            transport,
            protocol_context,
            server_capabilities: Some(init_result.capabilities),
            server_info: init_result.serverInfo,
            instructions: init_result.instructions,
        })
    }

    pub fn protocol_era(&self) -> McpProtocolEra {
        self.protocol_context.era
    }

    pub fn protocol_version(&self) -> &str {
        &self.protocol_context.version
    }

    /// Check if the server supports tools
    pub fn supports_tools(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.tools.as_ref())
            .is_some()
    }

    /// Check if the server supports resources
    pub fn supports_resources(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.resources.as_ref())
            .is_some()
    }

    pub fn supports_resource_subscribe(&self) -> bool {
        if self.protocol_context.era == McpProtocolEra::Modern {
            return false;
        }
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.resources.as_ref())
            .and_then(|r| r.subscribe)
            .unwrap_or(false)
    }

    /// Check if the server supports prompts
    pub fn supports_prompts(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.prompts.as_ref())
            .is_some()
    }

    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    pub fn child_id(&self) -> Option<u32> {
        self.transport.child_id()
    }

    pub fn child_has_exited(&mut self) -> Result<bool> {
        self.transport.child_has_exited()
    }

    pub async fn recent_stderr_lines(&self, limit: usize) -> Vec<String> {
        self.transport.recent_stderr_lines(limit).await
    }

    pub async fn kill_and_wait(&mut self, timeout: Duration) -> Result<()> {
        self.transport.kill_and_wait(timeout).await
    }

    pub async fn drain_notifications(&mut self) -> Vec<JsonRpcNotification> {
        self.transport.drain_notifications().await
    }

    pub async fn lifecycle_contract(&mut self, timeout: Duration) -> Result<LifecycleContract> {
        let result = self
            .send_request_with_timeout("uxc/lifecycle_contract", Some(json!({})), timeout)
            .await
            .context("Failed to fetch uxc/lifecycle_contract")?;
        let contract: LifecycleContract =
            serde_json::from_value(result).context("Failed to parse lifecycle contract result")?;
        Ok(contract)
    }

    /// List available tools
    #[allow(dead_code)]
    pub async fn list_tools(&mut self) -> Result<Vec<Tool>> {
        self.list_tools_with_timeout(McpStdioTransport::default_request_timeout())
            .await
    }

    #[allow(dead_code)]
    pub async fn list_tools_catalog(&mut self) -> Result<ToolsCatalog> {
        self.list_tools_catalog_with_timeout(McpStdioTransport::default_request_timeout())
            .await
    }

    pub async fn list_tools_with_timeout(&mut self, timeout: Duration) -> Result<Vec<Tool>> {
        Ok(self.list_tools_catalog_with_timeout(timeout).await?.items)
    }

    pub async fn list_tools_catalog_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<ToolsCatalog> {
        if !self.supports_tools() {
            bail!("Server does not support tools");
        }

        self.list_catalog_with_timeout::<Tool, ToolsListResponse>("tools/list", timeout)
            .await
            .context("Failed to list tools")
    }

    /// Call a tool
    #[allow(dead_code)]
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<JsonValue>,
    ) -> Result<CallToolResult> {
        self.call_tool_with_timeout(
            name,
            arguments,
            McpStdioTransport::default_request_timeout(),
        )
        .await
    }

    pub async fn call_tool_with_timeout(
        &mut self,
        name: &str,
        arguments: Option<JsonValue>,
        timeout: Duration,
    ) -> Result<CallToolResult> {
        self.call_tool_with_options_and_timeout(
            name,
            arguments,
            &McpExecutionOptions::default(),
            timeout,
        )
        .await
    }

    pub async fn call_tool_with_options_and_timeout(
        &mut self,
        name: &str,
        arguments: Option<JsonValue>,
        options: &McpExecutionOptions,
        timeout: Duration,
    ) -> Result<CallToolResult> {
        if !self.supports_tools() {
            bail!("Server does not support tools");
        }

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), JsonValue::String(name.to_string()));
        params.insert(
            "arguments".to_string(),
            arguments.unwrap_or_else(|| json!({})),
        );
        if let Some(continuation) = &options.continuation {
            let continuation = continuation
                .as_object()
                .context("MCP continuation must be a JSON object")?;
            for (key, value) in continuation {
                if matches!(key.as_str(), "name" | "arguments" | "_meta") {
                    bail!("MCP continuation must not override '{}'", key);
                }
                params.insert(key.clone(), value.clone());
            }
        }

        let result = self
            .send_request_with_timeout_and_capabilities(
                "tools/call",
                Some(JsonValue::Object(params)),
                options.capabilities.as_ref(),
                timeout,
            )
            .await
            .context(format!("Failed to call tool '{}'", name))?;

        let call_result: CallToolResult =
            serde_json::from_value(result).context("Failed to parse tool call result")?;

        Ok(call_result)
    }

    /// List available resources
    #[allow(dead_code)]
    pub async fn list_resources(&mut self) -> Result<Vec<Resource>> {
        Ok(self.list_resources_catalog().await?.items)
    }

    pub async fn list_resources_catalog(&mut self) -> Result<ResourcesCatalog> {
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }

        self.list_catalog::<Resource, ResourcesListResponse>("resources/list")
            .await
            .context("Failed to list resources")
    }

    /// Read a resource
    #[allow(dead_code)]
    pub async fn read_resource(&mut self, uri: &str) -> Result<ResourceContents> {
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }

        let params = json!({ "uri": uri });

        let result = self
            .send_request("resources/read", Some(params))
            .await
            .context(format!("Failed to read resource '{}'", uri))?;

        parse_read_resource_result(result).context("Failed to parse resource contents")
    }

    pub async fn subscribe_resource(&mut self, uri: &str) -> Result<()> {
        if self.protocol_context.era == McpProtocolEra::Modern {
            bail!("Modern MCP resource subscriptions require subscriptions/listen");
        }
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }
        if !self.supports_resource_subscribe() {
            bail!("Server does not support resources.subscribe");
        }

        let params = json!({ "uri": uri });
        self.send_request("resources/subscribe", Some(params))
            .await
            .context(format!("Failed to subscribe resource '{}'", uri))?;
        Ok(())
    }

    pub async fn unsubscribe_resource(&mut self, uri: &str) -> Result<()> {
        if self.protocol_context.era == McpProtocolEra::Modern {
            bail!("Modern MCP resource subscriptions require subscriptions/listen");
        }
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }

        let params = json!({ "uri": uri });
        self.send_request("resources/unsubscribe", Some(params))
            .await
            .context(format!("Failed to unsubscribe resource '{}'", uri))?;
        Ok(())
    }

    /// List available prompts
    #[allow(dead_code)]
    pub async fn list_prompts(&mut self) -> Result<Vec<Prompt>> {
        Ok(self.list_prompts_catalog().await?.items)
    }

    pub async fn list_prompts_catalog(&mut self) -> Result<PromptsCatalog> {
        if !self.supports_prompts() {
            bail!("Server does not support prompts");
        }

        self.list_catalog::<Prompt, PromptsListResponse>("prompts/list")
            .await
            .context("Failed to list prompts")
    }

    /// Get a prompt
    #[allow(dead_code)]
    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: Option<HashMap<String, String>>,
    ) -> Result<GetPromptResult> {
        if !self.supports_prompts() {
            bail!("Server does not support prompts");
        }

        let params = json!({
            "name": name,
            "arguments": arguments
        });

        let result = self
            .send_request("prompts/get", Some(params))
            .await
            .context(format!("Failed to get prompt '{}'", name))?;

        let prompt_result: GetPromptResult =
            serde_json::from_value(result).context("Failed to parse prompt result")?;

        Ok(prompt_result)
    }

    async fn send_request(&mut self, method: &str, params: Option<JsonValue>) -> Result<JsonValue> {
        let params = match self.protocol_context.era {
            McpProtocolEra::Modern => Some(self.protocol_context.modern_request_params(params)?),
            McpProtocolEra::Legacy => params,
        };
        self.transport.send_request(method, params).await
    }

    async fn send_request_with_timeout(
        &mut self,
        method: &str,
        params: Option<JsonValue>,
        timeout: Duration,
    ) -> Result<JsonValue> {
        self.send_request_with_timeout_and_capabilities(method, params, None, timeout)
            .await
    }

    async fn send_request_with_timeout_and_capabilities(
        &mut self,
        method: &str,
        params: Option<JsonValue>,
        capabilities: Option<&JsonValue>,
        timeout: Duration,
    ) -> Result<JsonValue> {
        let params = match self.protocol_context.era {
            McpProtocolEra::Modern => Some(
                self.protocol_context
                    .modern_request_params_with_capabilities(params, capabilities)?,
            ),
            McpProtocolEra::Legacy => params,
        };
        self.transport
            .send_request_with_timeout(method, params, timeout)
            .await
    }

    async fn list_catalog<T, R>(&mut self, method: &str) -> Result<McpListCatalog<T>>
    where
        R: serde::de::DeserializeOwned + McpListPage<Item = T>,
    {
        let mut paginator = McpCatalogPaginator::new(method);
        loop {
            let params = paginator.next_request_params()?;
            let result = self
                .send_request(method, params)
                .await
                .with_context(|| format!("Failed to fetch {}", method))?;
            if !paginator.absorb_response::<R>(result)? {
                return Ok(paginator.finish());
            }
        }
    }

    async fn list_catalog_with_timeout<T, R>(
        &mut self,
        method: &str,
        timeout: Duration,
    ) -> Result<McpListCatalog<T>>
    where
        R: serde::de::DeserializeOwned + McpListPage<Item = T>,
    {
        let mut paginator = McpCatalogPaginator::new(method);
        loop {
            let params = paginator.next_request_params()?;
            let result = self
                .send_request_with_timeout(method, params, timeout)
                .await
                .with_context(|| format!("Failed to fetch {}", method))?;
            if !paginator.absorb_response::<R>(result)? {
                return Ok(paginator.finish());
            }
        }
    }
}

fn is_unsupported_protocol_version(err: &anyhow::Error) -> bool {
    structured_error_from_anyhow(err)
        .and_then(|error| error.details)
        .and_then(|details| details.get("jsonrpc_code").and_then(JsonValue::as_i64))
        == Some(-32022)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::structured_error_from_anyhow;

    #[tokio::test]
    async fn modern_stdio_uses_discover_and_request_metadata_without_initialize() {
        let script = r#"
            read discover
            echo "$discover" | grep -q '"method":"server/discover"' || exit 10
            echo "$discover" | grep -q '"io.modelcontextprotocol/protocolVersion":"2026-07-28"' || exit 11
            echo "$discover" | grep -q '"io.modelcontextprotocol/clientCapabilities":{}' || exit 12
            echo '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private","_meta":{"io.modelcontextprotocol/serverInfo":{"name":"modern-test","version":"1.0.0"}}}}'
            read list
            echo "$list" | grep -q '"method":"tools/list"' || exit 13
            echo "$list" | grep -q '"io.modelcontextprotocol/protocolVersion":"2026-07-28"' || exit 14
            echo "$list" | grep -q '"method":"initialize"' && exit 15
            echo '{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","tools":[{"name":"ping","inputSchema":{"type":"object"}}],"ttlMs":0,"cacheScope":"private"}}'
            read call
            echo "$call" | grep -q '"method":"tools/call"' || exit 16
            echo "$call" | grep -q '"name":"ping"' || exit 17
            echo "$call" | grep -q '"io.modelcontextprotocol/protocolVersion":"2026-07-28"' || exit 18
            echo '{"jsonrpc":"2.0","id":3,"result":{"resultType":"complete","content":[{"type":"audio","data":"opaque","mimeType":"audio/wav","futureField":true}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        assert_eq!(client.protocol_era(), McpProtocolEra::Modern);
        assert_eq!(client.protocol_version(), MCP_MODERN_PROTOCOL_VERSION);
        assert_eq!(
            client.server_info().map(|server| server.name.as_str()),
            Some("modern-test")
        );
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "ping");
        assert_eq!(tools[0].description, None);
        let result = client.call_tool("ping", None).await.unwrap();
        assert_eq!(result.resultType, "complete");
        assert_eq!(result.content[0].content_type(), Some("audio"));
        assert_eq!(result.content[0].as_value()["futureField"], true);
    }

    #[tokio::test]
    async fn legacy_fallback_restarts_process_before_initialize() {
        let dir = tempfile::tempdir().unwrap();
        let count_path = dir.path().join("spawn-count");
        let script = format!(
            r#"
            count=0
            if [ -f "{path}" ]; then count=$(cat "{path}"); fi
            count=$((count + 1))
            echo "$count" > "{path}"
            read request
            if [ "$count" -eq 1 ]; then
                echo "$request" | grep -q '"method":"server/discover"' || exit 20
                echo '{{"jsonrpc":"2.0","id":1,"error":{{"code":-32601,"message":"Method not found"}}}}'
                while read ignored; do :; done
            else
                echo "$request" | grep -q '"method":"initialize"' || exit 21
                echo '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"legacy-test","version":"1.0.0"}}}}}}'
                read initialized
                echo "$initialized" | grep -q '"method":"notifications/initialized"' || exit 22
                while read ignored; do :; done
            fi
            "#,
            path = count_path.display()
        );

        let client = McpStdioClient::connect("sh", &["-c".to_string(), script])
            .await
            .unwrap();

        assert_eq!(client.protocol_era(), McpProtocolEra::Legacy);
        assert_eq!(client.protocol_version(), MCP_PROTOCOL_VERSION);
        assert_eq!(std::fs::read_to_string(count_path).unwrap().trim(), "2");
    }

    #[tokio::test]
    async fn client_requires_tools_capability() {
        // Test with a script that doesn't support tools
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}'
            while read line; do
                echo '{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"Method not found"}}'
            done
        "#;

        let client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        assert!(!client.supports_tools());
    }

    #[tokio::test]
    async fn client_detects_tools_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            while read line; do sleep 1; done
        "#;

        let client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        assert!(client.supports_tools());
    }

    #[tokio::test]
    async fn client_detects_resources_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"resources":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            while read line; do sleep 1; done
        "#;

        let client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        assert!(client.supports_resources());
    }

    #[tokio::test]
    async fn client_detects_prompts_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"prompts":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            while read line; do sleep 1; done
        "#;

        let client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        assert!(client.supports_prompts());
    }

    #[tokio::test]
    async fn list_tools_fails_without_tools_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}'
            echo '{"jsonrpc":"2.0","id":2,"result":{}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client.list_tools().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support tools"));
    }

    #[tokio::test]
    async fn list_tools_returns_tool_list() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"test_tool","description":"A test tool","inputSchema":{"type":"object"}}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "test_tool");
        assert_eq!(tools[0].description.as_deref(), Some("A test tool"));
    }

    #[tokio::test]
    async fn list_tools_catalog_paginates_and_aggregates_metadata() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read initialized
            echo "$initialized" | grep -q '"method":"notifications/initialized"' || exit 29
            read page1
            echo "$page1" | grep -q '"method":"tools/list"' || exit 30
            echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"tool-1"},{"name":"tool-2"}],"nextCursor":"cursor-2","ttlMs":30,"cacheScope":"public"}}'
            read page2
            echo "$page2" | grep -q '"cursor":"cursor-2"' || exit 31
            echo '{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"tool-3"}],"ttlMs":10,"cacheScope":"private"}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let catalog = client.list_tools_catalog().await.unwrap();
        assert_eq!(catalog.items.len(), 3);
        assert_eq!(catalog.items[0].name, "tool-1");
        assert_eq!(catalog.items[2].name, "tool-3");
        assert_eq!(catalog.metadata.ttlMs, Some(10));
        assert_eq!(catalog.metadata.cacheScope.as_deref(), Some("private"));
        assert_eq!(catalog.metadata.pageCount, 2);
        assert_eq!(catalog.metadata.itemCount, 3);
    }

    #[tokio::test]
    async fn list_tools_catalog_enforces_page_limit() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read initialized
            echo "$initialized" | grep -q '"method":"notifications/initialized"' || exit 59
            count=0
            while read request; do
                count=$((count + 1))
                response_id=$((count + 1))
                if [ "$count" -lt 100 ]; then
                    next=",\"nextCursor\":\"cursor-$((count + 1))\""
                else
                    next=",\"nextCursor\":\"cursor-overflow\""
                fi
                echo "{\"jsonrpc\":\"2.0\",\"id\":$response_id,\"result\":{\"tools\":[{\"name\":\"tool-$count\"}]$next}}"
            done
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let err = client.list_tools_catalog().await.unwrap_err();
        assert!(format!("{err:#}").contains("maximum of 100 pages"));
    }

    #[tokio::test]
    async fn call_tool_executes_tool_with_arguments() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"Tool executed successfully"}],"structuredContent":{"message":"Tool executed successfully"}}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let args = serde_json::json!({"param1": "value1"});
        let result = client.call_tool("test_tool", Some(args)).await.unwrap();

        assert_eq!(result.resultType, "complete");
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].content_type(), Some("text"));
        assert_eq!(
            result.content[0].as_value()["text"],
            "Tool executed successfully"
        );
        assert_eq!(
            result.structuredContent,
            Some(serde_json::json!({ "message": "Tool executed successfully" }))
        );
    }

    #[tokio::test]
    async fn call_tool_fails_without_tools_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client.call_tool("test", None).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support tools"));
    }

    #[tokio::test]
    async fn list_resources_fails_without_resources_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client.list_resources().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support resources"));
    }

    #[tokio::test]
    async fn list_resources_returns_resource_list() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"resources":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"resources":[{"name":"test_resource","uri":"test://resource","description":"A test resource"}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let resources = client.list_resources().await.unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "test_resource");
        assert_eq!(resources[0].uri, "test://resource");
    }

    #[tokio::test]
    async fn list_resources_paginates() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"resources":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read initialized
            echo "$initialized" | grep -q '"method":"notifications/initialized"' || exit 39
            read page1
            echo "$page1" | grep -q '"method":"resources/list"' || exit 40
            echo '{"jsonrpc":"2.0","id":2,"result":{"resources":[{"name":"r1","uri":"test://r1","description":"R1"}],"nextCursor":"cursor-2"}}'
            read page2
            echo "$page2" | grep -q '"cursor":"cursor-2"' || exit 41
            echo '{"jsonrpc":"2.0","id":3,"result":{"resources":[{"name":"r2","uri":"test://r2","description":"R2"}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let resources = client.list_resources().await.unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[1].uri, "test://r2");
    }

    #[tokio::test]
    async fn read_resource_returns_resource_contents() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"resources":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"contents":[{"uri":"test://resource"}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client.read_resource("test://resource").await.unwrap();

        assert_eq!(result.uri, "test://resource");
    }

    #[tokio::test]
    async fn list_prompts_fails_without_prompts_capability() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client.list_prompts().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support prompts"));
    }

    #[tokio::test]
    async fn list_prompts_returns_prompt_list() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"prompts":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"prompts":[{"name":"test_prompt","description":"A test prompt"}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let prompts = client.list_prompts().await.unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "test_prompt");
    }

    #[tokio::test]
    async fn list_prompts_detects_cursor_cycle() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"prompts":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read initialized
            echo "$initialized" | grep -q '"method":"notifications/initialized"' || exit 49
            read page1
            echo "$page1" | grep -q '"method":"prompts/list"' || exit 50
            echo '{"jsonrpc":"2.0","id":2,"result":{"prompts":[{"name":"p1","description":"P1"}],"nextCursor":"loop"}}'
            read page2
            echo "$page2" | grep -q '"cursor":"loop"' || exit 51
            echo '{"jsonrpc":"2.0","id":3,"result":{"prompts":[{"name":"p2","description":"P2"}],"nextCursor":"loop"}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let err = client.list_prompts().await.unwrap_err();
        assert!(format!("{err:#}").contains("cursor cycle"));
    }

    #[tokio::test]
    async fn get_prompt_returns_prompt_messages() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"prompts":{}},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"description":"Test prompt","messages":[{"role":"user","content":"Test content"}]}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client.get_prompt("test_prompt", None).await.unwrap();
        assert_eq!(result.description, "Test prompt");
        assert_eq!(result.messages.len(), 1);
    }

    #[tokio::test]
    async fn initialize_sequence_completes_successfully() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{},"resources":{},"prompts":{}},"serverInfo":{"name":"test-server","version":"1.0.0"}}}'
            # Read initialized notification
            read line
            while read line; do sleep 1; done
        "#;

        let client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        assert!(client.supports_tools());
        assert!(client.supports_resources());
        assert!(client.supports_prompts());
    }

    #[tokio::test]
    async fn tool_content_variants_deserialize_correctly() {
        // Test that different tool content types can be deserialized
        let text_json = r#"{"type":"text","text":"Hello"}"#;
        let text: ToolContent = serde_json::from_str(text_json).unwrap();
        assert_eq!(text.content_type(), Some("text"));
        assert_eq!(text.as_value()["text"], "Hello");

        let image_json = r#"{"type":"image","data":"base64data","mimeType":"image/png"}"#;
        let image: ToolContent = serde_json::from_str(image_json).unwrap();
        assert_eq!(image.content_type(), Some("image"));
        assert_eq!(image.as_value()["data"], "base64data");
        assert_eq!(image.as_value()["mimeType"], "image/png");

        let resource_json = r#"{"type":"resource","uri":"test://resource","text":"content"}"#;
        let resource: ToolContent = serde_json::from_str(resource_json).unwrap();
        assert_eq!(resource.content_type(), Some("resource"));
        assert_eq!(resource.as_value()["uri"], "test://resource");
        assert_eq!(resource.as_value()["text"], "content");
    }

    #[tokio::test]
    async fn server_capabilities_optional_fields() {
        // Test that optional fields in ServerCapabilities work correctly
        let caps_json = r#"{"tools":{},"resources":{"subscribe":true}}"#;
        let caps: ServerCapabilities = serde_json::from_str(caps_json).unwrap();
        assert!(caps.tools.is_some());
        assert!(caps.resources.is_some());
        assert!(caps.prompts.is_none());
    }

    #[tokio::test]
    async fn initialize_result_optional_fields() {
        // Test that optional fields in InitializeResult work correctly
        let init_json = r#"{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"},"instructions":"Use this server"}"#;
        let init: InitializeResult = serde_json::from_str(init_json).unwrap();
        assert_eq!(init.protocolVersion, "2024-11-05");
        assert!(init.serverInfo.is_some());
        assert!(init.instructions.is_some());
        assert_eq!(init.instructions.unwrap(), "Use this server");
    }

    #[tokio::test]
    async fn lifecycle_contract_parses_successful_response() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"reap_policy":"stateful"}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client
            .lifecycle_contract(Duration::from_millis(50))
            .await
            .unwrap();
        assert_eq!(result.reap_policy, LifecycleReapPolicy::Stateful);
    }

    #[tokio::test]
    async fn lifecycle_contract_surfaces_method_not_found_code() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"Method not found"}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let err = client
            .lifecycle_contract(Duration::from_millis(50))
            .await
            .unwrap_err();
        let structured = structured_error_from_anyhow(&err).expect("structured error");
        let jsonrpc_code = structured
            .details
            .as_ref()
            .and_then(|details| details.get("jsonrpc_code"))
            .and_then(|value| value.as_i64());
        assert_eq!(jsonrpc_code, Some(-32601));
    }
}
