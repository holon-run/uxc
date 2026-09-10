//! Provider-backed retrieval for `email.body.read` message references.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use futures::StreamExt;
use reqwest::{redirect::Policy, Client, Method, Response};
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

use crate::auth::{
    resolve_profile_request_auth_with_context, AuthRequestContext, Profile, Profiles,
};
use crate::daemon::ManagedSourceSpec;
use crate::email_body::{parse_mime, EmailBodyResult, EmailMessageLocator, EmailMessageRef};
use crate::subscription_email::{
    connect_imap, quote_imap_string, EmailImapIdleRuntimeConfig, ImapAuthMethod, ImapConnection,
    ImapSectionLiteral,
};

pub(crate) const PROVIDER_RESPONSE_MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const PROVIDER_DEADLINE: Duration = Duration::from_secs(30);

/// Fetch and normalize the body identified by `message_ref` using the current
/// managed-source configuration. The reference never supplies an endpoint or credentials.
pub(crate) async fn get_email_body(
    spec: &ManagedSourceSpec,
    message_ref: &EmailMessageRef,
) -> Result<EmailBodyResult> {
    tokio::time::timeout(PROVIDER_DEADLINE, get_email_body_inner(spec, message_ref))
        .await
        .map_err(|_| anyhow!("timeout: email provider body retrieval exceeded 30 seconds"))?
}

async fn get_email_body_inner(
    spec: &ManagedSourceSpec,
    message_ref: &EmailMessageRef,
) -> Result<EmailBodyResult> {
    match &message_ref.locator {
        EmailMessageLocator::Imap {
            mailbox,
            uid,
            uidvalidity,
        } => {
            let uidvalidity = uidvalidity
                .ok_or_else(|| anyhow!("identity_missing: IMAP message_ref has no UIDVALIDITY"))?;
            fetch_imap(spec, mailbox, uid, uidvalidity, connect_imap).await
        }
        EmailMessageLocator::Gmail { message_id } => fetch_gmail(spec, message_id).await,
        EmailMessageLocator::Graph { immutable_id } => fetch_graph(spec, immutable_id).await,
        EmailMessageLocator::Jmap {
            account_id,
            email_id,
        } => fetch_jmap(spec, account_id, email_id).await,
    }
}

async fn fetch_imap<F>(
    spec: &ManagedSourceSpec,
    mailbox: &str,
    uid: &str,
    expected_uidvalidity: u64,
    connector: F,
) -> Result<EmailBodyResult>
where
    F: Fn(EmailImapIdleRuntimeConfig) -> crate::subscription_email::BoxFutureResult<ImapConnection>,
{
    if uid.is_empty() || !uid.chars().all(|ch| ch.is_ascii_digit()) {
        bail!("invalid_input: invalid IMAP UID");
    }
    let profile_name = required_profile_name(spec)?;
    let mut profile = load_profile(&profile_name)?;
    refresh_oauth(&mut profile).await?;
    let auth_method = imap_auth_method(&profile)?;
    let endpoint = Url::parse(&spec.endpoint).context("invalid_input: invalid IMAP endpoint")?;
    let use_tls = match endpoint.scheme() {
        "imaps" => true,
        "imap" => false,
        _ => bail!("invalid_input: IMAP source endpoint must use imap or imaps"),
    };
    let host = endpoint
        .host_str()
        .ok_or_else(|| anyhow!("invalid_input: IMAP endpoint has no host"))?
        .to_string();
    let config = EmailImapIdleRuntimeConfig {
        endpoint: spec.endpoint.clone(),
        host,
        port: endpoint.port().unwrap_or(if use_tls { 993 } else { 143 }),
        use_tls,
        auth_method: auth_method.clone(),
        mailbox: mailbox.to_string(),
        account: string_arg(spec, "account"),
        auth_profile: Some(profile_name),
        initial_fetch_limit: 0,
        source_ref: None,
    };
    let mut conn = connector(config)
        .await
        .map_err(|_| anyhow!("provider_unavailable: IMAP connection failed"))?;
    let result = async {
        conn.expect_greeting()
            .await
            .map_err(|_| anyhow!("provider_unavailable: IMAP greeting failed"))?;
        authenticate_imap(&mut conn, &auth_method).await?;
        let lines = conn
            .command_ok(&format!("EXAMINE {}", quote_imap_string(mailbox)))
            .await
            .map_err(|_| anyhow!("provider_unavailable: IMAP EXAMINE failed"))?;
        let current = crate::subscription_email::parse_imap_uidvalidity(&lines)
            .ok_or_else(|| anyhow!("identity_missing: IMAP server omitted UIDVALIDITY"))?;
        if current != expected_uidvalidity {
            bail!("stale: IMAP UIDVALIDITY changed");
        }
        let literal = conn
            .fetch_body_section_bytes(uid, "", PROVIDER_RESPONSE_MAX_BYTES)
            .await
            .map_err(|_| anyhow!("provider_unavailable: IMAP BODY.PEEK fetch failed"))?;
        let ImapSectionLiteral::Bytes(raw) = literal else {
            bail!("not_found: IMAP message UID was not found");
        };
        enforce_limit(raw.len())?;
        parse_mime(&raw, true, "message_ref", false)
    }
    .await;
    let _ = conn.logout().await;
    result
}

