//! MCP stdio client implementation

use super::transport::{
    DefaultStdioProcessExecutor, McpStdioTransport, StdioProcessExecutor, StdioSpawnOptions,
};
use super::types::*;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CanReapState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owns_external_resource: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_for_human: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanReapProbeResult {
    pub can_reap: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<CanReapState>,
}

/// MCP stdio client
pub struct McpStdioClient {
    transport: McpStdioTransport,
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
        Self::connect_with_executor(
            command,
            args,
            options,
            Arc::new(DefaultStdioProcessExecutor),
        )
        .await
    }

    /// Create a new client with a custom executor (for testing)
    pub async fn connect_with_executor(
        command: &str,
        args: &[String],
        options: StdioSpawnOptions,
        executor: Arc<dyn StdioProcessExecutor>,
    ) -> Result<Self> {
        let mut transport =
            McpStdioTransport::connect_with_executor(command, args, options, executor).await?;

        // Initialize the session
        let client_info = ClientInfo {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let init_result = transport.initialize(client_info).await?;
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

        Ok(Self {
            transport,
            server_capabilities: Some(init_result.capabilities),
            server_info: init_result.serverInfo,
            instructions: init_result.instructions,
        })
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

    pub async fn probe_can_reap(
        &mut self,
        idle_for_secs: u64,
        idle_ttl_secs: u64,
        timeout: Duration,
    ) -> Result<CanReapProbeResult> {
        let params = json!({
            "idle_for_secs": idle_for_secs,
            "idle_ttl_secs": idle_ttl_secs,
        });
        let result = self
            .transport
            .send_request_with_timeout("uxc/can_reap", Some(params), timeout)
            .await
            .context("Failed to probe uxc/can_reap")?;
        let probe: CanReapProbeResult =
            serde_json::from_value(result).context("Failed to parse can_reap probe result")?;
        Ok(probe)
    }

    /// List available tools
    pub async fn list_tools(&mut self) -> Result<Vec<Tool>> {
        if !self.supports_tools() {
            bail!("Server does not support tools");
        }

        let result = self
            .transport
            .send_request("tools/list", None)
            .await
            .context("Failed to list tools")?;

        // Parse the response - tools are in result.tools
        let tools_value = result
            .get("tools")
            .context("Response missing 'tools' field")?
            .as_array()
            .context("'tools' is not an array")?;

        let mut tools = Vec::new();
        for tool_value in tools_value {
            let tool: Tool =
                serde_json::from_value(tool_value.clone()).context("Failed to parse tool")?;
            tools.push(tool);
        }

        Ok(tools)
    }

    /// Call a tool
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<JsonValue>,
    ) -> Result<CallToolResult> {
        if !self.supports_tools() {
            bail!("Server does not support tools");
        }

        let params = CallToolParams {
            name: name.to_string(),
            arguments,
        };

        let result = self
            .transport
            .send_request("tools/call", Some(serde_json::to_value(params)?))
            .await
            .context(format!("Failed to call tool '{}'", name))?;

        let call_result: CallToolResult =
            serde_json::from_value(result).context("Failed to parse tool call result")?;

        Ok(call_result)
    }

    /// List available resources
    #[allow(dead_code)]
    pub async fn list_resources(&mut self) -> Result<Vec<Resource>> {
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }

        let result = self
            .transport
            .send_request("resources/list", None)
            .await
            .context("Failed to list resources")?;

        let resources_value = result
            .get("resources")
            .context("Response missing 'resources' field")?
            .as_array()
            .context("'resources' is not an array")?;

        let mut resources = Vec::new();
        for resource_value in resources_value {
            let resource: Resource = serde_json::from_value(resource_value.clone())
                .context("Failed to parse resource")?;
            resources.push(resource);
        }

        Ok(resources)
    }

    /// Read a resource
    #[allow(dead_code)]
    pub async fn read_resource(&mut self, uri: &str) -> Result<ResourceContents> {
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }

        let params = json!({ "uri": uri });

        let result = self
            .transport
            .send_request("resources/read", Some(params))
            .await
            .context(format!("Failed to read resource '{}'", uri))?;

        parse_read_resource_result(result).context("Failed to parse resource contents")
    }

    pub async fn subscribe_resource(&mut self, uri: &str) -> Result<()> {
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }
        if !self.supports_resource_subscribe() {
            bail!("Server does not support resources.subscribe");
        }

        let params = json!({ "uri": uri });
        self.transport
            .send_request("resources/subscribe", Some(params))
            .await
            .context(format!("Failed to subscribe resource '{}'", uri))?;
        Ok(())
    }

    pub async fn unsubscribe_resource(&mut self, uri: &str) -> Result<()> {
        if !self.supports_resources() {
            bail!("Server does not support resources");
        }

        let params = json!({ "uri": uri });
        self.transport
            .send_request("resources/unsubscribe", Some(params))
            .await
            .context(format!("Failed to unsubscribe resource '{}'", uri))?;
        Ok(())
    }

    /// List available prompts
    #[allow(dead_code)]
    pub async fn list_prompts(&mut self) -> Result<Vec<Prompt>> {
        if !self.supports_prompts() {
            bail!("Server does not support prompts");
        }

        let result = self
            .transport
            .send_request("prompts/list", None)
            .await
            .context("Failed to list prompts")?;

        let prompts_value = result
            .get("prompts")
            .context("Response missing 'prompts' field")?
            .as_array()
            .context("'prompts' is not an array")?;

        let mut prompts = Vec::new();
        for prompt_value in prompts_value {
            let prompt: Prompt =
                serde_json::from_value(prompt_value.clone()).context("Failed to parse prompt")?;
            prompts.push(prompt);
        }

        Ok(prompts)
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
            .transport
            .send_request("prompts/get", Some(params))
            .await
            .context(format!("Failed to get prompt '{}'", name))?;

        let prompt_result: GetPromptResult =
            serde_json::from_value(result).context("Failed to parse prompt result")?;

        Ok(prompt_result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::structured_error_from_anyhow;

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
        assert_eq!(tools[0].description, "A test tool");
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

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            ToolContent::Text { text } => assert_eq!(text, "Tool executed successfully"),
            _ => panic!("Expected text content"),
        }
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
        match text {
            ToolContent::Text { text: t } => assert_eq!(t, "Hello"),
            _ => panic!("Expected text content"),
        }

        let image_json = r#"{"type":"image","data":"base64data","mimeType":"image/png"}"#;
        let image: ToolContent = serde_json::from_str(image_json).unwrap();
        match image {
            ToolContent::Image { data, mimeType } => {
                assert_eq!(data, "base64data");
                assert_eq!(mimeType, "image/png");
            }
            _ => panic!("Expected image content"),
        }

        let resource_json = r#"{"type":"resource","uri":"test://resource","text":"content"}"#;
        let resource: ToolContent = serde_json::from_str(resource_json).unwrap();
        match resource {
            ToolContent::Resource { uri, text, .. } => {
                assert_eq!(uri, "test://resource");
                assert_eq!(text.unwrap(), "content");
            }
            _ => panic!("Expected resource content"),
        }
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
    async fn probe_can_reap_parses_successful_response() {
        let script = r#"
            read line
            echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}'
            read line
            echo '{"jsonrpc":"2.0","id":2,"result":{"can_reap":false,"reason":"interactive_session","retry_after_secs":30,"state":{"interactive":true,"owns_external_resource":true}}}'
        "#;

        let mut client = McpStdioClient::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        let result = client
            .probe_can_reap(123, 600, Duration::from_millis(50))
            .await
            .unwrap();
        assert!(!result.can_reap);
        assert_eq!(result.reason.as_deref(), Some("interactive_session"));
        assert_eq!(result.retry_after_secs, Some(30));
        assert_eq!(
            result.state,
            Some(CanReapState {
                interactive: Some(true),
                owns_external_resource: Some(true),
                waiting_for_human: None,
            })
        );
    }

    #[tokio::test]
    async fn probe_can_reap_surfaces_method_not_found_code() {
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
            .probe_can_reap(123, 600, Duration::from_millis(50))
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
