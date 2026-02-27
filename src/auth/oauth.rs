use crate::auth::{OAuthFlow, OAuthProfile, Profile};
use crate::error::UxcError;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use getrandom::getrandom;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Debug, Clone)]
pub struct OAuthProviderMetadata {
    pub provider_issuer: Option<String>,
    pub resource_metadata_url: Option<String>,
    pub authorization_server: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub registration_endpoint: Option<String>,
    pub token_endpoint: String,
    pub device_authorization_endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthLoginResult {
    pub metadata: OAuthProviderMetadata,
    pub token: OAuthTokenResponse,
}

#[derive(Debug, Clone)]
pub struct AuthorizationCodeLoginResult {
    pub login: OAuthLoginResult,
    pub client_id: String,
    pub client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

enum TokenEndpointResponse {
    Token(OAuthTokenResponse),
    Error(OAuthErrorResponse),
}

#[derive(Debug, Deserialize)]
struct ResourceMetadataDocument {
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    authorization_server: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenIdConfiguration {
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    authorization_endpoint: Option<String>,
    #[serde(default)]
    registration_endpoint: Option<String>,
    token_endpoint: String,
    #[serde(default)]
    device_authorization_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthorizationServerMetadata {
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    authorization_endpoint: Option<String>,
    #[serde(default)]
    registration_endpoint: Option<String>,
    token_endpoint: String,
    #[serde(default)]
    device_authorization_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorizationResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

pub fn should_refresh_token(oauth: &OAuthProfile, skew_seconds: i64) -> bool {
    match oauth.expires_at {
        Some(exp) => now_unix() + skew_seconds >= exp,
        None => false,
    }
}

pub fn apply_token_to_profile(
    profile: &mut Profile,
    flow: OAuthFlow,
    metadata: OAuthProviderMetadata,
    token: OAuthTokenResponse,
    client_id: Option<String>,
    client_secret: Option<String>,
    scopes: Vec<String>,
) {
    let expires_at = token.expires_in.map(|seconds| now_unix() + seconds);
    let scope_values = if let Some(scope) = token.scope {
        scope
            .split_whitespace()
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    } else {
        scopes
    };

    let access_token = token.access_token;
    profile.auth_type = crate::auth::AuthType::OAuth;
    profile.api_key = access_token.clone();
    profile.oauth = Some(OAuthProfile {
        provider_issuer: metadata.provider_issuer,
        resource_metadata_url: metadata.resource_metadata_url,
        authorization_server: metadata.authorization_server,
        token_endpoint: Some(metadata.token_endpoint),
        device_authorization_endpoint: metadata.device_authorization_endpoint,
        client_id,
        client_secret,
        access_token: Some(access_token),
        refresh_token: token.refresh_token,
        token_type: token.token_type,
        scopes: scope_values,
        expires_at,
        oauth_flow: Some(flow),
    });
}

pub async fn discover_provider_metadata(
    endpoint: &str,
    client: &Client,
) -> Result<OAuthProviderMetadata> {
    let resource_metadata_url = discover_resource_metadata_url(endpoint, client).await?;

    let mut authorization_server = if let Some(resource_url) = &resource_metadata_url {
        let resource_doc = client
            .get(resource_url)
            .send()
            .await
            .context("Failed to fetch resource metadata")?
            .error_for_status()
            .context("Resource metadata request failed")?
            .json::<ResourceMetadataDocument>()
            .await
            .context("Failed to decode resource metadata")?;

        resource_doc
            .authorization_server
            .or_else(|| resource_doc.authorization_servers.first().cloned())
    } else {
        None
    };

    if authorization_server.is_none() {
        authorization_server = endpoint_origin(endpoint);
    }

    let issuer = authorization_server.clone().ok_or_else(|| {
        UxcError::OAuthDiscoveryFailed(
            "Could not determine OAuth authorization server from MCP endpoint".to_string(),
        )
    })?;

    let authorization_server_metadata = fetch_authorization_server_metadata(&issuer, client)
        .await
        .ok();
    let openid = fetch_openid_configuration(&issuer, client).await.ok();

    let token_endpoint = authorization_server_metadata
        .as_ref()
        .map(|meta| meta.token_endpoint.clone())
        .or_else(|| openid.as_ref().map(|config| config.token_endpoint.clone()))
        .ok_or_else(|| {
            UxcError::OAuthDiscoveryFailed(
                "Could not determine token_endpoint from provider metadata".to_string(),
            )
        })?;

    let authorization_endpoint = authorization_server_metadata
        .as_ref()
        .and_then(|meta| meta.authorization_endpoint.clone())
        .or_else(|| {
            openid
                .as_ref()
                .and_then(|config| config.authorization_endpoint.clone())
        });
    let registration_endpoint = authorization_server_metadata
        .as_ref()
        .and_then(|meta| meta.registration_endpoint.clone())
        .or_else(|| {
            openid
                .as_ref()
                .and_then(|config| config.registration_endpoint.clone())
        });

    let provider_issuer = authorization_server_metadata
        .as_ref()
        .and_then(|meta| meta.issuer.clone())
        .or_else(|| openid.as_ref().and_then(|config| config.issuer.clone()))
        .or(Some(issuer.clone()));
    let device_authorization_endpoint =
        infer_device_authorization_endpoint(&issuer, authorization_server_metadata.as_ref())
            .or_else(|| {
                openid
                    .as_ref()
                    .and_then(|config| config.device_authorization_endpoint.clone())
            });

    Ok(OAuthProviderMetadata {
        provider_issuer,
        resource_metadata_url,
        authorization_server: Some(issuer),
        authorization_endpoint,
        registration_endpoint,
        token_endpoint,
        device_authorization_endpoint,
    })
}

pub async fn login_with_authorization_code(
    endpoint: &str,
    client: &Client,
    client_id: Option<&str>,
    client_secret: Option<&str>,
    scopes: &[String],
    redirect_uri: &str,
    authorization_code: Option<String>,
) -> Result<AuthorizationCodeLoginResult> {
    let metadata = discover_provider_metadata(endpoint, client).await?;
    let authorization_endpoint = metadata.authorization_endpoint.clone().ok_or_else(|| {
        UxcError::OAuthDiscoveryFailed(
            "OAuth provider does not expose authorization_endpoint".to_string(),
        )
    })?;
    let (resolved_client_id, resolved_client_secret) = match client_id {
        Some(id) => (id.to_string(), client_secret.map(|s| s.to_string())),
        None => dynamic_client_registration(&metadata, client, redirect_uri, scopes).await?,
    };

    let state = random_urlsafe(24)?;
    let code_verifier = random_urlsafe(64)?;
    let code_challenge = pkce_s256_code_challenge(&code_verifier);
    let scope_value = if scopes.is_empty() {
        None
    } else {
        Some(scopes.join(" "))
    };
    let authorize_url = build_authorize_url(
        &authorization_endpoint,
        &resolved_client_id,
        redirect_uri,
        scope_value.as_deref(),
        &state,
        &code_challenge,
    )?;

    eprintln!("Open this URL to authorize:");
    eprintln!("{}", authorize_url);
    eprintln!();

    let input = authorization_code
        .or_else(read_authorization_code_from_stdin)
        .ok_or_else(|| {
            UxcError::OAuthTokenExchangeFailed(
                "Authorization code is required to continue".to_string(),
            )
        })?;
    let (code, returned_state) = parse_authorization_code_input(&input).ok_or_else(|| {
        UxcError::OAuthTokenExchangeFailed(
            "Could not parse authorization code from input".to_string(),
        )
    })?;

    if let Some(returned_state) = returned_state {
        if returned_state != state {
            return Err(UxcError::OAuthTokenExchangeFailed(
                "OAuth state mismatch from authorization response".to_string(),
            )
            .into());
        }
    }

    let mut form: HashMap<&str, String> = HashMap::new();
    form.insert("grant_type", "authorization_code".to_string());
    form.insert("code", code);
    form.insert("redirect_uri", redirect_uri.to_string());
    form.insert("client_id", resolved_client_id.clone());
    form.insert("code_verifier", code_verifier);
    if let Some(secret) = resolved_client_secret.clone() {
        form.insert("client_secret", secret);
    }

    let token = exchange_token(client, &metadata.token_endpoint, &form)
        .await
        .map_err(|err| UxcError::OAuthTokenExchangeFailed(err.to_string()))?;

    Ok(AuthorizationCodeLoginResult {
        login: OAuthLoginResult { metadata, token },
        client_id: resolved_client_id,
        client_secret: resolved_client_secret,
    })
}

#[derive(Debug, serde::Serialize)]
struct DynamicClientRegistrationRequest {
    client_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_uri: Option<String>,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DynamicClientRegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

async fn dynamic_client_registration(
    metadata: &OAuthProviderMetadata,
    client: &Client,
    redirect_uri: &str,
    scopes: &[String],
) -> Result<(String, Option<String>)> {
    let registration_endpoint = metadata.registration_endpoint.clone().ok_or_else(|| {
        UxcError::OAuthDiscoveryFailed(
            "OAuth provider does not expose registration_endpoint and --client-id was not provided"
                .to_string(),
        )
    })?;

    let request = DynamicClientRegistrationRequest {
        client_name: "uxc".to_string(),
        client_uri: Some(env!("CARGO_PKG_HOMEPAGE").to_string()),
        redirect_uris: vec![redirect_uri.to_string()],
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        response_types: vec!["code".to_string()],
        token_endpoint_auth_method: "none".to_string(),
        scope: if scopes.is_empty() {
            None
        } else {
            Some(scopes.join(" "))
        },
    };

    let response = client
        .post(&registration_endpoint)
        .header("Accept", "application/json")
        .json(&request)
        .send()
        .await
        .with_context(|| {
            format!(
                "Failed to call dynamic client registration endpoint: {}",
                registration_endpoint
            )
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read dynamic client registration response body")?;

    if !status.is_success() {
        return Err(UxcError::OAuthTokenExchangeFailed(format!(
            "Dynamic client registration failed: {} {}",
            status, body
        ))
        .into());
    }

    let result: DynamicClientRegistrationResponse = serde_json::from_str(&body).map_err(|err| {
        UxcError::OAuthTokenExchangeFailed(format!(
            "Failed to decode dynamic client registration response: {}",
            err
        ))
    })?;

    Ok((result.client_id, result.client_secret))
}

pub async fn login_with_client_credentials(
    endpoint: &str,
    client: &Client,
    client_id: &str,
    client_secret: &str,
    scopes: &[String],
) -> Result<OAuthLoginResult> {
    let metadata = discover_provider_metadata(endpoint, client).await?;

    let mut form: HashMap<&str, String> = HashMap::new();
    form.insert("grant_type", "client_credentials".to_string());
    form.insert("client_id", client_id.to_string());
    form.insert("client_secret", client_secret.to_string());
    if !scopes.is_empty() {
        form.insert("scope", scopes.join(" "));
    }

    let token = exchange_token(client, &metadata.token_endpoint, &form)
        .await
        .map_err(|err| UxcError::OAuthTokenExchangeFailed(err.to_string()))?;

    Ok(OAuthLoginResult { metadata, token })
}

pub async fn login_with_device_code(
    endpoint: &str,
    client: &Client,
    client_id: &str,
    scopes: &[String],
) -> Result<OAuthLoginResult> {
    let metadata = discover_provider_metadata(endpoint, client).await?;
    let device_endpoint = metadata
        .device_authorization_endpoint
        .clone()
        .ok_or_else(|| {
            UxcError::OAuthDiscoveryFailed(
                "OAuth provider does not expose device_authorization_endpoint".to_string(),
            )
        })?;

    let mut form: HashMap<&str, String> = HashMap::new();
    form.insert("client_id", client_id.to_string());
    if !scopes.is_empty() {
        form.insert("scope", scopes.join(" "));
    }

    let device = client
        .post(&device_endpoint)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .context("Failed to request OAuth device code")?
        .error_for_status()
        .context("OAuth device code request failed")?
        .json::<DeviceAuthorizationResponse>()
        .await
        .context("Failed to decode OAuth device code response")?;

    eprintln!("Open this URL to authorize: {}", device.verification_uri);
    eprintln!("User code: {}", device.user_code);
    if let Some(url) = &device.verification_uri_complete {
        eprintln!("Direct verification URL: {}", url);
    }

    let mut poll_interval = device.interval.unwrap_or(5);
    let deadline = now_unix() + device.expires_in.unwrap_or(600) as i64;

    let mut token_form: HashMap<&str, String> = HashMap::new();
    token_form.insert("grant_type", DEVICE_CODE_GRANT.to_string());
    token_form.insert("device_code", device.device_code.clone());
    token_form.insert("client_id", client_id.to_string());

    loop {
        if now_unix() > deadline {
            return Err(UxcError::OAuthTokenExchangeFailed(
                "Device authorization timed out".to_string(),
            )
            .into());
        }

        let response = client
            .post(&metadata.token_endpoint)
            .header("Accept", "application/json")
            .form(&token_form)
            .send()
            .await
            .context("Failed to poll OAuth token endpoint")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read OAuth token response body")?;

        match parse_token_endpoint_response(status, &body)? {
            TokenEndpointResponse::Token(token) => return Ok(OAuthLoginResult { metadata, token }),
            TokenEndpointResponse::Error(err) => {
                if err.error == "authorization_pending" {
                    tokio::time::sleep(Duration::from_secs(poll_interval)).await;
                    continue;
                }

                if err.error == "slow_down" {
                    // RFC 8628: increase polling interval when instructed by server.
                    poll_interval = poll_interval.saturating_add(5);
                    tokio::time::sleep(Duration::from_secs(poll_interval)).await;
                    continue;
                }

                return Err(UxcError::OAuthTokenExchangeFailed(format_oauth_error(err)).into());
            }
        }
    }
}

pub async fn refresh_oauth_profile(profile: &mut Profile, client: &Client) -> Result<()> {
    let oauth = profile
        .oauth
        .as_mut()
        .ok_or_else(|| UxcError::OAuthRefreshFailed("OAuth profile data is missing".to_string()))?;

    let token_endpoint = resolve_token_endpoint(oauth, client).await?;

    if let Some(refresh_token) = oauth.refresh_token.clone() {
        let mut form: HashMap<&str, String> = HashMap::new();
        form.insert("grant_type", "refresh_token".to_string());
        form.insert("refresh_token", refresh_token);
        if let Some(client_id) = oauth.client_id.clone() {
            form.insert("client_id", client_id);
        }
        if let Some(client_secret) = oauth.client_secret.clone() {
            form.insert("client_secret", client_secret);
        }

        let token = exchange_token(client, &token_endpoint, &form)
            .await
            .map_err(|err| UxcError::OAuthRefreshFailed(err.to_string()))?;
        update_oauth_tokens(oauth, token);
        profile.api_key = oauth.access_token.clone().unwrap_or_default();
        return Ok(());
    }

    if oauth.oauth_flow == Some(OAuthFlow::ClientCredentials) {
        let client_id = oauth.client_id.clone().ok_or_else(|| {
            UxcError::OAuthRefreshFailed(
                "Missing client_id for client_credentials flow".to_string(),
            )
        })?;
        let client_secret = oauth.client_secret.clone().ok_or_else(|| {
            UxcError::OAuthRefreshFailed(
                "Missing client_secret for client_credentials flow".to_string(),
            )
        })?;

        let mut form: HashMap<&str, String> = HashMap::new();
        form.insert("grant_type", "client_credentials".to_string());
        form.insert("client_id", client_id);
        form.insert("client_secret", client_secret);
        if !oauth.scopes.is_empty() {
            form.insert("scope", oauth.scopes.join(" "));
        }

        let token = exchange_token(client, &token_endpoint, &form)
            .await
            .map_err(|err| UxcError::OAuthRefreshFailed(err.to_string()))?;
        update_oauth_tokens(oauth, token);
        profile.api_key = oauth.access_token.clone().unwrap_or_default();
        return Ok(());
    }

    Err(UxcError::OAuthRequired(
        "No refresh token available. Run 'uxc auth oauth login <credential_id> --endpoint <mcp_url>'"
            .to_string(),
    )
    .into())
}

pub fn parse_scopes(scopes: &[String]) -> Vec<String> {
    scopes
        .iter()
        .flat_map(|scope| scope.split_whitespace())
        .filter(|scope| !scope.is_empty())
        .map(|scope| scope.to_string())
        .collect()
}

pub fn parse_resource_metadata_from_www_authenticate(header: &str) -> Option<String> {
    parse_parameter_value(header, "resource_metadata")
}

async fn discover_resource_metadata_url(endpoint: &str, client: &Client) -> Result<Option<String>> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "uxc", "version": env!("CARGO_PKG_VERSION") }
        }
    });

    let response = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&request)
        .send()
        .await
        .context("Failed to call MCP endpoint for OAuth discovery")?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let header = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok());
        if let Some(metadata_url) = header.and_then(parse_resource_metadata_from_www_authenticate) {
            return Ok(Some(metadata_url));
        }
    }