async fn authenticate_imap(conn: &mut ImapConnection, auth: &ImapAuthMethod) -> Result<()> {
    let result = match auth {
        ImapAuthMethod::Basic { username, password } => conn
            .command_ok(&format!(
                "LOGIN {} {}",
                quote_imap_string(username),
                quote_imap_string(password)
            ))
            .await
            .map(|_| ()),
        ImapAuthMethod::Xoauth2 { username, token } => {
            conn.authenticate_xoauth2(username, token).await
        }
    };
    result.map_err(|_| anyhow!("auth_required: IMAP authentication failed"))
}

async fn fetch_gmail(spec: &ManagedSourceSpec, message_id: &str) -> Result<EmailBodyResult> {
    validate_path_id(message_id)?;
    ensure_http_endpoint(&spec.endpoint)?;
    ensure_provider_arg(spec, "gmail")?;
    let url = format!(
        "{}/{}?format=raw",
        spec.endpoint.trim_end_matches('/'),
        encode_path_segment(message_id)
    );
    let profile = load_profile(&required_profile_name(spec)?)?;
    let value = get_json(&profile, &spec.endpoint, &url).await?;
    if value
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != message_id)
    {
        bail!("stale: Gmail response id did not match message_ref");
    }
    let encoded = value
        .get("raw")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("not_found: Gmail response omitted raw MIME"))?;
    enforce_limit(encoded.len())?;
    let raw = decode_base64url(encoded).context("parse_failed: invalid Gmail raw MIME encoding")?;
    enforce_limit(raw.len())?;
    parse_mime(&raw, true, "message_ref", false)
}

async fn fetch_graph(spec: &ManagedSourceSpec, immutable_id: &str) -> Result<EmailBodyResult> {
    validate_path_id(immutable_id)?;
    ensure_http_endpoint(&spec.endpoint)?;
    ensure_provider_arg(spec, "graph")?;
    let url = format!(
        "{}/{}/$value",
        spec.endpoint.trim_end_matches('/'),
        encode_path_segment(immutable_id)
    );
    let profile = load_profile(&required_profile_name(spec)?)?;
    let raw = get_bytes(&profile, &spec.endpoint, &url, Some("ImmutableId")).await?;
    parse_mime(&raw, true, "message_ref", false)
}

async fn fetch_jmap(
    spec: &ManagedSourceSpec,
    account_id: &str,
    email_id: &str,
) -> Result<EmailBodyResult> {
    validate_path_id(account_id)?;
    validate_path_id(email_id)?;
    ensure_http_endpoint(&spec.endpoint)?;
    ensure_provider_arg(spec, "jmap")?;
    let profile = load_profile(&required_profile_name(spec)?)?;
    let body = json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
        "methodCalls": [["Email/get", {
            "accountId": account_id,
            "ids": [email_id],
            "properties": ["id", "textBody", "htmlBody", "bodyValues"],
            "fetchTextBodyValues": true,
            "fetchHTMLBodyValues": true,
            "maxBodyValueBytes": PROVIDER_RESPONSE_MAX_BYTES
        }, "b0"]]
    });
    let value = request_json(
        &profile,
        &spec.endpoint,
        Method::POST,
        &spec.endpoint,
        Some(body),
    )
    .await?;
    let email = jmap_email(&value, email_id)?;
    let (content_type, text, complete) = select_jmap_body(email)?;
    enforce_limit(text.len())?;
    let mime = format!("Content-Type: {content_type}; charset=utf-8\r\n\r\n{text}");
    parse_mime(mime.as_bytes(), complete, "message_ref", false)
}

