//! MCP HTTP transport for communicating with MCP servers over HTTP/HTTPS

#![allow(dead_code)]

use super::types::*;
use crate::auth::{oauth, AuthType, Profile, Profiles};
use crate::error::UxcError;
use crate::http_client::build_resilient_http_client;
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const PROBE_TIMEOUT_SECS: u64 = 10;
const MAX_STREAM_BODY_BYTES: usize = 1024 * 1024;
const LEGACY_SSE_STREAM_TIMEOUT_SECS: u64 = 60 * 60 * 24;
const LEGACY_SSE_RESPONSE_TIMEOUT_SECS: u64 = 30;

type PendingResponse = tokio::sync::oneshot::Sender<Result<JsonValue, String>>;
type PendingResponses = Arc<Mutex<HashMap<RequestId, PendingResponse>>>;

/// MCP HTTP transport mode selected during endpoint probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpHttpMode {
    StreamableHttp,
    LegacySse,
}

/// Resolved MCP HTTP endpoint plus the transport mode required to talk to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMcpHttpTransport {
    pub mode: McpHttpMode,
    pub connect_url: String,
}

impl ResolvedMcpHttpTransport {
    pub fn new(mode: McpHttpMode, connect_url: String) -> Self {
        Self { mode, connect_url }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeInitializeOutcome {
    Success(McpHttpMode),
    NotMcp(String),
    AuthFailed(ProbeAuthFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAuthFailure {
    pub code: ProbeAuthFailureCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeAuthFailureCode {
    OAuthRequired,
    OAuthRefreshFailed,
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
    /// MCP Streamable HTTP session id (if provided by server)
    session_id: Arc<Mutex<Option<String>>>,
    /// Authentication profile
    auth_profile: Arc<Mutex<Option<Profile>>>,
    /// Lock for OAuth refresh operations
    oauth_refresh_lock: Arc<Mutex<()>>,
}

#[derive(Debug)]
pub struct LegacySseTransport {
    client: Client,
    connect_url: String,
    next_id: Arc<Mutex<i64>>,
    auth_profile: Arc<Mutex<Option<Profile>>>,
    oauth_refresh_lock: Arc<Mutex<()>>,
    connect_lock: Arc<Mutex<()>>,
    session: Arc<Mutex<Option<LegacySseSession>>>,
}

#[derive(Debug)]
struct LegacySseSession {
    messages_url: String,
    pending: PendingResponses,
    reader_task: JoinHandle<()>,
}

#[derive(Debug)]
pub enum McpRemoteTransport {
    Streamable(McpHttpTransport),
    Legacy(LegacySseTransport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SseEvent {
    event_type: String,
    data: String,
}

async fn persist_profile_update(profile: &Profile) -> Result<()> {
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

        let client =
            build_resilient_http_client(std::time::Duration::from_secs(30), "MCP HTTP transport")?;

        Ok(Self {
            client,
            server_url: url,
            next_id: Arc::new(Mutex::new(1i64)),
            session_id: Arc::new(Mutex::new(None)),
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
        if let Some(session_id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
        {
            *self.session_id.lock().await = Some(session_id);
        }
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
        let body = Self::read_response_body(&mut response, content_type.as_deref())
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
        let session_id = self.session_id.lock().await.clone();
        let authed_url = Self::apply_profile_auth_to_url(&self.server_url, profile.as_ref())?;

        let mut req = self
            .client
            .post(&authed_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(session_id) = session_id {
            req = req.header("mcp-session-id", session_id);
        }

        if let Some(profile) = profile {
            req = Self::apply_profile_auth(req, &profile)?;
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
        persist_profile_update(&profile).await?;
        *self.auth_profile.lock().await = Some(profile);

        Ok(())
    }

    fn apply_profile_auth(
        req: reqwest::RequestBuilder,
        profile: &Profile,
    ) -> Result<reqwest::RequestBuilder> {
        match profile.auth_type {
            AuthType::OAuth => {
                if let Some(token) = profile.bearer_token() {
                    let mut token_profile = profile.clone();
                    token_profile.api_key = token.to_string();
                    crate::auth::apply_profile_auth_to_request(req, &token_profile)
                } else {
                    Ok(req)
                }
            }
            _ => crate::auth::apply_profile_auth_to_request(req, profile),
        }
    }

    fn apply_profile_auth_to_url(url: &str, profile: Option<&Profile>) -> Result<String> {
        match profile {
            Some(profile) => crate::auth::apply_profile_auth_to_url(url, profile),
            None => Ok(url.to_string()),
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
                        "OAuth required.\n\
                         \n\
                         For agents:\n\
                           1. uxc auth oauth start <credential_id> --endpoint <mcp_url> --redirect-uri <callback_uri>\n\
                           2. uxc auth oauth complete <credential_id> --session-id <session_id> --authorization-response '<callback_url_or_code>'\n\
                         \n\
                         Interactive fallback:\n\
                           uxc auth oauth login <credential_id> --endpoint <mcp_url> --client-id <client_id>\n\
                         \n\
                         (resource_metadata: {})",
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

    async fn read_response_body(
        response: &mut reqwest::Response,
        content_type: Option<&str>,
    ) -> Result<String> {
        let is_sse = content_type
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("text/event-stream");

        let mut body = String::new();
        loop {
            let chunk = response
                .chunk()
                .await
                .context("Failed to read SSE response body chunk")?;
            let Some(chunk) = chunk else {
                break;
            };

            body.push_str(&String::from_utf8_lossy(&chunk));
            if is_sse && Self::parse_sse_response(&body).is_ok() {
                break;
            }
            if body.len() >= MAX_STREAM_BODY_BYTES {
                break;
            }
        }

        Ok(body)
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
        let client = build_resilient_http_client(
            std::time::Duration::from_secs(PROBE_TIMEOUT_SECS),
            "MCP HTTP probe",
        )?;

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

        let mut auth_profile = auth_profile;

        match Self::probe_initialize_once(url, &client, &request, auth_profile.as_ref()).await {
            ProbeAttemptResult::Success(mode) => Ok(ProbeInitializeOutcome::Success(mode)),
            ProbeAttemptResult::LegacyBootstrapRequired(reason) => {
                Self::probe_legacy_sse_with_oauth_retry(url, &client, &mut auth_profile, reason)
                    .await
            }
            ProbeAttemptResult::Unauthorized(reason) => {
                let is_oauth = auth_profile
                    .as_ref()
                    .map(|profile| profile.auth_type == AuthType::OAuth)
                    .unwrap_or(false);

                if !is_oauth {
                    return Ok(ProbeInitializeOutcome::NotMcp(reason));
                }

                // Safe to expect: when `is_oauth` is true, `auth_profile` must be `Some`.
                let profile = auth_profile
                    .as_mut()
                    .expect("oauth probe path requires auth profile");

                if let Err(err) = oauth::refresh_oauth_profile(profile, &client).await {
                    return Ok(Self::probe_auth_failure_from_refresh_error(err));
                }

                if let Err(err) = persist_profile_update(profile).await {
                    return Ok(ProbeInitializeOutcome::AuthFailed(ProbeAuthFailure {
                        code: ProbeAuthFailureCode::OAuthRefreshFailed,
                        message: format!("Failed to persist refreshed OAuth profile: {}", err),
                    }));
                }

                match Self::probe_initialize_once(url, &client, &request, Some(profile)).await {
                    ProbeAttemptResult::Success(mode) => Ok(ProbeInitializeOutcome::Success(mode)),
                    ProbeAttemptResult::LegacyBootstrapRequired(reason) => {
                        Self::probe_legacy_sse_with_oauth_retry(
                            url,
                            &client,
                            &mut auth_profile,
                            reason,
                        )
                        .await
                    }
                    ProbeAttemptResult::Unauthorized(retry_reason) => {
                        Ok(ProbeInitializeOutcome::AuthFailed(ProbeAuthFailure {
                            code: ProbeAuthFailureCode::OAuthRequired,
                            message: format!(
                                "OAuth token rejected after refresh during MCP probe: {}",
                                retry_reason
                            ),
                        }))
                    }
                    ProbeAttemptResult::NotMcp(reason) => {
                        Ok(ProbeInitializeOutcome::NotMcp(reason))
                    }
                }
            }
            ProbeAttemptResult::NotMcp(reason) => Ok(ProbeInitializeOutcome::NotMcp(reason)),
        }
    }

    async fn probe_legacy_sse_with_oauth_retry(
        url: &str,
        client: &Client,
        auth_profile: &mut Option<Profile>,
        probe_reason: String,
    ) -> Result<ProbeInitializeOutcome> {
        match Self::probe_legacy_sse_once(url, client, auth_profile.as_ref()).await {
            ProbeAttemptResult::Success(mode) => Ok(ProbeInitializeOutcome::Success(mode)),
            ProbeAttemptResult::Unauthorized(reason) => {
                let is_oauth = auth_profile
                    .as_ref()
                    .map(|profile| profile.auth_type == AuthType::OAuth)
                    .unwrap_or(false);

                if !is_oauth {
                    return Ok(ProbeInitializeOutcome::NotMcp(format!(
                        "{}; {}",
                        probe_reason, reason
                    )));
                }

                let profile = auth_profile
                    .as_mut()
                    .expect("oauth probe path requires auth profile");

                if let Err(err) = oauth::refresh_oauth_profile(profile, client).await {
                    return Ok(Self::probe_auth_failure_from_refresh_error(err));
                }

                if let Err(err) = persist_profile_update(profile).await {
                    return Ok(ProbeInitializeOutcome::AuthFailed(ProbeAuthFailure {
                        code: ProbeAuthFailureCode::OAuthRefreshFailed,
                        message: format!("Failed to persist refreshed OAuth profile: {}", err),
                    }));
                }

                match Self::probe_legacy_sse_once(url, client, Some(profile)).await {
                    ProbeAttemptResult::Success(mode) => Ok(ProbeInitializeOutcome::Success(mode)),
                    ProbeAttemptResult::Unauthorized(retry_reason) => {
                        Ok(ProbeInitializeOutcome::AuthFailed(ProbeAuthFailure {
                            code: ProbeAuthFailureCode::OAuthRequired,
                            message: format!(
                                "OAuth token rejected after refresh during MCP probe: {}",
                                retry_reason
                            ),
                        }))
                    }
                    ProbeAttemptResult::LegacyBootstrapRequired(legacy_reason)
                    | ProbeAttemptResult::NotMcp(legacy_reason) => {
                        Ok(ProbeInitializeOutcome::NotMcp(format!(
                            "{}; {}",
                            probe_reason, legacy_reason
                        )))
                    }
                }
            }
            ProbeAttemptResult::LegacyBootstrapRequired(legacy_reason)
            | ProbeAttemptResult::NotMcp(legacy_reason) => Ok(ProbeInitializeOutcome::NotMcp(
                format!("{}; {}", probe_reason, legacy_reason),
            )),
        }
    }

    async fn probe_initialize_once(
        url: &str,
        client: &Client,
        request: &JsonRpcRequest,
        auth_profile: Option<&Profile>,
    ) -> ProbeAttemptResult {
        let mut req = client
            .post(match Self::apply_profile_auth_to_url(url, auth_profile) {
                Ok(url) => url,
                Err(err) => {
                    return ProbeAttemptResult::NotMcp(format!(
                        "failed to apply auth profile to url: {}",
                        err
                    ));
                }
            })
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(profile) = auth_profile {
            req = match Self::apply_profile_auth(req, profile) {
                Ok(req) => req,
                Err(err) => {
                    return ProbeAttemptResult::NotMcp(format!(
                        "failed to apply auth profile: {}",
                        err
                    ));
                }
            };
        }

        let mut response = match req.json(request).send().await {
            Ok(response) => response,
            Err(err) => {
                return ProbeAttemptResult::NotMcp(format!("request failed: {}", err));
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = response.text().await.unwrap_or_default();
            return ProbeAttemptResult::Unauthorized(format!(
                "HTTP {}: {}",
                status,
                Self::summarize_body(&body)
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let reason = format!("HTTP {}: {}", status, Self::summarize_body(&body));
            return if status.is_client_error() {
                ProbeAttemptResult::LegacyBootstrapRequired(reason)
            } else {
                ProbeAttemptResult::NotMcp(reason)
            };
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = match Self::read_response_body(&mut response, content_type.as_deref()).await {
            Ok(body) => body,
            Err(err) => {
                return ProbeAttemptResult::NotMcp(format!(
                    "failed to read response body: {}",
                    err
                ));
            }
        };

        let response = match Self::parse_jsonrpc_response(content_type.as_deref(), &body) {
            Ok(response) => response,
            Err(err) => {
                return ProbeAttemptResult::NotMcp(format!("invalid JSON-RPC payload: {}", err));
            }
        };

        if let Some(error) = response.error {
            return ProbeAttemptResult::NotMcp(format!(
                "JSON-RPC error {}: {}",
                error.code, error.message
            ));
        }

        let Some(result) = response.result else {
            return ProbeAttemptResult::NotMcp("missing JSON-RPC result field".to_string());
        };

        match serde_json::from_value::<InitializeResult>(result) {
            Ok(_) => ProbeAttemptResult::Success(McpHttpMode::StreamableHttp),
            Err(err) => ProbeAttemptResult::NotMcp(format!("invalid initialize result: {}", err)),
        }
    }

    async fn probe_legacy_sse_once(
        url: &str,
        client: &Client,
        auth_profile: Option<&Profile>,
    ) -> ProbeAttemptResult {
        let authed_url = match Self::apply_profile_auth_to_url(url, auth_profile) {
            Ok(url) => url,
            Err(err) => {
                return ProbeAttemptResult::NotMcp(format!(
                    "failed to apply auth profile to url: {}",
                    err
                ));
            }
        };
        let mut req = client
            .get(&authed_url)
            .header("Accept", "text/event-stream");

        if let Some(profile) = auth_profile {
            req = match Self::apply_profile_auth(req, profile) {
                Ok(req) => req,
                Err(err) => {
                    return ProbeAttemptResult::NotMcp(format!(
                        "failed to apply auth profile: {}",
                        err
                    ));
                }
            };
        }

        let mut response = match req.send().await {
            Ok(response) => response,
            Err(err) => return ProbeAttemptResult::NotMcp(format!("request failed: {}", err)),
        };

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = response.text().await.unwrap_or_default();
            return ProbeAttemptResult::Unauthorized(format!(
                "HTTP {}: {}",
                status,
                Self::summarize_body(&body)
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return ProbeAttemptResult::NotMcp(format!(
                "HTTP {}: {}",
                status,
                Self::summarize_body(&body)
            ));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.contains("text/event-stream") {
            return ProbeAttemptResult::NotMcp(format!(
                "unexpected content-type for legacy SSE bootstrap: {}",
                content_type
            ));
        }

        let mut buffer = String::new();
        while buffer.len() < MAX_STREAM_BODY_BYTES {
            match response.chunk().await {
                Ok(Some(chunk)) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
                Ok(None) => break,
                Err(err) => {
                    return ProbeAttemptResult::NotMcp(format!(
                        "failed to read legacy SSE bootstrap: {}",
                        err
                    ));
                }
            }

            match LegacySseTransport::drain_sse_events(&mut buffer) {
                Ok(events) => {
                    if events.iter().any(|event| event.event_type == "endpoint") {
                        return ProbeAttemptResult::Success(McpHttpMode::LegacySse);
                    }
                }
                Err(err) => {
                    return ProbeAttemptResult::NotMcp(format!(
                        "invalid legacy SSE event stream: {}",
                        err
                    ));
                }
            }
        }

        ProbeAttemptResult::NotMcp(
            "legacy SSE bootstrap stream did not emit endpoint event".to_string(),
        )
    }

    fn probe_auth_failure_from_refresh_error(err: anyhow::Error) -> ProbeInitializeOutcome {
        let code = err
            .chain()
            .find_map(|cause| cause.downcast_ref::<UxcError>())
            .map(|uxc_err| match uxc_err {
                UxcError::OAuthRequired(_) => ProbeAuthFailureCode::OAuthRequired,
                UxcError::OAuthRefreshFailed(_) => ProbeAuthFailureCode::OAuthRefreshFailed,
                _ => ProbeAuthFailureCode::OAuthRefreshFailed,
            })
            .unwrap_or(ProbeAuthFailureCode::OAuthRefreshFailed);

        ProbeInitializeOutcome::AuthFailed(ProbeAuthFailure {
            code,
            message: format!("OAuth refresh failed during MCP probe: {}", err),
        })
    }

    /// Lightweight MCP HTTP probe used for endpoint discovery.
    pub async fn probe_initialize(url: &str, auth_profile: Option<Profile>) -> Result<bool> {
        Ok(matches!(
            Self::probe_initialize_with_reason(url, auth_profile).await?,
            ProbeInitializeOutcome::Success(_)
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
        let params = match arguments {
            Some(arguments) => serde_json::json!({
                "name": name,
                "arguments": arguments
            }),
            None => serde_json::json!({
                "name": name,
                "arguments": {}
            }),
        };

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

impl LegacySseTransport {
    pub fn with_auth(url: String, auth_profile: Option<Profile>) -> Result<Self> {
        let parsed = url::Url::parse(&url).context("Invalid MCP server URL")?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            bail!(
                "Legacy MCP SSE transport only supports http:// and https:// URLs, got: {}",
                parsed.scheme()
            );
        }

        let client = build_resilient_http_client(
            std::time::Duration::from_secs(LEGACY_SSE_STREAM_TIMEOUT_SECS),
            "Legacy MCP SSE",
        )?;

        Ok(Self {
            client,
            connect_url: url,
            next_id: Arc::new(Mutex::new(1i64)),
            auth_profile: Arc::new(Mutex::new(auth_profile)),
            oauth_refresh_lock: Arc::new(Mutex::new(())),
            connect_lock: Arc::new(Mutex::new(())),
            session: Arc::new(Mutex::new(None)),
        })
    }

    async fn ensure_connected(&self) -> Result<()> {
        {
            let mut session_guard = self.session.lock().await;
            if let Some(session) = session_guard.as_ref() {
                if !session.reader_task.is_finished() {
                    return Ok(());
                }
                *session_guard = None;
            }
        }

        let _connect_guard = self.connect_lock.lock().await;
        {
            let mut session_guard = self.session.lock().await;
            if let Some(session) = session_guard.as_ref() {
                if !session.reader_task.is_finished() {
                    return Ok(());
                }
                *session_guard = None;
            }
        }

        self.maybe_refresh_oauth_token().await?;

        let mut response = self.send_bootstrap_request().await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED && self.is_oauth_profile().await {
            self.force_refresh_oauth_token().await?;
            response = self.send_bootstrap_request().await?;
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

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return McpHttpTransport::map_http_error(status, &body, www_authenticate.as_deref())
                .map(|_| ());
        }

        if !content_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("text/event-stream")
        {
            let body = response.text().await.unwrap_or_default();
            bail!(
                "Legacy SSE bootstrap did not return text/event-stream: {}",
                body
            );
        }

        let mut buffer = String::new();
        let mut messages_url = None;

        while messages_url.is_none() && buffer.len() < MAX_STREAM_BODY_BYTES {
            let chunk = response
                .chunk()
                .await
                .context("Failed to read legacy SSE bootstrap chunk")?;
            let Some(chunk) = chunk else {
                break;
            };

            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for event in Self::drain_sse_events(&mut buffer)? {
                if event.event_type == "endpoint" {
                    let resolved = url::Url::parse(&self.connect_url)
                        .context("Invalid legacy SSE bootstrap URL")?
                        .join(event.data.trim())
                        .context("Failed to resolve legacy SSE messages endpoint")?;
                    let origin = url::Url::parse(&self.connect_url)
                        .context("Invalid legacy SSE bootstrap URL")?
                        .origin()
                        .ascii_serialization();
                    if resolved.origin().ascii_serialization() != origin {
                        bail!("Legacy SSE endpoint origin does not match bootstrap URL");
                    }
                    messages_url = Some(resolved.to_string());
                    break;
                }
            }
        }

        let messages_url =
            messages_url.context("Legacy SSE bootstrap stream did not emit endpoint event")?;
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let reader_task = tokio::spawn(Self::run_sse_reader(response, buffer, pending.clone()));

        *self.session.lock().await = Some(LegacySseSession {
            messages_url,
            pending,
            reader_task,
        });

        Ok(())
    }

    async fn send_bootstrap_request(&self) -> Result<reqwest::Response> {
        let profile = self.auth_profile.lock().await.clone();
        let connect_url =
            McpHttpTransport::apply_profile_auth_to_url(&self.connect_url, profile.as_ref())?;
        let mut req = self
            .client
            .get(&connect_url)
            .header("Accept", "text/event-stream");

        if let Some(profile) = profile {
            req = McpHttpTransport::apply_profile_auth(req, &profile)?;
        }

        req.send().await.map_err(Into::into)
    }

    async fn send_request(&self, method: &str, params: Option<JsonValue>) -> Result<JsonValue> {
        self.ensure_connected().await?;
        self.maybe_refresh_oauth_token().await?;

        let id = {
            let mut next_id = self.next_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: RequestId::Number(id),
        };

        let (messages_url, pending) = {
            let session = self.session.lock().await;
            let session = session
                .as_ref()
                .context("Legacy SSE transport not connected after bootstrap")?;
            (session.messages_url.clone(), session.pending.clone())
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        pending.lock().await.insert(request.id.clone(), tx);

        let mut response = match self.send_messages_request(&messages_url, &request).await {
            Ok(response) => response,
            Err(err) => {
                pending.lock().await.remove(&request.id);
                return Err(err).context("Failed to send legacy SSE HTTP request");
            }
        };

        if response.status() == reqwest::StatusCode::UNAUTHORIZED && self.is_oauth_profile().await {
            self.force_refresh_oauth_token().await?;
            response = match self.send_messages_request(&messages_url, &request).await {
                Ok(response) => response,
                Err(err) => {
                    pending.lock().await.remove(&request.id);
                    return Err(err).context("Failed to send legacy SSE HTTP retry request");
                }
            };
        }

        let status = response.status();
        let www_authenticate = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            pending.lock().await.remove(&request.id);
            return McpHttpTransport::map_http_error(status, &body, www_authenticate.as_deref());
        }

        let result =
            match tokio::time::timeout(Duration::from_secs(LEGACY_SSE_RESPONSE_TIMEOUT_SECS), rx)
                .await
            {
                Ok(result) => result
                    .context("Legacy SSE reader closed before delivering a response")?
                    .map_err(anyhow::Error::msg)?,
                Err(_) => {
                    pending.lock().await.remove(&request.id);
                    bail!(
                        "Timed out waiting for legacy SSE response after {}s",
                        LEGACY_SSE_RESPONSE_TIMEOUT_SECS
                    );
                }
            };

        Ok(result)
    }

    async fn send_messages_request(
        &self,
        messages_url: &str,
        request: &JsonRpcRequest,
    ) -> Result<reqwest::Response> {
        let profile = self.auth_profile.lock().await.clone();
        let messages_url =
            McpHttpTransport::apply_profile_auth_to_url(messages_url, profile.as_ref())?;
        let mut req = self
            .client
            .post(&messages_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(profile) = profile {
            req = McpHttpTransport::apply_profile_auth(req, &profile)?;
        }

        req.json(request).send().await.map_err(Into::into)
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

    async fn is_oauth_profile(&self) -> bool {
        self.auth_profile
            .lock()
            .await
            .as_ref()
            .map(|profile| profile.auth_type == AuthType::OAuth)
            .unwrap_or(false)
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
        persist_profile_update(&profile).await?;
        *self.auth_profile.lock().await = Some(profile);
        Ok(())
    }

    async fn initialize(&self) -> Result<InitializeResult> {
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

    async fn list_tools(&self) -> Result<Vec<Tool>> {
        let result = self.send_request("tools/list", None).await?;
        let response: ToolsListResponse =
            serde_json::from_value(result).context("Failed to parse tools/list response")?;
        Ok(response.tools)
    }

    async fn call_tool(&self, name: &str, arguments: Option<JsonValue>) -> Result<ToolCallResult> {
        let params = match arguments {
            Some(arguments) => serde_json::json!({
                "name": name,
                "arguments": arguments
            }),
            None => serde_json::json!({
                "name": name,
                "arguments": {}
            }),
        };

        let result = self.send_request("tools/call", Some(params)).await?;
        serde_json::from_value(result).context("Failed to parse tools/call result")
    }

    async fn run_sse_reader(
        mut response: reqwest::Response,
        mut buffer: String,
        pending: PendingResponses,
    ) {
        loop {
            match Self::drain_sse_events(&mut buffer) {
                Ok(events) => {
                    for event in events {
                        if event.event_type != "message" {
                            continue;
                        }

                        match serde_json::from_str::<JsonRpcResponse>(&event.data) {
                            Ok(message) => {
                                let payload = if let Some(error) = message.error {
                                    Err(format!(
                                        "MCP server returned error: {} - {}",
                                        error.code, error.message
                                    ))
                                } else {
                                    message
                                        .result
                                        .context("MCP server response missing result field")
                                        .map_err(|err| err.to_string())
                                };

                                if let Some(sender) = pending.lock().await.remove(&message.id) {
                                    let _ = sender.send(payload);
                                }
                            }
                            Err(err) => {
                                tracing::debug!(
                                    "Ignoring malformed legacy SSE message event: {}",
                                    err
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    Self::fail_pending(
                        &pending,
                        format!("Failed to parse legacy SSE event stream: {}", err),
                    )
                    .await;
                    return;
                }
            }

            match response.chunk().await {
                Ok(Some(chunk)) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
                Ok(None) => {
                    Self::fail_pending(&pending, "Legacy SSE stream closed".to_string()).await;
                    return;
                }
                Err(err) => {
                    Self::fail_pending(
                        &pending,
                        format!("Failed to read legacy SSE stream chunk: {}", err),
                    )
                    .await;
                    return;
                }
            }
        }
    }

    async fn fail_pending(pending: &PendingResponses, message: String) {
        let senders = {
            let mut guard = pending.lock().await;
            std::mem::take(&mut *guard)
        };

        for (_, sender) in senders {
            let _ = sender.send(Err(message.clone()));
        }
    }

    // Minimal SSE parser for MCP-compatible event streams. It ignores directives and comments
    // we do not currently use and only extracts complete `event:` / `data:` blocks.
    fn drain_sse_events(buffer: &mut String) -> Result<Vec<SseEvent>> {
        let mut events = Vec::new();
        let mut consumed = 0usize;

        while let Some(delim_len) = Self::find_sse_event_delimiter(&buffer[consumed..]) {
            let event_block = &buffer[consumed..consumed + delim_len];
            consumed += delim_len;
            consumed += if buffer[consumed..].starts_with("\r\n\r\n") {
                4
            } else {
                2
            };

            let mut event_type = None;
            let mut data_lines = Vec::new();

            for line in event_block.lines() {
                let line = line.trim_end_matches('\r');
                if line.starts_with(':') || line.starts_with("retry:") || line.starts_with("id:") {
                    continue;
                } else if let Some(value) = line.strip_prefix("event:") {
                    event_type = Some(value.trim().to_string());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data_lines.push(value.trim_start().to_string());
                }
            }

            if !data_lines.is_empty() {
                events.push(SseEvent {
                    event_type: event_type.unwrap_or_else(|| "message".to_string()),
                    data: data_lines.join("\n"),
                });
            }
        }

        if consumed > 0 {
            buffer.drain(..consumed);
        }

        Ok(events)
    }

    fn find_sse_event_delimiter(input: &str) -> Option<usize> {
        input.find("\r\n\r\n").or_else(|| input.find("\n\n"))
    }
}

impl Drop for LegacySseTransport {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.session.try_lock() {
            if let Some(session) = guard.take() {
                session.reader_task.abort();
            }
        }
    }
}

impl McpRemoteTransport {
    pub fn with_auth(
        resolved: ResolvedMcpHttpTransport,
        auth_profile: Option<Profile>,
    ) -> Result<Self> {
        match resolved.mode {
            McpHttpMode::StreamableHttp => Ok(Self::Streamable(McpHttpTransport::with_auth(
                resolved.connect_url,
                auth_profile,
            )?)),
            McpHttpMode::LegacySse => Ok(Self::Legacy(LegacySseTransport::with_auth(
                resolved.connect_url,
                auth_profile,
            )?)),
        }
    }

    pub async fn initialize(&self) -> Result<InitializeResult> {
        match self {
            Self::Streamable(transport) => transport.initialize().await,
            Self::Legacy(transport) => transport.initialize().await,
        }
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        match self {
            Self::Streamable(transport) => transport.list_tools().await,
            Self::Legacy(transport) => transport.list_tools().await,
        }
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonValue>,
    ) -> Result<ToolCallResult> {
        match self {
            Self::Streamable(transport) => transport.call_tool(name, arguments).await,
            Self::Legacy(transport) => transport.call_tool(name, arguments).await,
        }
    }
}

enum ProbeAttemptResult {
    Success(McpHttpMode),
    LegacyBootstrapRequired(String),
    Unauthorized(String),
    NotMcp(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthType, OAuthFlow, OAuthProfile, Profile, Profiles};
    use hyper::service::{make_service_fn, service_fn};
    use hyper::{Body, Method, Request, Response, Server, StatusCode};
    use std::convert::Infallible;
    use std::net::TcpListener;
    use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;
    use tokio::sync::{mpsc, oneshot};
    use tokio_stream::wrappers::UnboundedReceiverStream;

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

    type LegacySseSender = mpsc::UnboundedSender<Result<hyper::body::Bytes, Infallible>>;

    #[derive(Clone, Default)]
    struct LegacySseTestState {
        sender: Arc<tokio::sync::Mutex<Option<LegacySseSender>>>,
    }

    struct LegacySseTestServer {
        base_url: String,
        shutdown: Option<oneshot::Sender<()>>,
        task: JoinHandle<()>,
    }

    impl Drop for LegacySseTestServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            self.task.abort();
        }
    }

    async fn spawn_legacy_sse_test_server() -> LegacySseTestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind legacy SSE test server");
        let addr = listener.local_addr().expect("legacy SSE test addr");
        let state = LegacySseTestState::default();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let make_svc = make_service_fn(move |_| {
            let state = state.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let state = state.clone();
                    async move { Ok::<_, Infallible>(legacy_sse_test_response(req, state).await) }
                }))
            }
        });

        let server = Server::from_tcp(listener)
            .expect("legacy SSE test server from tcp")
            .serve(make_svc)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });

        let task = tokio::spawn(async move {
            let _ = server.await;
        });

        LegacySseTestServer {
            base_url: format!("http://{}", addr),
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    async fn legacy_sse_test_response(
        req: Request<Body>,
        state: LegacySseTestState,
    ) -> Response<Body> {
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or_default().to_string();

        match (req.method(), path.as_str()) {
            (&Method::POST, "/sse") => Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header("allow", "GET,HEAD")
                .body(Body::empty())
                .expect("method not allowed response"),
            (&Method::GET, "/sse") => {
                let (tx, rx) = mpsc::unbounded_channel();
                *state.sender.lock().await = Some(tx.clone());
                tx.send(Ok(hyper::body::Bytes::from(
                    "event: endpoint\ndata: /messages?sessionId=test-session\n\n",
                )))
                .expect("bootstrap event");

                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/event-stream")
                    .body(Body::wrap_stream(UnboundedReceiverStream::new(rx)))
                    .expect("legacy sse response")
            }
            (&Method::POST, "/messages") if query == "sessionId=test-session" => {
                let body = hyper::body::to_bytes(req.into_body())
                    .await
                    .expect("legacy message body");
                let request: JsonRpcRequest =
                    serde_json::from_slice(&body).expect("legacy JSON-RPC request");

                let response_payload = match request.method.as_str() {
                    "initialize" => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": request.id,
                        "result": {
                            "protocolVersion":"2024-11-05",
                            "capabilities":{"tools":{"listChanged":false}},
                            "serverInfo":{"name":"legacy-test","version":"1.0"}
                        }
                    }),
                    "tools/list" => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": request.id,
                        "result": {
                            "tools": [{
                                "name":"legacy_tool",
                                "description":"Legacy test tool",
                                "inputSchema":{"type":"object"}
                            }]
                        }
                    }),
                    "tools/call" => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": request.id,
                        "result": {
                            "content": [{"type":"text","text":"legacy-ok"}]
                        }
                    }),
                    other => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": request.id,
                        "error": {"code": -32601, "message": format!("unknown method: {}", other)}
                    }),
                };

                let sender = state
                    .sender
                    .lock()
                    .await
                    .clone()
                    .expect("active SSE sender");
                sender
                    .send(Ok(hyper::body::Bytes::from(format!(
                        "event: message\ndata: {}\n\n",
                        response_payload
                    ))))
                    .expect("legacy message event");

                Response::builder()
                    .status(StatusCode::ACCEPTED)
                    .body(Body::empty())
                    .expect("legacy accepted response")
            }
            _ => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("not found response"),
        }
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

    #[tokio::test]
    async fn call_tool_parses_structured_content() {
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
                    "content":[{"type":"text","text":"ok"}],
                    "structuredContent":{"status":"ok","count":1}
                }
            }"#,
            )
            .create_async()
            .await;

        let transport = McpHttpTransport::new(server.url()).unwrap();

        let result = transport.call_tool("test_tool", None).await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert_eq!(
            tool_result.structuredContent,
            Some(serde_json::json!({"status":"ok","count":1}))
        );
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
        assert!(result.unwrap());
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
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn probe_initialize_with_network_error_returns_false() {
        let result =
            McpHttpTransport::probe_initialize("http://localhost:59999/nonexistent", None).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
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
        assert!(!result.unwrap());
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
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn probe_initialize_with_api_key_query_param() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_query(mockito::Matcher::UrlEncoded(
                "apiKey".into(),
                "test-key".into(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1.0.0"}}}"#,
            )
            .create_async()
            .await;

        let mut profile = Profile::new("test-key".to_string(), AuthType::ApiKey);
        profile.auth_query_params = Some(vec![crate::auth::AuthQueryParam::new(
            "apiKey",
            "{{secret}}",
        )
        .unwrap()]);

        let result = McpHttpTransport::probe_initialize(&server.url(), Some(profile)).await;

        assert!(result.is_ok());
        assert!(result.unwrap());
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
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn probe_initialize_with_legacy_sse_bootstrap_returns_legacy_mode() {
        let server = spawn_legacy_sse_test_server().await;

        let result = McpHttpTransport::probe_initialize_with_reason(
            &format!("{}/sse", server.base_url),
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            result,
            ProbeInitializeOutcome::Success(McpHttpMode::LegacySse)
        );
    }

    #[tokio::test]
    async fn legacy_sse_transport_supports_initialize_list_and_call() {
        let server = spawn_legacy_sse_test_server().await;
        let transport =
            LegacySseTransport::with_auth(format!("{}/sse", server.base_url), None).unwrap();

        let init = transport.initialize().await.unwrap();
        assert_eq!(
            init.serverInfo.as_ref().map(|info| info.name.as_str()),
            Some("legacy-test")
        );

        let tools = transport.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "legacy_tool");

        let result = transport.call_tool("legacy_tool", None).await.unwrap();
        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            ToolContent::Text { text } => assert_eq!(text, "legacy-ok"),
            other => panic!("expected text tool content, got {:?}", other),
        }
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
    async fn send_request_with_api_key_custom_header_template() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_header(
                "ok-access-key",
                mockito::Matcher::Exact("test-key".to_string()),
            )
            .match_header("x-client", mockito::Matcher::Exact("uxc".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let mut profile = Profile::new("test-key".to_string(), AuthType::ApiKey);
        profile.auth_headers = Some(vec![
            crate::auth::AuthHeader::new("ok-access-key", "{{secret}}").unwrap(),
            crate::auth::AuthHeader::new("x-client", "uxc").unwrap(),
        ]);
        let transport = McpHttpTransport::with_auth(server.url(), Some(profile)).unwrap();

        let result = transport.send_request("test", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_request_with_api_key_query_param() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("POST", "/")
            .match_query(mockito::Matcher::UrlEncoded(
                "apiKey".into(),
                "test-key".into(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
            .create_async()
            .await;

        let mut profile = Profile::new("test-key".to_string(), AuthType::ApiKey);
        profile.auth_query_params = Some(vec![crate::auth::AuthQueryParam::new(
            "apiKey",
            "{{secret}}",
        )
        .unwrap()]);
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
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn probe_initialize_oauth_401_refreshes_and_succeeds() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/mcp", server.url());
        let token_endpoint = format!("{}/token", server.url());

        let _first = server
            .mock("POST", "/mcp")
            .match_header("authorization", "Bearer old-token")
            .with_status(401)
            .with_body(r#"{"error":"invalid_token","error_description":"Invalid access token"}"#)
            .create_async()
            .await;
        let _refresh = server
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
        let _retry = server
            .mock("POST", "/mcp")
            .match_header("authorization", "Bearer new-token")
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

        let mut profile = Profile::new(String::new(), AuthType::OAuth);
        profile.oauth = Some(OAuthProfile {
            token_endpoint: Some(token_endpoint),
            refresh_token: Some("refresh-1".to_string()),
            access_token: Some("old-token".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(now_unix() + 600),
            oauth_flow: Some(OAuthFlow::AuthorizationCode),
            ..Default::default()
        });

        let result = McpHttpTransport::probe_initialize_with_reason(&endpoint, Some(profile)).await;
        assert!(matches!(
            result.unwrap(),
            ProbeInitializeOutcome::Success(McpHttpMode::StreamableHttp)
        ));
    }

    #[tokio::test]
    async fn probe_initialize_oauth_401_refresh_failed_returns_auth_failed() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/mcp", server.url());
        let token_endpoint = format!("{}/token", server.url());

        let _first = server
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
            refresh_token: Some("refresh-1".to_string()),
            access_token: Some("old-token".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(now_unix() + 600),
            oauth_flow: Some(OAuthFlow::AuthorizationCode),
            ..Default::default()
        });

        let result = McpHttpTransport::probe_initialize_with_reason(&endpoint, Some(profile)).await;
        match result.unwrap() {
            ProbeInitializeOutcome::AuthFailed(failure) => {
                assert_eq!(failure.code, ProbeAuthFailureCode::OAuthRefreshFailed);
                assert!(failure.message.contains("OAuth refresh failed"));
            }
            other => panic!("expected AuthFailed outcome, got {:?}", other),
        }
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

    #[test]
    fn drain_sse_events_ignores_comments_and_retry_fields() {
        let mut buffer = r#": keepalive
retry: 1000
event: endpoint
data: /messages?sessionId=test

"#
        .to_string();

        let events = LegacySseTransport::drain_sse_events(&mut buffer).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "endpoint");
        assert_eq!(events[0].data, "/messages?sessionId=test");
        assert!(buffer.is_empty());
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