    discover_resource_metadata_url_from_well_known(endpoint, client).await
}

async fn discover_resource_metadata_url_from_well_known(
    endpoint: &str,
    client: &Client,
) -> Result<Option<String>> {
    let Some(origin) = endpoint_origin(endpoint) else {
        return Ok(None);
    };
    let candidate = format!("{}/.well-known/oauth-protected-resource", origin);
    let response = client
        .get(&candidate)
        .header("Accept", "application/json")
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => Ok(Some(candidate)),
        Ok(_) => Ok(None),
        Err(_) => Ok(None),
    }
}

async fn resolve_token_endpoint(oauth: &mut OAuthProfile, client: &Client) -> Result<String> {
    if let Some(token_endpoint) = oauth.token_endpoint.clone() {
        return Ok(token_endpoint);
    }

    let issuer = oauth
        .provider_issuer
        .clone()
        .or_else(|| oauth.authorization_server.clone())
        .ok_or_else(|| {
            UxcError::OAuthRefreshFailed(
                "Missing provider_issuer/authorization_server for refresh".to_string(),
            )
        })?;

    let authorization_server_metadata = fetch_authorization_server_metadata(&issuer, client).await;
    let openid = fetch_openid_configuration(&issuer, client).await;

    let token_endpoint = authorization_server_metadata
        .as_ref()
        .ok()
        .map(|meta| meta.token_endpoint.clone())
        .or_else(|| {
            openid
                .as_ref()
                .ok()
                .map(|config| config.token_endpoint.clone())
        })
        .ok_or_else(|| {
            UxcError::OAuthRefreshFailed(
                "Could not determine token endpoint from provider metadata".to_string(),
            )
        })?;

    oauth.token_endpoint = Some(token_endpoint.clone());
    if oauth.device_authorization_endpoint.is_none() {
        oauth.device_authorization_endpoint = infer_device_authorization_endpoint(
            &issuer,
            authorization_server_metadata.as_ref().ok(),
        )
        .or_else(|| {
            openid
                .as_ref()
                .ok()
                .and_then(|config| config.device_authorization_endpoint.clone())
        });
    }
    if oauth.provider_issuer.is_none() {
        oauth.provider_issuer = authorization_server_metadata
            .as_ref()
            .ok()
            .and_then(|meta| meta.issuer.clone())
            .or_else(|| {
                openid
                    .as_ref()
                    .ok()
                    .and_then(|config| config.issuer.clone())
            })
            .or(Some(issuer));
    }

    Ok(token_endpoint)
}

