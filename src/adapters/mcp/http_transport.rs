//! MCP HTTP transport for communicating with MCP servers over HTTP/HTTPS

#![allow(dead_code)]

use super::types::*;
use crate::auth::{oauth, AuthType, Profile, Profiles};
use crate::error::UxcError;
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tokio::sync::Mutex;

const PROBE_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeInitializeOutcome {
    Success,
    NotMcp(String),
}

/// MCP HTTP transport client
#[derive(Debug)]
pub struct McpHttpTransport {
    /// HTTP client
    client: Client,
    /// Server URL
    server_url: String,
    /// Request ID counter
    next_id: Arc<Mutex<i64>>,
    /// Authentication profile
    auth_profile: Arc<Mutex<Option<Profile>>>,
    /// Lock for OAuth refresh operations
    oauth_refresh_lock: Arc<Mutex<()>>,
}

impl McpHttpTransport {
    /// Create a new HTTP transport connected to the given URL
    pub fn new(url: String) -> Result<Self> {
        Self::with_auth(url, None)
    }

    /// Create a new HTTP transport with authentication
    pub fn with_auth(url: String, auth_profile: Option<Profile>) -> Result<Self> {
        // Validate URL
        let parsed = url::Url::parse(&url).context("Invalid MCP server URL")?;

        // Ensure it's http or https
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            bail!(
                "MCP HTTP transport only supports http:// and https:// URLs, got: {}",
                parsed.scheme()
            );
        }

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            server_url: url,
            next_id: Arc::new(Mutex::new(1i64)),
            auth_profile: Arc::new(Mutex::new(auth_profile)),
            oauth_refresh_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Send a request and wait for response
    pub async fn send_request(&self, method: &str, params: Option<JsonValue>) -> Result<JsonValue> {
        self.maybe_refresh_oauth_token().await?;

        // Generate request ID
        let id = {
            let mut next_id = self.next_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };

        // Build JSON-RPC request
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: RequestId::Number(id),
        };

        tracing::debug!(
            "Sending MCP HTTP request: {} to {}",
            method,
            self.server_url
        );

        let mut response = self
            .send_jsonrpc_request(&request)
            .await
            .context("Failed to send HTTP request to MCP server")?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED && self.is_oauth_profile().await {
            self.force_refresh_oauth_token().await?;
            response = self
                .send_jsonrpc_request(&request)
                .await
                .context("Failed to send HTTP retry request to MCP server")?;
        }

        let status = response.status();
        let www_authenticate = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unable to read response body".to_string());

        // Check HTTP status
        if !status.is_success() {
            return Self::map_http_error(status, &body, www_authenticate.as_deref());
        }

        // Parse JSON or streamable HTTP (SSE) response
        let json_response = Self::parse_jsonrpc_response(content_type.as_deref(), &body)
            .context("Failed to parse MCP server response")?;

        // Check for JSON-RPC error
        if let Some(error) = json_response.error {
            bail!(
                "MCP server returned error: {} - {}",
                error.code,
                error.message
            );
        }

