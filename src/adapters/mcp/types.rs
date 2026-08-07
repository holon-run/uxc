//! MCP (Model Context Protocol) types and JSON-RPC messages

#![allow(non_snake_case)]

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<JsonValue>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 Notification (no response expected)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<JsonValue>,
}

/// JSON-RPC Request ID
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

/// JSON-RPC Error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
}

/// Legacy MCP protocol version currently supported by UXC.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
/// Latest stateless MCP protocol version supported by UXC.
pub const MCP_MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpProtocolEra {
    Legacy,
    Modern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpProtocolContext {
    pub era: McpProtocolEra,
    pub version: String,
    pub client_capabilities: ClientCapabilities,
    pub server_capabilities: ServerCapabilities,
    pub client_info: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_info: Option<ServerInfo>,
}

impl McpProtocolContext {
    pub fn modern_request_params(&self, params: Option<JsonValue>) -> Result<JsonValue> {
        let mut params = match params {
            Some(JsonValue::Object(params)) => params,
            Some(_) => return Err(anyhow!("MCP request params must be a JSON object")),
            None => serde_json::Map::new(),
        };
        let mut meta = match params.remove("_meta") {
            Some(JsonValue::Object(meta)) => meta,
            Some(_) => return Err(anyhow!("MCP request params._meta must be a JSON object")),
            None => serde_json::Map::new(),
        };
        meta.insert(
            "io.modelcontextprotocol/protocolVersion".to_string(),
            JsonValue::String(self.version.clone()),
        );
        meta.insert(
            "io.modelcontextprotocol/clientInfo".to_string(),
            serde_json::to_value(&self.client_info)?,
        );
        meta.insert(
            "io.modelcontextprotocol/clientCapabilities".to_string(),
            serde_json::to_value(&self.client_capabilities)?,
        );
        params.insert("_meta".to_string(), JsonValue::Object(meta));
        Ok(JsonValue::Object(params))
    }
}

/// Initialize request parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocolVersion: String,
    pub capabilities: ClientCapabilities,
    pub clientInfo: ClientInfo,
}

/// Client capabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elicitation: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<HashMap<String, JsonValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, JsonValue>>,
    #[serde(flatten)]
    pub additional: HashMap<String, JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RootsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SamplingCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

/// Client information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websiteUrl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<JsonValue>>,
}

/// Initialize response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocolVersion: String,
    pub capabilities: ServerCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serverInfo: Option<ServerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Server capabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, JsonValue>>,
    #[serde(flatten)]
    pub additional: HashMap<String, JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourcesCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listChanged: Option<bool>,
}

/// Server information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websiteUrl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<JsonValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverResult {
    pub supportedVersions: Vec<String>,
    pub capabilities: ServerCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default = "complete_result_type")]
    pub resultType: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttlMs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cacheScope: Option<String>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonValue>,
}

impl DiscoverResult {
    pub fn server_info(&self) -> Option<ServerInfo> {
        self.meta
            .as_ref()
            .and_then(|meta| meta.get("io.modelcontextprotocol/serverInfo"))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputSchema: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputSchema: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<JsonValue>>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonValue>,
}

impl Tool {
    pub fn display_name(&self) -> &str {
        self.title
            .as_deref()
            .or_else(|| {
                self.annotations
                    .as_ref()
                    .and_then(|annotations| annotations.get("title"))
                    .and_then(JsonValue::as_str)
            })
            .unwrap_or(&self.name)
    }
}

/// Tool call parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<JsonValue>,
}

/// Tool call result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolResult {
    #[serde(default = "complete_result_type")]
    pub resultType: String,
    #[serde(default)]
    pub content: Vec<ToolContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isError: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structuredContent: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputRequests: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requestState: Option<String>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonValue>,
}

fn complete_result_type() -> String {
    "complete".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct ToolContent(pub JsonValue);

impl ToolContent {
    #[cfg(test)]
    pub fn content_type(&self) -> Option<&str> {
        self.0.get("type").and_then(JsonValue::as_str)
    }

    pub fn as_value(&self) -> &JsonValue {
        &self.0
    }
}

/// Resource definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimeType: Option<String>,
}

/// Resource content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimeType: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResourceResponse {
    pub contents: Vec<ResourceContents>,
}

pub fn parse_read_resource_result(result: JsonValue) -> Result<ResourceContents> {
    if let Ok(contents) = serde_json::from_value::<ResourceContents>(result.clone()) {
        return Ok(contents);
    }

    let response: ReadResourceResponse =
        serde_json::from_value(result).context("Failed to parse resources/read result")?;
    response
        .contents
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("resources/read response contained no contents"))
}

/// Prompt definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Prompt message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: PromptContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptContent {
    Text(String),
    Image { data: String, mimeType: String },
    Resource { uri: String },
}

/// Get prompt result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetPromptResult {
    pub description: String,
    pub messages: Vec<PromptMessage>,
}

/// Tools list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsListResponse {
    pub tools: Vec<Tool>,
}

/// Tool call result (alias for CallToolResult)
pub type ToolCallResult = CallToolResult;

/// Resources list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesListResponse {
    pub resources: Vec<Resource>,
}

/// Prompts list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsListResponse {
    pub prompts: Vec<Prompt>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn modern_context() -> McpProtocolContext {
        McpProtocolContext {
            era: McpProtocolEra::Modern,
            version: MCP_MODERN_PROTOCOL_VERSION.to_string(),
            client_capabilities: ClientCapabilities::default(),
            server_capabilities: ServerCapabilities::default(),
            client_info: ClientInfo {
                name: "uxc".to_string(),
                version: "1.0.0".to_string(),
                title: None,
                description: None,
                websiteUrl: None,
                icons: None,
            },
            server_info: None,
        }
    }

    #[test]
    fn modern_request_params_add_required_metadata_and_preserve_existing_meta() {
        let params = modern_context()
            .modern_request_params(Some(json!({
                "name": "ping",
                "_meta": {
                    "progressToken": "token-1",
                    "com.example/custom": true
                }
            })))
            .unwrap();

        assert_eq!(params["name"], "ping");
        assert_eq!(params["_meta"]["progressToken"], "token-1");
        assert_eq!(params["_meta"]["com.example/custom"], true);
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/protocolVersion"],
            MCP_MODERN_PROTOCOL_VERSION
        );
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
            "uxc"
        );
    }

    #[test]
    fn legacy_tool_result_defaults_to_complete_and_preserves_unknown_content() {
        let result: CallToolResult = serde_json::from_value(json!({
            "content": [{
                "type": "future-content",
                "nested": {"value": 1}
            }]
        }))
        .unwrap();

        assert_eq!(result.resultType, "complete");
        assert_eq!(result.content[0].content_type(), Some("future-content"));
        assert_eq!(result.content[0].as_value()["nested"]["value"], 1);
    }
}
