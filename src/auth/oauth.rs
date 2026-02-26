use crate::auth::{OAuthFlow, OAuthProfile, Profile};
use crate::error::UxcError;
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Debug, Clone)]
pub struct OAuthProviderMetadata {
    pub provider_issuer: Option<String>,
    pub resource_metadata_url: Option<String>,
    pub authorization_server: Option<String>,
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

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
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

    let authorization_server = if let Some(resource_url) = &resource_metadata_url {
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

    let issuer = authorization_server.clone().ok_or_else(|| {
        UxcError::OAuthDiscoveryFailed(
            "Could not determine OAuth authorization server from MCP endpoint".to_string(),
        )
    })?;

    let openid = fetch_openid_configuration(&issuer, client).await?;

    Ok(OAuthProviderMetadata {
        provider_issuer: openid.issuer.or(Some(issuer.clone())),
        resource_metadata_url,
        authorization_server: Some(issuer),
        token_endpoint: openid.token_endpoint,
        device_authorization_endpoint: openid.device_authorization_endpoint,
    })
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
            .form(&token_form)
            .send()
            .await
            .context("Failed to poll OAuth token endpoint")?;

        if response.status().is_success() {
            let token = response
                .json::<OAuthTokenResponse>()
                .await
                .context("Failed to decode OAuth token response")?;
            return Ok(OAuthLoginResult { metadata, token });
        }

        let err = response
            .json::<OAuthErrorResponse>()
            .await
            .unwrap_or(OAuthErrorResponse {
                error: "unknown_error".to_string(),
                error_description: None,
            });

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
        "No refresh token available. Run 'uxc auth oauth login <profile> --endpoint <mcp_url>'"
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

    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(None);
    }

    let header = response
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok());

    Ok(header.and_then(parse_resource_metadata_from_www_authenticate))
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

    let config = fetch_openid_configuration(&issuer, client)
        .await
        .map_err(|err| UxcError::OAuthRefreshFailed(err.to_string()))?;

    oauth.token_endpoint = Some(config.token_endpoint.clone());
    if oauth.device_authorization_endpoint.is_none() {
        oauth.device_authorization_endpoint = config.device_authorization_endpoint;
    }
    if oauth.provider_issuer.is_none() {
        oauth.provider_issuer = config.issuer;
    }

    Ok(config.token_endpoint)
}

async fn fetch_openid_configuration(issuer: &str, client: &Client) -> Result<OpenIdConfiguration> {
    let issuer = issuer.trim_end_matches('/');
    let url = format!("{}/.well-known/openid-configuration", issuer);

    client
        .get(url)
        .send()
        .await
        .context("Failed to fetch OAuth OpenID configuration")?
        .error_for_status()
        .context("OAuth OpenID configuration request failed")?
        .json::<OpenIdConfiguration>()
        .await
        .context("Failed to decode OAuth OpenID configuration")
        .map_err(|err| UxcError::OAuthDiscoveryFailed(err.to_string()).into())
}

async fn exchange_token(
    client: &Client,
    token_endpoint: &str,
    form: &HashMap<&str, String>,
) -> Result<OAuthTokenResponse> {
    let response = client
        .post(token_endpoint)
        .form(form)
        .send()
        .await
        .with_context(|| format!("Failed to call token endpoint: {}", token_endpoint))?;

    if response.status().is_success() {
        return response
            .json::<OAuthTokenResponse>()
            .await
            .context("Failed to decode OAuth token response");
    }

    let status = response.status();
    let err = response
        .json::<OAuthErrorResponse>()
        .await
        .unwrap_or(OAuthErrorResponse {
            error: status.as_str().to_string(),
            error_description: None,
        });

    Err(anyhow!(format_oauth_error(err)))
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
}