async fn fetch_openid_configuration(issuer: &str, client: &Client) -> Result<OpenIdConfiguration> {
    let candidates = metadata_candidates(issuer, ".well-known/openid-configuration")?;
    let mut last_error: Option<anyhow::Error> = None;

    for url in candidates {
        let response = client
            .get(url.clone())
            .header("Accept", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                return resp
                    .json::<OpenIdConfiguration>()
                    .await
                    .context("Failed to decode OAuth OpenID configuration")
                    .map_err(|err| UxcError::OAuthDiscoveryFailed(err.to_string()).into());
            }
            Ok(resp) => {
                last_error = Some(anyhow!(
                    "OAuth OpenID configuration request failed at {}: {}",
                    url,
                    resp.status()
                ));
            }
            Err(err) => {
                last_error = Some(err.into());
            }
        }
    }

    Err(UxcError::OAuthDiscoveryFailed(
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Failed to fetch OAuth OpenID configuration".to_string()),
    )
    .into())
}

async fn fetch_authorization_server_metadata(
    issuer: &str,
    client: &Client,
) -> Result<AuthorizationServerMetadata> {
    let candidates = metadata_candidates(issuer, ".well-known/oauth-authorization-server")?;
    let mut last_error: Option<anyhow::Error> = None;

    for url in candidates {
        let response = client
            .get(url.clone())
            .header("Accept", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                return resp
                    .json::<AuthorizationServerMetadata>()
                    .await
                    .context("Failed to decode OAuth authorization server metadata")
                    .map_err(|err| UxcError::OAuthDiscoveryFailed(err.to_string()).into());
            }
            Ok(resp) => {
                last_error = Some(anyhow!(
                    "OAuth authorization server metadata request failed at {}: {}",
                    url,
                    resp.status()
                ));
            }
            Err(err) => {
                last_error = Some(err.into());
            }
        }
    }

    Err(UxcError::OAuthDiscoveryFailed(
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Failed to fetch OAuth authorization server metadata".to_string()),
    )
    .into())
}