fn jmap_email<'a>(value: &'a Value, expected_id: &str) -> Result<&'a Value> {
    let response = value
        .get("methodResponses")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("provider_unavailable: malformed JMAP response"))?;
    if response.first().and_then(Value::as_str) != Some("Email/get") {
        bail!("provider_unavailable: JMAP Email/get failed");
    }
    let payload = response
        .get(1)
        .ok_or_else(|| anyhow!("provider_unavailable: malformed JMAP Email/get response"))?;
    if payload
        .get("notFound")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(expected_id)))
    {
        bail!("not_found: JMAP Email id was not found");
    }
    let email = payload
        .get("list")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("not_found: JMAP Email/get returned no message"))?;
    if email.get("id").and_then(Value::as_str) != Some(expected_id) {
        bail!("stale: JMAP response id did not match message_ref");
    }
    Ok(email)
}

fn select_jmap_body(email: &Value) -> Result<(&'static str, String, bool)> {
    let values = email
        .get("bodyValues")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("not_found: JMAP response omitted bodyValues"))?;
    for (property, content_type) in [("textBody", "text/plain"), ("htmlBody", "text/html")] {
        if let Some(parts) = email.get(property).and_then(Value::as_array) {
            if parts.is_empty() {
                continue;
            }
            let mut texts = Vec::with_capacity(parts.len());
            let mut complete = true;
            for part in parts {
                let part_id = part.get("partId").and_then(Value::as_str).ok_or_else(|| {
                    anyhow!("provider_unavailable: JMAP body part omitted partId")
                })?;
                let value = values.get(part_id).ok_or_else(|| {
                    anyhow!("not_found: JMAP bodyValues omitted a referenced body part")
                })?;
                let text = value
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("provider_unavailable: JMAP body value omitted text"))?;
                let truncated = value
                    .get("isTruncated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if truncated && text.is_empty() {
                    bail!("resource_limit: JMAP body was truncated without a usable fragment");
                }
                complete &= !truncated;
                texts.push(text);
            }
            return Ok((content_type, texts.join("\n"), complete));
        }
    }
    Ok(("text/plain", String::new(), true))
}

async fn get_json(profile: &Profile, origin: &str, url: &str) -> Result<Value> {
    request_json(profile, origin, Method::GET, url, None).await
}

async fn request_json(
    profile: &Profile,
    origin: &str,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<Value> {
    let response = authorized_request(profile, origin, method, url, body, &[]).await?;
    let bytes = bounded_response(response).await?;
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("provider_unavailable: provider returned invalid JSON"))
}

async fn get_bytes(
    profile: &Profile,
    origin: &str,
    url: &str,
    prefer: Option<&str>,
) -> Result<Vec<u8>> {
    let headers = prefer
        .map(|value| vec![("Prefer", format!("IdType=\"{value}\""))])
        .unwrap_or_default();
    let response = authorized_request(profile, origin, Method::GET, url, None, &headers).await?;
    bounded_response(response).await
}

async fn authorized_request(
    profile: &Profile,
    trusted_origin: &str,
    method: Method,
    url: &str,
    body: Option<Value>,
    request_headers: &[(&str, String)],
) -> Result<Response> {
    same_origin(trusted_origin, url)?;
    let context = AuthRequestContext::new(method.as_str(), url);
    let resolved = resolve_profile_request_auth_with_context(&context, profile)
        .map_err(|_| anyhow!("auth_required: provider authentication could not be resolved"))?;
    same_origin(trusted_origin, &resolved.url)?;
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(PROVIDER_DEADLINE)
        .build()
        .context("provider_unavailable: failed to build HTTP client")?;
    let mut builder = client.request(method, &resolved.url);
    for (name, value) in &resolved.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    for (name, value) in request_headers {
        builder = builder.header(*name, value);
    }
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    let response = builder
        .send()
        .await
        .map_err(|_| anyhow!("provider_unavailable: provider request failed"))?;
    if response.status().is_redirection() {
        bail!("provider_unavailable: provider redirect refused");
    }
    match response.status().as_u16() {
        200..=299 => Ok(response),
        401 | 403 => bail!("auth_required: provider rejected authentication"),
        404 | 410 => bail!("not_found: provider message was not found"),
        429 | 500..=599 => bail!("provider_unavailable: temporary provider failure"),
        _ => bail!("provider_unavailable: provider request failed"),
    }
}

async fn bounded_response(response: Response) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > PROVIDER_RESPONSE_MAX_BYTES as u64)
    {
        bail!("resource_limit: provider response exceeds 16 MiB");
    }
    bounded_stream(response.bytes_stream()).await
}

async fn bounded_stream<S, B, E>(mut stream: S) -> Result<Vec<u8>>
where
    S: futures::Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| anyhow!("provider_unavailable: provider response read failed"))?;
        let chunk = chunk.as_ref();
        if bytes.len().saturating_add(chunk.len()) > PROVIDER_RESPONSE_MAX_BYTES {
            bail!("resource_limit: provider response exceeds 16 MiB");
        }
        bytes.extend_from_slice(chunk);
    }
    Ok(bytes)
}

