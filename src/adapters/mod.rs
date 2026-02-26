//! Protocol adapters for different schema types
//!
//! Each adapter implements a common interface for:
//! - Protocol detection
//! - Schema retrieval
//! - Operation discovery
//! - Execution

pub mod graphql;
pub mod grpc;
pub mod jsonrpc;
pub mod mcp;
pub mod openapi;

use crate::error::UxcError;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Enum of all available adapters
#[allow(non_camel_case_types)]
pub enum AdapterEnum {
    OpenAPI(openapi::OpenAPIAdapter),
    GRpc(grpc::GrpcAdapter),
    JsonRpc(jsonrpc::JsonRpcAdapter),
    Mcp(mcp::McpAdapter),
    GraphQL(graphql::GraphQLAdapter),
}

#[async_trait]
impl Adapter for AdapterEnum {
    fn protocol_type(&self) -> ProtocolType {
        match self {
            AdapterEnum::OpenAPI(_) => ProtocolType::OpenAPI,
            AdapterEnum::GRpc(_) => ProtocolType::GRpc,
            AdapterEnum::JsonRpc(_) => ProtocolType::JsonRpc,
            AdapterEnum::Mcp(_) => ProtocolType::Mcp,
            AdapterEnum::GraphQL(_) => ProtocolType::GraphQL,
        }
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        match self {
            AdapterEnum::OpenAPI(a) => a.can_handle(url).await,
            AdapterEnum::GRpc(a) => a.can_handle(url).await,
            AdapterEnum::JsonRpc(a) => a.can_handle(url).await,
            AdapterEnum::Mcp(a) => a.can_handle(url).await,
            AdapterEnum::GraphQL(a) => a.can_handle(url).await,
        }
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        match self {
            AdapterEnum::OpenAPI(a) => a.fetch_schema(url).await,
            AdapterEnum::GRpc(a) => a.fetch_schema(url).await,
            AdapterEnum::JsonRpc(a) => a.fetch_schema(url).await,
            AdapterEnum::Mcp(a) => a.fetch_schema(url).await,
            AdapterEnum::GraphQL(a) => a.fetch_schema(url).await,
        }
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        match self {
            AdapterEnum::OpenAPI(a) => a.list_operations(url).await,
            AdapterEnum::GRpc(a) => a.list_operations(url).await,
            AdapterEnum::JsonRpc(a) => a.list_operations(url).await,
            AdapterEnum::Mcp(a) => a.list_operations(url).await,
            AdapterEnum::GraphQL(a) => a.list_operations(url).await,
        }
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        match self {
            AdapterEnum::OpenAPI(a) => a.describe_operation(url, operation).await,
            AdapterEnum::GRpc(a) => a.describe_operation(url, operation).await,
            AdapterEnum::JsonRpc(a) => a.describe_operation(url, operation).await,
            AdapterEnum::Mcp(a) => a.describe_operation(url, operation).await,
            AdapterEnum::GraphQL(a) => a.describe_operation(url, operation).await,
        }
    }

    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult> {
        match self {
            AdapterEnum::OpenAPI(a) => a.execute(url, operation, args).await,
            AdapterEnum::GRpc(a) => a.execute(url, operation, args).await,
            AdapterEnum::JsonRpc(a) => a.execute(url, operation, args).await,
            AdapterEnum::Mcp(a) => a.execute(url, operation, args).await,
            AdapterEnum::GraphQL(a) => a.execute(url, operation, args).await,
        }
    }
}

/// Supported protocol types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ProtocolType {
    OpenAPI,
    GRpc,
    JsonRpc,
    Mcp,
    GraphQL,
}

impl ProtocolType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProtocolType::OpenAPI => "openapi",
            ProtocolType::GRpc => "grpc",
            ProtocolType::JsonRpc => "jsonrpc",
            ProtocolType::Mcp => "mcp",
            ProtocolType::GraphQL => "graphql",
        }
    }
}

/// Operation metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub operation_id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub parameters: Vec<Parameter>,
    #[allow(dead_code)]
    pub return_type: Option<String>,
}

/// Parameter definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub param_type: String,
    pub required: bool,
    pub description: Option<String>,
}

/// Rich operation metadata for progressive discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDetail {
    pub operation_id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<String>,
    pub input_schema: Option<Value>,
}

/// Execution result
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub data: Value,
    pub metadata: ExecutionMetadata,
}