fn metadata_candidates(issuer: &str, well_known: &str) -> Result<Vec<String>> {
    let issuer_url = reqwest::Url::parse(issuer.trim_end_matches('/'))
        .with_context(|| format!("Invalid issuer URL: {}", issuer))?;
    let mut candidates = Vec::new();

    let mut root = issuer_url.clone();
    root.set_path("/");
    root.set_query(None);
    root.set_fragment(None);
    let mut path_aware = format!("{}/{}", root.as_str().trim_end_matches('/'), well_known);
    let issuer_path = issuer_url.path().trim_start_matches('/');
    if !issuer_path.is_empty() {
        path_aware.push('/');
        path_aware.push_str(issuer_path);
    }
    candidates.push(path_aware);

    let mut legacy = issuer_url;
    let mut legacy_path = legacy.path().trim_end_matches('/').to_string();
    if legacy_path.is_empty() {
        legacy_path = "/".to_string();
    }
    legacy_path.push_str(&format!("/{}", well_known));
    legacy.set_path(&legacy_path);
    legacy.set_query(None);
    legacy.set_fragment(None);
    let legacy_str = legacy.to_string();
    if !candidates.contains(&legacy_str) {
        candidates.push(legacy_str);
    }

    Ok(candidates)
}

fn infer_device_authorization_endpoint(
    issuer: &str,
    metadata: Option<&AuthorizationServerMetadata>,
) -> Option<String> {
    if let Some(endpoint) = metadata.and_then(|m| m.device_authorization_endpoint.clone()) {
        return Some(endpoint);
    }

    if issuer.trim_end_matches('/') == "https://github.com/login/oauth" {
        return Some("https://github.com/login/device/code".to_string());
    }

    None
}