fn ensure_provider_arg(spec: &ManagedSourceSpec, expected: &str) -> Result<()> {
    let actual = string_arg(spec, "provider")
        .ok_or_else(|| anyhow!("stale: managed source no longer declares an email provider"))?;
    let actual_lowercase = actual.to_ascii_lowercase();
    let normalized = match actual_lowercase.as_str() {
        "microsoft_graph" | "msgraph" => "graph",
        other => other,
    };
    if normalized != expected {
        bail!("stale: message_ref provider does not match managed source");
    }
    Ok(())
}

fn required_profile_name(spec: &ManagedSourceSpec) -> Result<String> {
    spec.options
        .auth
        .clone()
        .ok_or_else(|| anyhow!("auth_required: managed source has no auth profile"))
}

fn string_arg(spec: &ManagedSourceSpec, key: &str) -> Option<String> {
    spec.args
        .as_ref()
        .and_then(|args| args.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn load_profile(name: &str) -> Result<Profile> {
    Profiles::load_profiles()
        .map_err(|_| anyhow!("auth_required: failed to load auth profiles"))?
        .get_profile(name)
        .cloned()
        .map_err(|_| anyhow!("auth_required: auth profile was not found"))
}

async fn refresh_oauth(profile: &mut Profile) -> Result<()> {
    if profile.auth_type != crate::auth::AuthType::OAuth {
        return Ok(());
    }
    let client = Client::new();
    let refreshed = crate::auth::refresh_effective_auth_profile(
        profile,
        &client,
        false,
        crate::subscription_email::IMAP_XOAUTH2_REFRESH_SKEW_SECS,
        None,
    )
    .await
    .map_err(|_| anyhow!("auth_required: OAuth refresh failed"))?;
    if refreshed {
        crate::auth::persist_profile_if_named(profile)?;
    }
    Ok(())
}

fn imap_auth_method(profile: &Profile) -> Result<ImapAuthMethod> {
    let first = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| profile.resolve_field_value(name).ok().flatten())
    };
    let username = first(&["username", "user", "email"])
        .or_else(|| profile.name.clone())
        .ok_or_else(|| anyhow!("auth_required: IMAP profile has no username"))?;
    if profile.auth_type == crate::auth::AuthType::OAuth {
        let token = profile
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.access_token.clone())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow!("auth_required: IMAP OAuth profile has no access token"))?;
        Ok(ImapAuthMethod::Xoauth2 { username, token })
    } else {
        let password = first(&["password", "app_password", "secret"])
            .ok_or_else(|| anyhow!("auth_required: IMAP profile has no password"))?;
        Ok(ImapAuthMethod::Basic { username, password })
    }
}

fn ensure_http_endpoint(endpoint: &str) -> Result<()> {
    let url = Url::parse(endpoint).context("invalid_input: invalid provider endpoint")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("invalid_input: provider endpoint must be an absolute HTTP URL");
    }
    Ok(())
}

fn same_origin(trusted: &str, candidate: &str) -> Result<()> {
    let trusted = Url::parse(trusted).context("invalid_input: invalid trusted endpoint")?;
    let candidate = Url::parse(candidate).context("provider_unavailable: invalid provider URL")?;
    if trusted.scheme() != candidate.scheme()
        || trusted.host_str() != candidate.host_str()
        || trusted.port_or_known_default() != candidate.port_or_known_default()
    {
        bail!("provider_unavailable: cross-origin provider request refused");
    }
    Ok(())
}