        // Return result
        json_response
            .result
            .context("MCP server response missing result field")
    }

    async fn send_jsonrpc_request(&self, request: &JsonRpcRequest) -> Result<reqwest::Response> {
        let profile = self.auth_profile.lock().await.clone();

        let mut req = self
            .client
            .post(&self.server_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(profile) = profile {
            req = Self::apply_profile_auth(req, &profile);
        }

        req.json(request).send().await.map_err(Into::into)
    }

    async fn is_oauth_profile(&self) -> bool {
        self.auth_profile
            .lock()
            .await
            .as_ref()
            .map(|profile| profile.auth_type == AuthType::OAuth)
            .unwrap_or(false)
    }

    async fn maybe_refresh_oauth_token(&self) -> Result<()> {
        let should_refresh = {
            let guard = self.auth_profile.lock().await;
            if let Some(profile) = guard.as_ref() {
                if profile.auth_type == AuthType::OAuth {
                    if let Some(oauth_profile) = &profile.oauth {
                        oauth::should_refresh_token(oauth_profile, 60)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_refresh {
            self.force_refresh_oauth_token().await?;
        }

        Ok(())
    }

    async fn force_refresh_oauth_token(&self) -> Result<()> {
        let _refresh_guard = self.oauth_refresh_lock.lock().await;
        let mut profile = self.auth_profile.lock().await.clone().ok_or_else(|| {
            UxcError::OAuthRequired("No authentication profile available".to_string())
        })?;

        if profile.auth_type != AuthType::OAuth {
            return Ok(());
        }

        oauth::refresh_oauth_profile(&mut profile, &self.client).await?;
        self.persist_profile_update(&profile).await?;
        *self.auth_profile.lock().await = Some(profile);

        Ok(())
    }

    async fn persist_profile_update(&self, profile: &Profile) -> Result<()> {
        let Some(profile_name) = profile.name.clone() else {
            return Ok(());
        };

        let mut stored = profile.clone();
        stored.name = None;

        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut profiles = Profiles::load_profiles()?;
            profiles.set_profile(profile_name, stored)?;
            profiles.save_profiles()?;
            Ok(())
        })
        .await
        .context("Failed to persist refreshed OAuth profile")??;
        Ok(())
    }

    fn apply_profile_auth(
        req: reqwest::RequestBuilder,
        profile: &Profile,
    ) -> reqwest::RequestBuilder {
        match profile.auth_type {
            AuthType::OAuth => {
                if let Some(token) = profile.bearer_token() {
                    crate::auth::apply_auth_to_request(req, &profile.auth_type, token)
                } else {
                    req
                }
            }
            _ => crate::auth::apply_auth_to_request(req, &profile.auth_type, &profile.api_key),
        }
    }

    fn map_http_error(
        status: reqwest::StatusCode,
        body: &str,
        www_authenticate: Option<&str>,
    ) -> Result<JsonValue> {
        // Only treat 401 as OAuth-required when the server explicitly advertises
        // OAuth-related metadata in the WWW-Authenticate header. Otherwise, fall
        // back to a generic HTTP/auth failure to avoid misleading users of
        // non-OAuth authentication schemes.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(header) = www_authenticate {
                if let Some(resource_metadata) =
                    oauth::parse_resource_metadata_from_www_authenticate(header)
                {
                    let next_step = format!(
                        "OAuth required. Login with: uxc auth oauth login <profile> --endpoint <mcp_url> --client-id <id> (resource_metadata: {})",
                        resource_metadata
                    );
                    return Err(UxcError::OAuthRequired(next_step).into());
                }
            }
        }

        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(UxcError::OAuthScopeInsufficient(format!(
                "MCP server returned HTTP error: {} - {}",
                status, body
            ))
            .into());
        }

        bail!("MCP server returned HTTP error: {} - {}", status, body)
    }

    fn parse_jsonrpc_response(content_type: Option<&str>, body: &str) -> Result<JsonRpcResponse> {
        let content_type = content_type.unwrap_or_default().to_ascii_lowercase();

        if content_type.contains("text/event-stream") {
            return Self::parse_sse_response(body);
        }

        serde_json::from_str::<JsonRpcResponse>(body)
            .or_else(|_| Self::parse_sse_response(body))
            .context("Response is neither JSON-RPC JSON nor JSON-RPC SSE")
    }

    fn parse_sse_response(body: &str) -> Result<JsonRpcResponse> {
        for line in body.lines() {
            let trimmed = line.trim();
            if let Some(data) = trimmed.strip_prefix("data:") {
                let payload = data.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }

                if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(payload) {
                    return Ok(response);
                }
            }
        }

        bail!("No JSON-RPC payload found in SSE response")
    }

    fn summarize_body(body: &str) -> String {
        const MAX_CHARS: usize = 240;
        let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() <= MAX_CHARS {
            compact
        } else {
            let truncated: String = compact.chars().take(MAX_CHARS).collect();
            format!("{}...", truncated)
        }
    }

    /// Lightweight MCP HTTP probe used for endpoint discovery.
    pub async fn probe_initialize_with_reason(
        url: &str,
        auth_profile: Option<Profile>,
    ) -> Result<ProbeInitializeOutcome> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(PROBE_TIMEOUT_SECS))
            .build()
            .context("Failed to create MCP probe HTTP client")?;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "uxc-probe",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            id: RequestId::Number(1),
        };

        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(profile) = &auth_profile {
            req = Self::apply_profile_auth(req, profile);
        }

        let response = match req.json(&request).send().await {
            Ok(response) => response,
            Err(err) => {
                return Ok(ProbeInitializeOutcome::NotMcp(format!(
                    "request failed: {}",
                    err
                )));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Ok(ProbeInitializeOutcome::NotMcp(format!(
                "HTTP {}: {}",
                status,
                Self::summarize_body(&body)
            )));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = match response.text().await {
            Ok(body) => body,
            Err(err) => {
                return Ok(ProbeInitializeOutcome::NotMcp(format!(
                    "failed to read response body: {}",
                    err
                )));
            }
        };

        let response = match Self::parse_jsonrpc_response(content_type.as_deref(), &body) {
            Ok(response) => response,
            Err(err) => {
                return Ok(ProbeInitializeOutcome::NotMcp(format!(
                    "invalid JSON-RPC payload: {}",
                    err
                )));
            }
        };

        if let Some(error) = response.error {
            return Ok(ProbeInitializeOutcome::NotMcp(format!(
                "JSON-RPC error {}: {}",
                error.code, error.message
            )));
        }

        let Some(result) = response.result else {
            return Ok(ProbeInitializeOutcome::NotMcp(
                "missing JSON-RPC result field".to_string(),
            ));
        };

        match serde_json::from_value::<InitializeResult>(result) {
            Ok(_) => Ok(ProbeInitializeOutcome::Success),
            Err(err) => Ok(ProbeInitializeOutcome::NotMcp(format!(
                "invalid initialize result: {}",
                err
            ))),
        }
    }

    /// Lightweight MCP HTTP probe used for endpoint discovery.
    pub async fn probe_initialize(url: &str, auth_profile: Option<Profile>) -> Result<bool> {
        Ok(matches!(
            Self::probe_initialize_with_reason(url, auth_profile).await?,
            ProbeInitializeOutcome::Success
        ))
    }

    /// Initialize the MCP session
    pub async fn initialize(&self) -> Result<InitializeResult> {
        tracing::info!("Initializing MCP HTTP session with {}", self.server_url);

        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": {
                    "listChanged": true
                }
            },
            "clientInfo": {
                "name": "uxc",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let result = self.send_request("initialize", Some(params)).await?;

        serde_json::from_value(result).context("Failed to parse initialize result")
    }

    /// List available tools
    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        let result = self.send_request("tools/list", None).await?;

        let response: ToolsListResponse =
            serde_json::from_value(result).context("Failed to parse tools/list response")?;

        Ok(response.tools)
    }

    /// Call a tool
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonValue>,
    ) -> Result<ToolCallResult> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result = self.send_request("tools/call", Some(params)).await?;

        serde_json::from_value(result).context("Failed to parse tools/call result")
    }

    /// List available resources
    pub async fn list_resources(&self) -> Result<Vec<Resource>> {
        let result = self.send_request("resources/list", None).await?;

        let response: ResourcesListResponse =
            serde_json::from_value(result).context("Failed to parse resources/list response")?;

        Ok(response.resources)
    }

    /// Read a resource
    pub async fn read_resource(&self, uri: &str) -> Result<ResourceContents> {
        let params = serde_json::json!({
            "uri": uri
        });

        let result = self.send_request("resources/read", Some(params)).await?;

        serde_json::from_value(result).context("Failed to parse resources/read result")
    }

    /// List available prompts
    pub async fn list_prompts(&self) -> Result<Vec<Prompt>> {
        let result = self.send_request("prompts/list", None).await?;

        let response: PromptsListResponse =
            serde_json::from_value(result).context("Failed to parse prompts/list response")?;

        Ok(response.prompts)
    }

    /// Get a prompt
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<JsonValue>,
    ) -> Result<GetPromptResult> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result = self.send_request("prompts/get", Some(params)).await?;

        serde_json::from_value(result).context("Failed to parse prompts/get result")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthType, OAuthFlow, OAuthProfile, Profile, Profiles};
    use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    fn home_env_lock() -> &'static StdMutex<()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
    }

    struct TestEnv {
        _home_guard: MutexGuard<'static, ()>,
        _temp_dir: TempDir,
        previous_home: Option<std::ffi::OsString>,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            match &self.previous_home {
                Some(prev) => std::env::set_var("HOME", prev),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    fn setup_test_env() -> TestEnv {
        let guard = home_env_lock()
            .lock()
            .expect("Failed to lock HOME env guard");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        TestEnv {
            _home_guard: guard,
            _temp_dir: temp_dir,
            previous_home,
        }
    }

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs() as i64
    }

    // ===== URL Validation Tests =====

    #[test]
    fn new_with_valid_http_url_succeeds() {
        let transport = McpHttpTransport::new("http://localhost:3000/mcp".to_string());
        assert!(transport.is_ok());
    }

    #[test]
    fn new_with_valid_https_url_succeeds() {
        let transport = McpHttpTransport::new("https://example.com/mcp".to_string());
        assert!(transport.is_ok());
    }

    #[test]
    fn new_with_invalid_url_fails() {
        let transport = McpHttpTransport::new("not-a-url".to_string());
        assert!(transport.is_err());
        let err_msg = transport.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid MCP server URL"));
    }

    #[test]
    fn new_with_unsupported_scheme_fails() {
        let transport = McpHttpTransport::new("ftp://example.com/mcp".to_string());
        assert!(transport.is_err());
        let err_msg = transport.unwrap_err().to_string();
        assert!(err_msg.contains("only supports http:// and https://"));
    }

    #[test]
    fn new_with_file_scheme_fails() {
        let transport = McpHttpTransport::new("file:///path/to/file".to_string());
        assert!(transport.is_err());
        let err_msg = transport.unwrap_err().to_string();
        assert!(err_msg.contains("only supports http:// and https://"));
    }

    #[test]
    fn new_with_ws_scheme_fails() {
        let transport = McpHttpTransport::new("ws://localhost:3000/mcp".to_string());
        assert!(transport.is_err());
        let err_msg = transport.unwrap_err().to_string();
        assert!(err_msg.contains("only supports http:// and https://"));
    }

    #[test]
    fn with_auth_succeeds() {
        let profile = Profile::new("test-key".to_string(), AuthType::Bearer);
        let transport =
            McpHttpTransport::with_auth("https://example.com/mcp".to_string(), Some(profile));
        assert!(transport.is_ok());
    }

    #[test]
    fn with_auth_none_succeeds() {
        let transport = McpHttpTransport::with_auth("https://example.com/mcp".to_string(), None);
        assert!(transport.is_ok());
    }

    // ===== SSE Parsing Tests =====

    #[test]
    fn parse_sse_jsonrpc_response() {
        let sse = r#"event: message
data: {"jsonrpc":"2.0","id":1,"result":{"tools":[]}}

"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_some());
    }

    #[test]
    fn parse_sse_with_multiple_events_returns_first_valid() {
        let sse = r#"data: invalid
data: {"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}
data: {"jsonrpc":"2.0","id":2,"result":{"other":"data"}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, RequestId::Number(1));
    }

    #[test]
    fn parse_sse_with_empty_data_lines_skips_them() {
        let sse = r#"data:

data:
data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_sse_with_done_marker_skips_it() {
        let sse = r#"data: [DONE]
data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_sse_with_whitespace_in_data_strips_it() {
        let sse = r#"data:  {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_sse_with_error_response() {
        let sse = r#"data: {"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn parse_sse_with_no_valid_data_fails() {
        let sse = r#"data: [DONE]
data: invalid json
"#;

        let result = McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No JSON-RPC payload found"));
    }

    #[test]
    fn parse_sse_case_insensitive_content_type() {
        let sse = r#"data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("TEXT/EVENT-STREAM"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_sse_with_mixed_case_content_type() {
        let sse = r#"data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("Text/Event-Stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    // ===== JSON Response Parsing Tests =====

    #[test]
    fn parse_json_response_with_content_type() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("application/json"), json).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_some());
    }

    #[test]
    fn parse_json_response_without_content_type() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#;

        let response = McpHttpTransport::parse_jsonrpc_response(None, json).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_json_response_falls_back_to_sse() {
        // If JSON parsing fails, should try SSE
        let sse = r#"data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("application/json"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_invalid_json_response_fails() {
        let invalid = "not json at all";

        let result = McpHttpTransport::parse_jsonrpc_response(Some("application/json"), invalid);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("neither JSON-RPC JSON nor JSON-RPC SSE"));
    }

    #[test]
    fn parse_json_with_error_field() {
        let json = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32700,"message":"Parse error"}}"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("application/json"), json).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.error.is_some());
        assert_eq!(response.error.as_ref().unwrap().code, -32700);
    }

    #[test]
    fn parse_json_without_result_or_error() {
        let json = r#"{"jsonrpc":"2.0","id":1}"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("application/json"), json).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_none());
        assert!(response.error.is_none());
    }

    // ===== Request ID Tests =====

    #[tokio::test]
    async fn request_id_increments_with_each_request() {
        let mut server = mockito::Server::new_async().await;

        let mock1 = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let mock2 = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#)
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        transport.send_request("test1", None).await.unwrap();
        transport.send_request("test2", None).await.unwrap();

        mock1.assert_async().await;
        mock2.assert_async().await;
    }

    #[tokio::test]
    async fn request_id_starts_at_1() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        transport.send_request("test", None).await.unwrap();
    }

    // ===== Error Handling Tests =====

    #[tokio::test]
    async fn http_error_status_returns_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("HTTP error"));
        assert!(err_msg.contains("500"));
    }

    #[tokio::test]
    async fn http_404_status_returns_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(404)
            .with_body("Not Found")
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("404"));
    }

    #[tokio::test]
    async fn http_401_without_oauth_signal_returns_generic_http_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_body("Unauthorized")
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("HTTP error"));
        assert!(!err_msg.contains("OAuth required"));
    }

    #[tokio::test]
    async fn jsonrpc_error_field_returns_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("unknown_method", None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Method not found"));
        assert!(err_msg.contains("-32601"));
    }

    #[tokio::test]
    async fn missing_result_field_returns_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1}"#)
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing result field"));
    }

    #[tokio::test]
    async fn invalid_response_body_returns_error() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("invalid json{{{")
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn network_failure_returns_error() {
        // Use an invalid URL to simulate network failure
        let transport =
            McpHttpTransport::new("http://localhost:59999/nonexistent".to_string()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("HTTP")
                || err.contains("http")
                || err.contains("request")
                || err.contains("connect"),
            "unexpected error message: {err}"
        );
    }

    // ===== Initialize Tests =====

    #[tokio::test]
    async fn initialize_with_valid_response_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "protocolVersion":"2024-11-05",
                    "capabilities":{
                        "tools":{}
                    },
                    "serverInfo":{
                        "name":"test-server",
                        "version":"1.0.0"
                    }
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.initialize().await;
        assert!(result.is_ok());
        let init_result = result.unwrap();
        assert_eq!(init_result.protocolVersion, "2024-11-05");
        assert_eq!(init_result.serverInfo.unwrap().name, "test-server");
    }

    #[tokio::test]
    async fn initialize_with_sse_response_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(r#"data: {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test-server","version":"1.0.0"}}}
"#)
            .create_async().await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.initialize().await;
        assert!(result.is_ok());
        let init_result = result.unwrap();
        assert_eq!(init_result.protocolVersion, "2024-11-05");
    }

    #[tokio::test]
    async fn initialize_with_error_response_fails() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "error":{
                    "code":-32600,
                    "message":"Invalid Request"
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.initialize().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn initialize_with_invalid_result_fails() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "invalid":"data"
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.initialize().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse initialize result"));
    }

    // ===== Tool Listing Tests =====

    #[tokio::test]
    async fn list_tools_with_empty_list_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "tools":[]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.list_tools().await;
        assert!(result.is_ok());
        let tools = result.unwrap();
        assert_eq!(tools.len(), 0);
    }

    #[tokio::test]
    async fn list_tools_with_multiple_tools_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "tools":[
                        {
                            "name":"tool1",
                            "description":"First tool",
                            "inputSchema":{"type":"object"}
                        },
                        {
                            "name":"tool2",
                            "description":"Second tool",
                            "inputSchema":{"type":"object"}
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.list_tools().await;
        assert!(result.is_ok());
        let tools = result.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "tool1");
        assert_eq!(tools[1].name, "tool2");
    }

    #[tokio::test]
    async fn list_tools_with_sse_response_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(r#"data: {"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"tool1","description":"Tool 1"}]}}
"#)
            .create_async().await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.list_tools().await;
        assert!(result.is_ok());
        let tools = result.unwrap();
        assert_eq!(tools.len(), 1);
    }

    // ===== Tool Call Tests =====

    #[tokio::test]
    async fn call_tool_with_no_arguments_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "content":[
                        {
                            "type":"text",
                            "text":"Tool result"
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.call_tool("test_tool", None).await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert_eq!(tool_result.content.len(), 1);
    }

    #[tokio::test]
    async fn call_tool_with_arguments_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "content":[
                        {
                            "type":"text",
                            "text":"Success"
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let args = serde_json::json!({"param1": "value1"});
        let result = transport.call_tool("test_tool", Some(args)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn call_tool_with_error_response_fails() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "error":{
                    "code":-32602,
                    "message":"Invalid params"
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.call_tool("test_tool", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid params"));
    }

    #[tokio::test]
    async fn call_tool_returns_error_flag() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "content":[],
                    "isError":true
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.call_tool("test_tool", None).await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert_eq!(tool_result.isError, Some(true));
    }

    // ===== Resource Tests =====

    #[tokio::test]
    async fn list_resources_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "resources":[
                        {
                            "uri":"file:///test.txt",
                            "name":"test",
                            "description":"Test resource"
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.list_resources().await;
        assert!(result.is_ok());
        let resources = result.unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "test");
    }

    #[tokio::test]
    async fn read_resource_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "uri":"file:///test.txt",
                    "text":"Resource content"
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.read_resource("file:///test.txt").await;
        assert!(result.is_ok());
        let resource = result.unwrap();
        assert_eq!(resource.text, Some("Resource content".to_string()));
    }

    // ===== Prompt Tests =====

    #[tokio::test]
    async fn list_prompts_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "prompts":[
                        {
                            "name":"prompt1",
                            "description":"Test prompt"
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.list_prompts().await;
        assert!(result.is_ok());
        let prompts = result.unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "prompt1");
    }

    #[tokio::test]
    async fn get_prompt_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "description":"Prompt description",
                    "messages":[
                        {
                            "role":"user",
                            "content":"Hello"
                        }
                    ]
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.get_prompt("prompt1", None).await;
        assert!(result.is_ok());
        let prompt_result = result.unwrap();
        assert_eq!(prompt_result.messages.len(), 1);
    }

    // ===== Probe Tests =====

    #[tokio::test]
    async fn probe_initialize_with_valid_server_returns_true() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "protocolVersion":"2024-11-05",
                    "capabilities":{},
                    "serverInfo":{
                        "name":"test",
                        "version":"1.0"
                    }
                }
            }"#,
            )
            .create_async()
            .await;

        let result = McpHttpTransport::probe_initialize(&server.url(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[tokio::test]
    async fn probe_initialize_with_invalid_response_returns_false() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "invalid":"data"
                }
            }"#,
            )
            .create_async()
            .await;

        let result = McpHttpTransport::probe_initialize(&server.url(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn probe_initialize_with_network_error_returns_false() {
        let result =
            McpHttpTransport::probe_initialize("http://localhost:59999/nonexistent", None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn probe_initialize_with_http_error_returns_false() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let result = McpHttpTransport::probe_initialize(&server.url(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn probe_initialize_with_jsonrpc_error_returns_false() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "error":{
                    "code":-32600,
                    "message":"Invalid Request"
                }
            }"#,
            )
            .create_async()
            .await;

        let result = McpHttpTransport::probe_initialize(&server.url(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn probe_initialize_with_sse_response_returns_true() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(r#"data: {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1.0"}}}
"#)
            .create_async().await;

        let result = McpHttpTransport::probe_initialize(&server.url(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    // ===== Authentication Tests =====

    #[tokio::test]
    async fn send_request_with_bearer_auth_includes_header() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                mockito::Matcher::Regex("Bearer .*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let profile = Profile::new("test-token".to_string(), AuthType::Bearer);
        let transport = McpHttpTransport::with_auth(server.url(), Some(profile)).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_request_with_api_key_includes_header() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_header("x-api-key", mockito::Matcher::Exact("test-key".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let profile = Profile::new("test-key".to_string(), AuthType::ApiKey);
        let transport = McpHttpTransport::with_auth(server.url(), Some(profile)).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn oauth_request_refreshes_before_expiry_and_uses_new_token() {
        let mut server = mockito::Server::new_async().await;
        let token_endpoint = format!("{}/token", server.url());

        let _refresh_mock = server
            .mock("POST", "/token")
            .match_body(mockito::Matcher::Regex(
                "grant_type=refresh_token".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "access_token":"refreshed-token",
                    "token_type":"Bearer",
                    "expires_in":3600,
                    "refresh_token":"refresh-2"
                }"#,
            )
            .create_async()
            .await;

        let _request_mock = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer refreshed-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let mut profile = Profile::new(String::new(), AuthType::OAuth);
        profile.oauth = Some(OAuthProfile {
            token_endpoint: Some(token_endpoint),
            refresh_token: Some("refresh-1".to_string()),
            access_token: Some("stale-token".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(now_unix() - 10),
            oauth_flow: Some(OAuthFlow::DeviceCode),
            ..Default::default()
        });

        let transport = McpHttpTransport::with_auth(server.url(), Some(profile)).unwrap();
        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn oauth_401_refresh_retry_updates_profile_persistence() {
        let _env = setup_test_env();
        let mut server = mockito::Server::new_async().await;
        let token_endpoint = format!("{}/token", server.url());

        let mut profiles = Profiles::new();
        let mut persisted = Profile::new(String::new(), AuthType::OAuth);
        persisted.oauth = Some(OAuthProfile {
            token_endpoint: Some(token_endpoint.clone()),
            refresh_token: Some("refresh-1".to_string()),
            access_token: Some("old-token".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(now_unix() + 600),
            oauth_flow: Some(OAuthFlow::DeviceCode),
            ..Default::default()
        });
        profiles
            .set_profile("oauth".to_string(), persisted.clone())
            .unwrap();
        profiles.save_profiles().unwrap();

        let _first_request = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer old-token")
            .with_status(401)
            .with_header(
                "www-authenticate",
                r#"Bearer resource_metadata="https://example.com/.well-known/oauth-protected-resource""#,
            )
            .with_body("Unauthorized")
            .create_async()
            .await;

        let _refresh_mock = server
            .mock("POST", "/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "access_token":"new-token",
                    "token_type":"Bearer",
                    "expires_in":3600,
                    "refresh_token":"refresh-2"
                }"#,
            )
            .create_async()
            .await;

        let _retry_request = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer new-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let mut runtime_profile = persisted;
        runtime_profile.name = Some("oauth".to_string());
        let transport = McpHttpTransport::with_auth(server.url(), Some(runtime_profile)).unwrap();
        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());

        let loaded = Profiles::load_profiles().unwrap();
        let updated = loaded.get_profile("oauth").unwrap();
        assert_eq!(
            updated
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.access_token.clone())
                .as_deref(),
            Some("new-token")
        );
    }

    #[tokio::test]
    async fn probe_with_bearer_auth_includes_header() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                mockito::Matcher::Regex("Bearer .*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "protocolVersion":"2024-11-05",
                    "capabilities":{},
                    "serverInfo":{"name":"test","version":"1.0"}
                }
            }"#,
            )
            .create_async()
            .await;

        let profile = Profile::new("test-token".to_string(), AuthType::Bearer);
        let result = McpHttpTransport::probe_initialize(&server.url(), Some(profile)).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    // ===== Content Type Tests =====

    #[test]
    fn parse_response_with_charset_in_content_type() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("application/json; charset=utf-8"), json)
                .unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[test]
    fn parse_sse_with_charset_in_content_type() {
        let sse = r#"data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream; charset=utf-8"), sse)
                .unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    // ===== Edge Cases =====

    #[test]
    fn parse_sse_with_only_done_markers_fails() {
        let sse = r#"data: [DONE]
data: [DONE]
"#;

        let result = McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse);
        assert!(result.is_err());
    }

    #[test]
    fn parse_sse_with_malformed_json_skips_to_next() {
        let sse = r#"data: invalid json
data: {"jsonrpc":"2.0","id":1,"result":{}}
"#;

        let response =
            McpHttpTransport::parse_jsonrpc_response(Some("text/event-stream"), sse).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
    }

    #[tokio::test]
    async fn send_request_with_empty_params_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#)
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_request_with_complex_params_succeeds() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#)
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let params = serde_json::json!({
            "nested": {
                "array": [1, 2, 3],
                "string": "test"
            }
        });
        let result = transport.send_request("test", Some(params)).await;
        assert!(result.is_ok());
    }

    // ===== OAuth Tests =====

    #[tokio::test]
    async fn send_request_with_oauth_uses_bearer_token() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                mockito::Matcher::Exact("Bearer oauth-access-token".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let oauth_profile = crate::auth::OAuthProfile {
            access_token: Some("oauth-access-token".to_string()),
            ..Default::default()
        };
        let profile =
            Profile::new("stale-api-key".to_string(), AuthType::OAuth).with_oauth(oauth_profile);
        let transport = McpHttpTransport::with_auth(server.url(), Some(profile)).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_request_with_oauth_missing_token_skips_auth() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let oauth_profile = crate::auth::OAuthProfile {
            access_token: None,
            ..Default::default()
        };
        let profile =
            Profile::new("stale-api-key".to_string(), AuthType::OAuth).with_oauth(oauth_profile);
        let transport = McpHttpTransport::with_auth(server.url(), Some(profile)).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn map_http_error_401_with_resource_metadata_emits_oauth_required() {
        let err = McpHttpTransport::map_http_error(
            reqwest::StatusCode::UNAUTHORIZED,
            "Access denied",
            Some("Bearer resource_metadata=\"https://example.com/metadata\""),
        );

        assert!(err.is_err());
        let err_msg = err.unwrap_err().to_string();
        assert!(err_msg.contains("OAuth required"));
        assert!(err_msg.contains("resource_metadata"));
    }

    #[tokio::test]
    async fn map_http_error_401_without_resource_metadata_falls_through() {
        let err = McpHttpTransport::map_http_error(
            reqwest::StatusCode::UNAUTHORIZED,
            "Invalid token",
            Some("Bearer realm=\"api\""),
        );

        assert!(err.is_err());
        let err_msg = err.unwrap_err().to_string();
        assert!(!err_msg.contains("OAuth required"));
        assert!(err_msg.contains("HTTP error"));
        assert!(err_msg.contains("401"));
    }

    #[tokio::test]
    async fn map_http_error_401_without_www_authenticate_falls_through() {
        let err = McpHttpTransport::map_http_error(
            reqwest::StatusCode::UNAUTHORIZED,
            "Invalid credentials",
            None,
        );

        assert!(err.is_err());
        let err_msg = err.unwrap_err().to_string();
        assert!(!err_msg.contains("OAuth required"));
        assert!(err_msg.contains("HTTP error"));
        assert!(err_msg.contains("401"));
    }
}
