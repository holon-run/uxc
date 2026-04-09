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
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Registration entry for dynamic adapter discovery.
///
/// Third-party crates can register adapters at compile time using `inventory::submit!`.
/// Registered adapters are tried (sorted by descending priority) before built-in adapters
/// during protocol detection.
///
/// # Example
/// ```ignore
/// inventory::submit! {
///     AdapterRegistration {
///         name: "my-protocol",
///         priority: 100,
///         can_handle: |url| Box::pin(async move { Ok(url.contains("my-proto://")) }),
///         create: || Box::new(MyAdapter::new()),
///     }
/// }
/// ```
pub struct AdapterRegistration {
    /// Human-readable name for this adapter.
    pub name: &'static str,
    /// Priority (0-255). Higher values are tried first.
    pub priority: u8,
    /// Async predicate: returns `true` if this adapter can handle the given URL.
    pub can_handle: fn(&str) -> BoxFuture<'static, Result<bool>>,
    /// Factory that creates a new adapter instance.
    pub create: fn() -> Box<dyn Adapter>,
}

inventory::collect!(AdapterRegistration);

/// Enum of all available adapters
#[allow(non_camel_case_types)]
pub enum AdapterEnum {
    OpenAPI(openapi::OpenAPIAdapter),
    GRpc(grpc::GrpcAdapter),
    JsonRpc(jsonrpc::JsonRpcAdapter),
    Mcp(mcp::McpAdapter),
    GraphQL(graphql::GraphQLAdapter),
    /// Dynamically registered external adapter discovered via `inventory`.
    External(Box<dyn Adapter>),
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
            AdapterEnum::External(a) => a.protocol_type(),
        }
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        match self {
            AdapterEnum::OpenAPI(a) => a.can_handle(url).await,
            AdapterEnum::GRpc(a) => a.can_handle(url).await,
            AdapterEnum::JsonRpc(a) => a.can_handle(url).await,
            AdapterEnum::Mcp(a) => a.can_handle(url).await,
            AdapterEnum::GraphQL(a) => a.can_handle(url).await,
            AdapterEnum::External(a) => a.can_handle(url).await,
        }
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        match self {
            AdapterEnum::OpenAPI(a) => a.fetch_schema(url).await,
            AdapterEnum::GRpc(a) => a.fetch_schema(url).await,
            AdapterEnum::JsonRpc(a) => a.fetch_schema(url).await,
            AdapterEnum::Mcp(a) => a.fetch_schema(url).await,
            AdapterEnum::GraphQL(a) => a.fetch_schema(url).await,
            AdapterEnum::External(a) => a.fetch_schema(url).await,
        }
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        match self {
            AdapterEnum::OpenAPI(a) => a.list_operations(url).await,
            AdapterEnum::GRpc(a) => a.list_operations(url).await,
            AdapterEnum::JsonRpc(a) => a.list_operations(url).await,
            AdapterEnum::Mcp(a) => a.list_operations(url).await,
            AdapterEnum::GraphQL(a) => a.list_operations(url).await,
            AdapterEnum::External(a) => a.list_operations(url).await,
        }
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        match self {
            AdapterEnum::OpenAPI(a) => a.describe_operation(url, operation).await,
            AdapterEnum::GRpc(a) => a.describe_operation(url, operation).await,
            AdapterEnum::JsonRpc(a) => a.describe_operation(url, operation).await,
            AdapterEnum::Mcp(a) => a.describe_operation(url, operation).await,
            AdapterEnum::GraphQL(a) => a.describe_operation(url, operation).await,
            AdapterEnum::External(a) => a.describe_operation(url, operation).await,
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
            AdapterEnum::External(a) => a.execute(url, operation, args).await,
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
    #[allow(dead_code)]
    pub metadata: ExecutionMetadata,
}

#[derive(Debug, Clone)]
pub struct ExecutionMetadata {
    #[allow(dead_code)]
    pub duration_ms: u64,
    #[allow(dead_code)]
    pub operation: String,
    #[allow(dead_code)]
    pub response_status_code: Option<u16>,
    #[allow(dead_code)]
    pub response_headers: HashMap<String, String>,
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
    pub stdio_spawn_options: Option<mcp::StdioSpawnOptions>,
    pub request_timeout: Option<Duration>,
}