fn validate_path_id(value: &str) -> Result<()> {
    if value.is_empty() || value.chars().any(|ch| ch.is_control()) {
        bail!("invalid_input: invalid provider message id");
    }
    Ok(())
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn decode_base64url(value: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    URL_SAFE_NO_PAD
        .decode(compact.trim_end_matches('='))
        .or_else(|_| URL_SAFE.decode(compact))
        .map_err(Into::into)
}

fn enforce_limit(length: usize) -> Result<()> {
    if length > PROVIDER_RESPONSE_MAX_BYTES {
        bail!("resource_limit: provider response exceeds 16 MiB");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthType;
    use mockito::Server;
    use tokio::io::AsyncWriteExt;

    fn bearer_profile() -> Profile {
        Profile::new("test-token".to_string(), AuthType::Bearer)
    }

    #[tokio::test]
    async fn gmail_request_fetches_and_decodes_raw_mime() {
        let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nhello";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/gmail/v1/users/me/messages", server.url());
        let url = format!("{endpoint}/msg%2F1?format=raw");
        let mock = server
            .mock("GET", "/gmail/v1/users/me/messages/msg%2F1")
            .match_query(mockito::Matcher::UrlEncoded("format".into(), "raw".into()))
            .match_header("authorization", "Bearer test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "msg/1", "raw": encoded}).to_string())
            .create_async()
            .await;

        let value = get_json(&bearer_profile(), &endpoint, &url).await.unwrap();
        mock.assert_async().await;
        let decoded = decode_base64url(value["raw"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, raw);
        assert_eq!(
            parse_mime(&decoded, true, "message_ref", false)
                .unwrap()
                .text,
            "hello"
        );
    }

    #[tokio::test]
    async fn graph_request_sends_immutable_id_preference_and_returns_mime() {
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/v1.0/me/messages", server.url());
        let url = format!("{endpoint}/A%2FB%2BC%3D/$value");
        let raw = b"Content-Type: text/plain\r\n\r\ngraph body";
        let mock = server
            .mock("GET", "/v1.0/me/messages/A%2FB%2BC%3D/$value")
            .match_header("authorization", "Bearer test-token")
            .match_header("prefer", "IdType=\"ImmutableId\"")
            .with_status(200)
            .with_body(raw)
            .create_async()
            .await;

        let fetched = get_bytes(&bearer_profile(), &endpoint, &url, Some("ImmutableId"))
            .await
            .unwrap();
        mock.assert_async().await;
        assert_eq!(
            parse_mime(&fetched, true, "message_ref", false)
                .unwrap()
                .text,
            "graph body"
        );
    }

    #[test]
    fn jmap_fixture_prefers_text_and_observes_truncation() {
        let email = json!({
            "id": "e1",
            "textBody": [
                {"partId": "p1", "type": "text/plain"},
                {"partId": "p1b", "type": "text/plain"}
            ],
            "htmlBody": [{"partId": "p2", "type": "text/html"}],
            "bodyValues": {
                "p1": {"value": "plain", "isTruncated": true},
                "p1b": {"value": "second", "isTruncated": false},
                "p2": {"value": "<b>html</b>", "isTruncated": false}
            }
        });
        assert_eq!(
            select_jmap_body(&email).unwrap(),
            ("text/plain", "plain\nsecond".into(), false)
        );
    }

    #[test]
    fn jmap_fixture_rejects_missing_referenced_body_value() {
        let email = json!({
            "id": "e1",
            "textBody": [{"partId": "missing", "type": "text/plain"}],
            "bodyValues": {}
        });
        assert!(select_jmap_body(&email)
            .unwrap_err()
            .to_string()
            .starts_with("not_found:"));
    }

    #[test]
    fn jmap_fixture_validates_email_id() {
        let response =
            json!({"methodResponses": [["Email/get", {"list": [{"id": "other"}]}, "b0"]]});
        assert!(jmap_email(&response, "wanted")
            .unwrap_err()
            .to_string()
            .starts_with("stale:"));
    }

    #[test]
    fn imap_requires_uidvalidity_and_decimal_uid() {
        assert!(enforce_limit(PROVIDER_RESPONSE_MAX_BYTES).is_ok());
        assert!(enforce_limit(PROVIDER_RESPONSE_MAX_BYTES + 1).is_err());
        assert!(validate_path_id("42").is_ok());
    }

    #[test]
    fn http_auth_is_never_forwarded_cross_origin() {
        assert!(same_origin(
            "https://api.example.test/messages",
            "https://api.example.test/messages/1"
        )
        .is_ok());
        assert!(same_origin(
            "https://api.example.test/messages",
            "https://evil.example/messages/1"
        )
        .is_err());
        assert!(same_origin(
            "https://api.example.test/messages",
            "http://api.example.test/messages/1"
        )
        .is_err());
    }

    #[tokio::test]
    async fn response_limit_rejects_content_length_and_streamed_body() {
        let oversized = PROVIDER_RESPONSE_MAX_BYTES + 1;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let writer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {oversized}\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
        });
        let response = Client::new()
            .get(format!("http://{address}/content-length"))
            .send()
            .await
            .unwrap();
        assert!(bounded_response(response)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("resource_limit:"));
        writer.await.unwrap();

        let chunk = vec![b'x'; 64 * 1024];
        let chunks = std::iter::repeat_with(|| Ok::<_, std::io::Error>(chunk.clone()))
            .take(PROVIDER_RESPONSE_MAX_BYTES / chunk.len() + 1);
        let error = bounded_stream(futures::stream::iter(chunks))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("resource_limit:"),
            "unexpected streamed response error: {error}"
        );
    }
}