#[derive(Debug, Clone)]
pub struct ExecutionMetadata {
    pub duration_ms: u64,
    #[allow(dead_code)]
    pub operation: String,
}

/// Adapter trait - must be implemented by all protocol adapters
#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    /// Get the protocol type this adapter handles
    fn protocol_type(&self) -> ProtocolType;

    /// Detect if this adapter can handle the given endpoint
    async fn can_handle(&self, url: &str) -> Result<bool>;

    /// Retrieve schema from the endpoint
    async fn fetch_schema(&self, url: &str) -> Result<Value>;

    /// List available operations
    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>>;

    /// Get rich metadata for a specific operation
    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail>;

    /// Execute an operation
    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult>;
}

/// Protocol detector - attempts to identify the protocol type
pub struct ProtocolDetector;

/// Optional settings that affect protocol detection.
#[derive(Debug, Clone, Default)]
pub struct DetectionOptions {
    pub schema_url: Option<String>,
    pub auth_profile: Option<crate::auth::Profile>,
}

impl ProtocolDetector {
    pub fn new() -> Self {
        Self
    }

    /// Get adapter for a URL (auto-detects protocol)
    pub async fn detect_adapter(&self, url: &str) -> Result<AdapterEnum> {
        self.detect_adapter_with_options(url, &DetectionOptions::default())
            .await
    }

    /// Get adapter for a URL using explicit detection options.
    pub async fn detect_adapter_with_options(
        &self,
        url: &str,
        options: &DetectionOptions,
    ) -> Result<AdapterEnum> {
        // Try MCP first (stdio commands are distinct)
        let mcp_adapter = if let Some(profile) = options.auth_profile.clone() {
            mcp::McpAdapter::new().with_auth(profile)
        } else {
            mcp::McpAdapter::new()
        };
        if mcp_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::Mcp(mcp_adapter));
        }

        // Try GraphQL (introspection is reliable)
        let graphql_adapter = graphql::GraphQLAdapter::new();
        if graphql_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::GraphQL(graphql_adapter));
        }

        // Try OpenAPI
        let openapi_adapter =
            openapi::OpenAPIAdapter::new().with_schema_url_override(options.schema_url.clone());
        if openapi_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::OpenAPI(openapi_adapter));
        }

        // Try JSON-RPC (OpenRPC discovery)
        let jsonrpc_adapter = jsonrpc::JsonRpcAdapter::new();
        if jsonrpc_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::JsonRpc(jsonrpc_adapter));
        }

        // Try gRPC (less reliable detection, try last)
        let grpc_adapter = grpc::GrpcAdapter::new();
        if grpc_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::GRpc(grpc_adapter));
        }

        let mut message = format!("No adapter found for URL: {}", url);
        if let Some(diag) = mcp_adapter.latest_probe_diagnostics().await {
            message.push_str(&format!(". MCP probe diagnostics: {}", diag));
        }

        Err(UxcError::ProtocolDetectionFailed(message).into())
    }
}

impl Default for ProtocolDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthType, Profile};

    #[tokio::test]
    async fn detector_uses_auth_profile_for_mcp_probe() {
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
            .match_header("authorization", "Bearer test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
  "jsonrpc":"2.0",
  "id":1,
  "result":{
    "protocolVersion":"2024-11-05",
    "capabilities":{"tools":{}},
    "serverInfo":{"name":"mock-mcp","version":"1.0.0"}
  }
}"#,
            )
            .create_async()
            .await;

        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: None,
            auth_profile: Some(Profile::new("test-token".to_string(), AuthType::Bearer)),
        };

        let adapter = detector
            .detect_adapter_with_options(&server.url(), &options)
            .await
            .unwrap();
        assert!(matches!(adapter, AdapterEnum::Mcp(_)));
    }

    #[tokio::test]
    async fn detector_does_not_detect_mcp_without_auth_profile() {
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
            .match_header("authorization", "Bearer test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
  "jsonrpc":"2.0",
  "id":1,
  "result":{
    "protocolVersion":"2024-11-05",
    "capabilities":{"tools":{}},
    "serverInfo":{"name":"mock-mcp","version":"1.0.0"}
  }
}"#,
            )
            .create_async()
            .await;

        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: None,
            auth_profile: None,
        };

        let result = detector
            .detect_adapter_with_options(&server.url(), &options)
            .await;
        match result {
            Ok(adapter) => assert!(!matches!(adapter, AdapterEnum::Mcp(_))),
            Err(_) => {}
        }
    }
}