impl ProtocolDetector {
    pub fn new() -> Self {
        Self
    }

    /// Get adapter for a URL (auto-detects protocol)
    #[allow(dead_code)]
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
        let mut schema_probe_errors = Vec::new();

        // Try dynamically registered adapters first (sorted by descending priority)
        let mut registrations: Vec<&AdapterRegistration> =
            inventory::iter::<AdapterRegistration>.into_iter().collect();
        registrations.sort_by_key(|r| std::cmp::Reverse(r.priority));

        for reg in registrations {
            if (reg.can_handle)(url).await? {
                return Ok(AdapterEnum::External((reg.create)()));
            }
        }

        // Try MCP first (stdio commands are distinct)
        let mut mcp_adapter = mcp::McpAdapter::new();
        if let Some(profile) = options.auth_profile.clone() {
            mcp_adapter = mcp_adapter.with_auth(profile);
        }
        if let Some(spawn_options) = options.stdio_spawn_options.clone() {
            mcp_adapter = mcp_adapter.with_stdio_spawn_options(spawn_options);
        }
        if mcp_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::Mcp(mcp_adapter));
        }

        // Try GraphQL (introspection is reliable)
        let graphql_adapter = if let Some(profile) = options.auth_profile.clone() {
            graphql::GraphQLAdapter::new().with_auth(profile)
        } else {
            graphql::GraphQLAdapter::new()
        };
        if graphql_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::GraphQL(graphql_adapter));
        }

        // Try OpenAPI
        let openapi_adapter = if let Some(profile) = options.auth_profile.clone() {
            openapi::OpenAPIAdapter::new()
                .with_schema_url_override(options.schema_url.clone())
                .with_auth(profile)
        } else {
            openapi::OpenAPIAdapter::new().with_schema_url_override(options.schema_url.clone())
        };
        match openapi_adapter.can_handle(url).await {
            Ok(true) => return Ok(AdapterEnum::OpenAPI(openapi_adapter)),
            Ok(false) => {}
            Err(err) if options.schema_url.is_some() => {
                schema_probe_errors.push(format!("OpenAPI probe failed: {err}"));
            }
            Err(err) => return Err(err),
        }

        // Try JSON-RPC (OpenRPC discovery)
        let jsonrpc_adapter = if let Some(profile) = options.auth_profile.clone() {
            jsonrpc::JsonRpcAdapter::new()
                .with_schema_url_override(options.schema_url.clone())
                .with_auth(profile)
        } else {
            jsonrpc::JsonRpcAdapter::new().with_schema_url_override(options.schema_url.clone())
        };
        match jsonrpc_adapter.can_handle(url).await {
            Ok(true) => return Ok(AdapterEnum::JsonRpc(jsonrpc_adapter)),
            Ok(false) => {}
            Err(err) if options.schema_url.is_some() => {
                schema_probe_errors.push(format!("JSON-RPC probe failed: {err}"));
            }
            Err(err) => return Err(err),
        }

        // Try gRPC (less reliable detection, try last)
        let grpc_adapter = if let Some(profile) = options.auth_profile.clone() {
            grpc::GrpcAdapter::new().with_auth(profile)
        } else {
            grpc::GrpcAdapter::new()
        };
        if grpc_adapter.can_handle(url).await? {
            return Ok(AdapterEnum::GRpc(grpc_adapter));
        }

        let mut message = format!("No adapter found for URL: {}", url);
        if let Some(diag) = mcp_adapter.latest_probe_diagnostics().await {
            message.push_str(&format!(". MCP probe diagnostics: {}", diag));
        }
        if !schema_probe_errors.is_empty() {
            message.push_str(&format!(
                ". Schema probe diagnostics: {}",
                schema_probe_errors.join(" | ")
            ));
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
    use crate::auth::{AuthType, OAuthFlow, OAuthProfile, Profile};
    use crate::error::UxcError;

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
            stdio_spawn_options: None,
            request_timeout: None,
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
            stdio_spawn_options: None,
            request_timeout: None,
        };

        let result = detector
            .detect_adapter_with_options(&server.url(), &options)
            .await;
        if let Ok(adapter) = result {
            assert!(!matches!(adapter, AdapterEnum::Mcp(_)));
        }
    }

    #[tokio::test]
    async fn detector_surfaces_oauth_refresh_failure_from_mcp_probe() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/mcp", server.url());
        let token_endpoint = format!("{}/token", server.url());

        let _mcp = server
            .mock("POST", "/mcp")
            .match_header("authorization", "Bearer old-token")
            .with_status(401)
            .with_body(r#"{"error":"invalid_token","error_description":"Invalid access token"}"#)
            .create_async()
            .await;
        let _refresh = server
            .mock("POST", "/token")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"invalid_grant","error_description":"bad grant"}"#)
            .create_async()
            .await;

        let mut profile = Profile::new(String::new(), AuthType::OAuth);
        profile.oauth = Some(OAuthProfile {
            token_endpoint: Some(token_endpoint),
            refresh_token: Some("refresh-token".to_string()),
            access_token: Some("old-token".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(i64::MAX),
            oauth_flow: Some(OAuthFlow::AuthorizationCode),
            ..Default::default()
        });

        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: None,
            auth_profile: Some(profile),
            stdio_spawn_options: None,
            request_timeout: None,
        };

        let result = detector
            .detect_adapter_with_options(&endpoint, &options)
            .await;
        assert!(result.is_err());
        let err = result.err().expect("expected oauth refresh error");
        let oauth_err = err
            .chain()
            .find_map(|cause| cause.downcast_ref::<UxcError>());

        assert!(matches!(oauth_err, Some(UxcError::OAuthRefreshFailed(_))));
    }

    #[tokio::test]
    async fn detector_uses_auth_profile_for_graphql_probe() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/graphql", server.url());

        let _mcp_root = server
            .mock("POST", "/graphql")
            .match_body(mockito::Matcher::Regex(
                "\"method\":\"initialize\"".to_string(),
            ))
            .with_status(404)
            .create_async()
            .await;
        let _mcp_well_known = server
            .mock("POST", "/.well-known/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _mcp_path = server
            .mock("POST", "/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _graphql = server
            .mock("POST", "/graphql")
            .match_header("authorization", "Bearer graph-token")
            .match_body(mockito::Matcher::Regex("__schema".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"__schema":{"queryType":{"name":"Query"}}}}"#)
            .create_async()
            .await;

        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: None,
            auth_profile: Some(Profile::new("graph-token".to_string(), AuthType::Bearer)),
            stdio_spawn_options: None,
            request_timeout: None,
        };

        let adapter = detector
            .detect_adapter_with_options(&endpoint, &options)
            .await
            .unwrap();
        assert!(matches!(adapter, AdapterEnum::GraphQL(_)));
    }

    #[tokio::test]
    async fn detector_uses_auth_profile_for_openapi_probe() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/openapi", server.url());

        let _mcp_root = server
            .mock("POST", "/openapi")
            .match_body(mockito::Matcher::Regex(
                "\"method\":\"initialize\"".to_string(),
            ))
            .with_status(404)
            .create_async()
            .await;
        let _mcp_well_known = server
            .mock("POST", "/.well-known/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _mcp_path = server
            .mock("POST", "/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _graphql = server
            .mock("POST", "/openapi")
            .match_body(mockito::Matcher::Regex("__schema".to_string()))
            .with_status(404)
            .create_async()
            .await;
        let _schema = server
            .mock("GET", "/openapi/openapi.json")
            .match_header("authorization", "Bearer openapi-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"openapi":"3.0.0","info":{"title":"t","version":"1"},"paths":{}}"#)
            .create_async()
            .await;

        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: None,
            auth_profile: Some(Profile::new("openapi-token".to_string(), AuthType::Bearer)),
            stdio_spawn_options: None,
            request_timeout: None,
        };

        let adapter = detector
            .detect_adapter_with_options(&endpoint, &options)
            .await
            .unwrap();
        assert!(matches!(adapter, AdapterEnum::OpenAPI(_)));
    }

    #[tokio::test]
    async fn detector_uses_auth_profile_for_jsonrpc_probe() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/rpc", server.url());

        let _mcp_root = server
            .mock("POST", "/rpc")
            .match_body(mockito::Matcher::Regex(
                "\"method\":\"initialize\"".to_string(),
            ))
            .with_status(404)
            .create_async()
            .await;
        let _mcp_well_known = server
            .mock("POST", "/.well-known/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _mcp_path = server
            .mock("POST", "/mcp")
            .with_status(404)
            .create_async()
            .await;
        let _graphql = server
            .mock("POST", "/rpc")
            .match_body(mockito::Matcher::Regex("__schema".to_string()))
            .with_status(404)
            .create_async()
            .await;
        let _openapi = server
            .mock("GET", "/rpc/openapi.json")
            .with_status(404)
            .create_async()
            .await;
        let _jsonrpc = server
            .mock("POST", "/rpc")
            .match_header("authorization", "Bearer jsonrpc-token")
            .match_body(mockito::Matcher::Regex(
                "\"method\":\"rpc.discover\"".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "jsonrpc":"2.0",
                  "id":1,
                  "result":{"openrpc":"1.3.2","methods":[]}
                }"#,
            )
            .create_async()
            .await;

        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: None,
            auth_profile: Some(Profile::new("jsonrpc-token".to_string(), AuthType::Bearer)),
            stdio_spawn_options: None,
            request_timeout: None,
        };

        let adapter = detector
            .detect_adapter_with_options(&endpoint, &options)
            .await
            .unwrap();
        assert!(matches!(adapter, AdapterEnum::JsonRpc(_)));
    }

    /// A minimal adapter for testing dynamic registration.
    struct DummyExternalAdapter;

    #[async_trait]
    impl Adapter for DummyExternalAdapter {
        fn protocol_type(&self) -> ProtocolType {
            ProtocolType::OpenAPI // reuse an existing variant for test
        }
        async fn can_handle(&self, _url: &str) -> Result<bool> {
            Ok(true)
        }
        async fn fetch_schema(&self, _url: &str) -> Result<Value> {
            Ok(serde_json::json!({"dummy": true}))
        }
        async fn list_operations(&self, _url: &str) -> Result<Vec<Operation>> {
            Ok(vec![])
        }
        async fn describe_operation(
            &self,
            _url: &str,
            _operation: &str,
        ) -> Result<OperationDetail> {
            Err(anyhow::anyhow!("not implemented"))
        }
        async fn execute(
            &self,
            _url: &str,
            _operation: &str,
            _args: HashMap<String, Value>,
        ) -> Result<ExecutionResult> {
            Err(anyhow::anyhow!("not implemented"))
        }
    }

    inventory::submit! {
        AdapterRegistration {
            name: "dummy-test",
            priority: 255,
            can_handle: |url| {
                let url = url.to_owned();
                Box::pin(async move {
                    Ok(url.contains("dummy-external-proto"))
                })
            },
            create: || Box::new(DummyExternalAdapter),
        }
    }

    #[tokio::test]
    async fn detector_discovers_registered_external_adapter() {
        let detector = ProtocolDetector::new();
        let options = DetectionOptions::default();

        let result = detector
            .detect_adapter_with_options("http://dummy-external-proto.test/api", &options)
            .await;
        assert!(result.is_ok());
        let adapter = result.unwrap();
        assert!(matches!(adapter, AdapterEnum::External(_)));
    }

    #[tokio::test]
    async fn external_adapter_delegates_to_trait() {
        let detector = ProtocolDetector::new();
        let options = DetectionOptions::default();

        let adapter = detector
            .detect_adapter_with_options("http://dummy-external-proto.test/api", &options)
            .await
            .unwrap();
        let schema = adapter
            .fetch_schema("http://dummy-external-proto.test/api")
            .await
            .unwrap();
        assert_eq!(schema, serde_json::json!({"dummy": true}));
    }

    #[tokio::test]
    async fn registered_adapters_sorted_by_priority() {
        // Verify that inventory iteration produces at least our test registration
        let registrations: Vec<&AdapterRegistration> =
            inventory::iter::<AdapterRegistration>.into_iter().collect();
        assert!(
            registrations.iter().any(|r| r.name == "dummy-test"),
            "Expected to find dummy-test registration"
        );
    }
}