fn endpoint_origin(endpoint: &str) -> Option<String> {
    let url = Url::parse(endpoint).ok()?;
    let scheme = url.scheme();
    let host = url.host_str()?;
    let mut origin = format!("{}://{}", scheme, host);
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Some(origin)
}

fn random_urlsafe(bytes_len: usize) -> Result<String> {
    let mut bytes = vec![0u8; bytes_len];
    getrandom(&mut bytes).context("Failed to generate random bytes for OAuth")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn pkce_s256_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn build_authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scope: Option<&str>,
    state: &str,
    code_challenge: &str,
) -> Result<String> {
    let mut url = Url::parse(authorization_endpoint).with_context(|| {
        format!(
            "Invalid OAuth authorization endpoint: {}",
            authorization_endpoint
        )
    })?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("state", state);
        query.append_pair("code_challenge", code_challenge);
        query.append_pair("code_challenge_method", "S256");
        if let Some(scope) = scope {
            query.append_pair("scope", scope);
        }
    }
    Ok(url.into())
}

fn read_authorization_code_from_stdin() -> Option<String> {
    eprint!("Paste authorization code or callback URL: ");
    if io::stderr().flush().is_err() {
        return None;
    }
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf).is_err() {
        return None;
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Extract code and state parameters from a URL's query string.
/// Returns Some((code, state)) if code is found, None otherwise.
fn extract_code_and_state_from_query(url: &Url) -> Option<(String, Option<String>)> {
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for (k, v) in url.query_pairs() {
        if k == "code" {
            code = Some(v.to_string());
        } else if k == "state" {
            state = Some(v.to_string());
        }
    }
    code.map(|c| (c, state))
}

fn parse_authorization_code_input(input: &str) -> Option<(String, Option<String>)> {
    if let Some((_, query)) = input.split_once('?') {
        // Try parsing as a full URL first
        if let Ok(url) = Url::parse(input) {
            return extract_code_and_state_from_query(&url);
        }
        // Fallback: parse as query string only
        let url_like = format!("https://placeholder.local/?{}", query);
        if let Ok(url) = Url::parse(&url_like) {
            return extract_code_and_state_from_query(&url);
        }
    }

    // Plain code input (no query string)
    Some((input.trim().to_string(), None))
}

async fn exchange_token(
    client: &Client,
    token_endpoint: &str,
    form: &HashMap<&str, String>,
) -> Result<OAuthTokenResponse> {
    let response = client
        .post(token_endpoint)
        .header("Accept", "application/json")
        .form(form)
        .send()
        .await
        .with_context(|| format!("Failed to call token endpoint: {}", token_endpoint))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("Failed to read OAuth token response body")?;

    match parse_token_endpoint_response(status, &body)? {
        TokenEndpointResponse::Token(token) => Ok(token),
        TokenEndpointResponse::Error(err) => Err(anyhow!(format_oauth_error(err))),
    }
}

fn parse_token_endpoint_response(
    status: reqwest::StatusCode,
    body: &str,
) -> Result<TokenEndpointResponse> {
    if status.is_success() {
        if let Ok(token) = serde_json::from_str::<OAuthTokenResponse>(body) {
            return Ok(TokenEndpointResponse::Token(token));
        }
        if let Ok(err) = serde_json::from_str::<OAuthErrorResponse>(body) {
            return Ok(TokenEndpointResponse::Error(err));
        }
        return Err(anyhow!("Failed to decode OAuth token response"));
    }

    if let Ok(err) = serde_json::from_str::<OAuthErrorResponse>(body) {
        return Ok(TokenEndpointResponse::Error(err));
    }

    Ok(TokenEndpointResponse::Error(OAuthErrorResponse {
        error: status.as_str().to_string(),
        error_description: None,
    }))
}

fn update_oauth_tokens(oauth: &mut OAuthProfile, token: OAuthTokenResponse) {
    oauth.access_token = Some(token.access_token);
    oauth.token_type = token.token_type;
    if let Some(refresh_token) = token.refresh_token {
        oauth.refresh_token = Some(refresh_token);
    }
    if let Some(scope) = token.scope {
        oauth.scopes = scope
            .split_whitespace()
            .map(|value| value.to_string())
            .collect();
    }
    oauth.expires_at = token.expires_in.map(|seconds| now_unix() + seconds);
}

fn parse_parameter_value(header: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let start = header.find(&needle)? + needle.len();
    let remaining = &header[start..];
    let end = remaining.find('"')?;
    Some(remaining[..end].to_string())
}

fn format_oauth_error(err: OAuthErrorResponse) -> String {
    match err.error_description {
        Some(desc) if !desc.is_empty() => format!("{}: {}", err.error, desc),
        _ => err.error,
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resource_metadata_header() {
        let header = r#"Bearer realm="mcp", resource_metadata="https://api.example.com/.well-known/oauth-protected-resource""#;
        assert_eq!(
            parse_resource_metadata_from_www_authenticate(header).as_deref(),
            Some("https://api.example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn refresh_decision_works() {
        let oauth = OAuthProfile {
            expires_at: Some(now_unix() + 30),
            ..Default::default()
        };
        assert!(should_refresh_token(&oauth, 60));
    }

    #[test]
    fn metadata_candidates_support_path_issuer() {
        let candidates = metadata_candidates(
            "https://github.com/login/oauth",
            ".well-known/oauth-authorization-server",
        )
        .unwrap();
        assert_eq!(
            candidates[0],
            "https://github.com/.well-known/oauth-authorization-server/login/oauth"
        );
        assert_eq!(
            candidates[1],
            "https://github.com/login/oauth/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn infer_github_device_endpoint_when_missing_in_metadata() {
        let endpoint = infer_device_authorization_endpoint("https://github.com/login/oauth", None);
        assert_eq!(
            endpoint.as_deref(),
            Some("https://github.com/login/device/code")
        );
    }

    #[test]
    fn parse_authorization_code_input_supports_plain_code() {
        let parsed = parse_authorization_code_input("abc123").unwrap();
        assert_eq!(parsed.0, "abc123");
        assert!(parsed.1.is_none());
    }

    #[test]
    fn parse_authorization_code_input_supports_callback_url() {
        let parsed =
            parse_authorization_code_input("http://127.0.0.1/callback?code=abc123&state=xyz")
                .unwrap();
        assert_eq!(parsed.0, "abc123");
        assert_eq!(parsed.1.as_deref(), Some("xyz"));
    }

    #[test]
    fn parse_authorization_code_input_supports_query_string_only() {
        let parsed = parse_authorization_code_input("?code=abc123&state=xyz").unwrap();
        assert_eq!(parsed.0, "abc123");
        assert_eq!(parsed.1.as_deref(), Some("xyz"));
    }

    #[test]
    fn build_authorize_url_includes_pkce_params() {
        let url = build_authorize_url(
            "https://example.com/authorize",
            "client-1",
            "http://127.0.0.1/callback",
            Some("scope:a scope:b"),
            "state-1",
            "challenge-1",
        )
        .unwrap();
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-1"));
        assert!(url.contains("code_challenge=challenge-1"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn endpoint_origin_extracts_host_and_scheme() {
        let origin = endpoint_origin("https://mcp.notion.com/mcp").unwrap();
        assert_eq!(origin, "https://mcp.notion.com");
    }

    #[test]
    fn endpoint_origin_preserves_non_standard_port() {
        let origin = endpoint_origin("https://mcp.example.com:8443/mcp").unwrap();
        assert_eq!(origin, "https://mcp.example.com:8443");
    }

    #[test]
    fn dynamic_registration_request_uses_expected_defaults() {
        let req = DynamicClientRegistrationRequest {
            client_name: "uxc".to_string(),
            client_uri: Some("https://example.com".to_string()),
            redirect_uris: vec!["http://127.0.0.1/callback".to_string()],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "none".to_string(),
            scope: Some("read write".to_string()),
        };
        let json = serde_json::to_value(req).unwrap();
        assert_eq!(json["token_endpoint_auth_method"], "none");
        assert_eq!(json["grant_types"][0], "authorization_code");
        assert_eq!(json["grant_types"][1], "refresh_token");
    }

    #[test]
    fn parse_token_endpoint_response_supports_success_error_payload() {
        let parsed = parse_token_endpoint_response(
            reqwest::StatusCode::OK,
            r#"{"error":"authorization_pending","error_description":"pending"}"#,
        )
        .unwrap();
        match parsed {
            TokenEndpointResponse::Error(err) => {
                assert_eq!(err.error, "authorization_pending");
            }
            TokenEndpointResponse::Token(_) => panic!("expected OAuth error payload"),
        }
    }

    #[test]
    fn parse_token_endpoint_response_supports_success_token_payload() {
        let parsed = parse_token_endpoint_response(
            reqwest::StatusCode::OK,
            r#"{"access_token":"token","token_type":"bearer","expires_in":3600}"#,
        )
        .unwrap();
        match parsed {
            TokenEndpointResponse::Token(token) => {
                assert_eq!(token.access_token, "token");
                assert_eq!(token.token_type.as_deref(), Some("bearer"));
                assert_eq!(token.expires_in, Some(3600));
            }
            TokenEndpointResponse::Error(_) => panic!("expected OAuth token payload"),
        }
    }

    #[test]
    fn parse_token_endpoint_response_supports_error_status_error_payload() {
        let parsed = parse_token_endpoint_response(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_grant","error_description":"bad grant"}"#,
        )
        .unwrap();
        match parsed {
            TokenEndpointResponse::Error(err) => {
                assert_eq!(err.error, "invalid_grant");
                assert_eq!(err.error_description.as_deref(), Some("bad grant"));
            }
            TokenEndpointResponse::Token(_) => panic!("expected OAuth error payload"),
        }
    }

    #[test]
    fn parse_token_endpoint_response_falls_back_to_status_for_invalid_error_body() {
        let parsed =
            parse_token_endpoint_response(reqwest::StatusCode::UNAUTHORIZED, "{}").unwrap();
        match parsed {
            TokenEndpointResponse::Error(err) => {
                assert_eq!(err.error, "401");
                assert!(err.error_description.is_none());
            }
            TokenEndpointResponse::Token(_) => panic!("expected OAuth error payload"),
        }
    }
}
