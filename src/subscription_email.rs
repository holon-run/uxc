use crate::auth::Profile;
use crate::daemon::{SubscribeStartRequest, SubscriptionEventRecorder};
use crate::daemon_log::redact_endpoint;
use crate::email_attachment::{
    imap_message_attachments, ImapAttachmentContext, MIME_MAX_DEPTH, MIME_MAX_PARTS,
};
use crate::subscription_poll::{PollCheckpointState, PollFetchResult};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rustls_pki_types::ServerName;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf,
    WriteHalf,
};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use url::Url;

const DEFAULT_MAILBOX: &str = "INBOX";
const IMAP_CONNECT_TIMEOUT_SECS: u64 = 10;
const IMAP_COMMAND_TIMEOUT_SECS: u64 = 30;
const IMAP_IDLE_DONE_TIMEOUT_SECS: u64 = 5;
/// Number of existing messages fetched on the first look by default.
pub const EMAIL_IMAP_DEFAULT_INITIAL_FETCH_LIMIT: u64 = 25;
/// Upper bound for the IMAP first-look backfill depth.
pub const EMAIL_IMAP_MAX_INITIAL_FETCH_LIMIT: u64 = 100;
const IMAP_ATTACHMENT_TIMEOUT_SECS: u64 = 120;
const EMAIL_SNIPPET_CHARS: usize = 512;
const EMAIL_RAW_INLINE_BYTES: usize = 32 * 1024;
/// Marker Microsoft Exchange servers return when IMAP basic auth is disabled
/// for the account (typical for personal outlook.com/hotmail.com/live.com
/// accounts). The server rejects LOGIN before credential validation, so no
/// credential fix can help; only an OAuth-capable transport can proceed.
const IMAP_BASIC_AUTH_DISABLED_MARKER: &str = "basic authentication is disabled";

pub(crate) trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type BoxedIo = Box<dyn AsyncReadWrite>;
pub(crate) type BoxFutureResult<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

#[derive(Debug, Clone)]
pub struct EmailImapIdleRuntimeConfig {
    pub endpoint: String,
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    pub auth_method: ImapAuthMethod,
    pub mailbox: String,
    pub account: Option<String>,
    pub auth_profile: Option<String>,
    pub initial_fetch_limit: usize,
}

/// How an `email-imap-idle` session authenticates against the IMAP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImapAuthMethod {
    /// Classic `LOGIN` with username and password. Personal Microsoft
    /// accounts reject this path server-side.
    Basic { username: String, password: String },
    /// `AUTHENTICATE XOAUTH2` with an OAuth access token (Microsoft
    /// personal/organization accounts, Gmail with OAuth, etc).
    Xoauth2 { username: String, token: String },
}

impl ImapAuthMethod {
    pub(crate) fn username(&self) -> &str {
        match self {
            Self::Basic { username, .. } | Self::Xoauth2 { username, .. } => username,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailProviderKind {
    Gmail,
    Graph,
    Jmap,
}

#[derive(Debug, Clone)]
pub struct EmailProviderPollRuntimeConfig {
    pub endpoint: String,
    pub provider: EmailProviderKind,
    pub account: Option<String>,
    pub mailbox: String,
    /// Auth profile name referenced by attachment handles (never credentials).
    pub auth_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapFetchedMessage {
    pub uid: String,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
}

pub fn resolve_email_imap_idle_runtime_config(
    request: &SubscribeStartRequest,
    auth_profile: &Profile,
) -> Result<EmailImapIdleRuntimeConfig> {
    let url = Url::parse(&request.endpoint).context("invalid email IMAP endpoint")?;
    let use_tls = match url.scheme() {
        "imaps" => true,
        "imap" => false,
        other => bail!(
            "email-imap-idle transport requires imap:// or imaps:// endpoint, got '{}'",
            other
        ),
    };
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("email-imap-idle endpoint missing host"))?
        .to_string();
    let port = url.port().unwrap_or(if use_tls { 993 } else { 143 });
    let args = request.args.as_ref();
    let mailbox = args
        .and_then(|args| args.get("mailbox"))
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_MAILBOX)
        .to_string();
    let account = args
        .and_then(|args| args.get("account"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| auth_profile.resolve_field_value("account").ok().flatten());
    // 0 means "new mail only": skip the initial backfill entirely and only
    // emit messages that arrive after the subscription is established.
    let initial_fetch_limit = args
        .and_then(|args| args.get("initial_fetch_limit"))
        .and_then(Value::as_u64)
        .unwrap_or(EMAIL_IMAP_DEFAULT_INITIAL_FETCH_LIMIT)
        .min(EMAIL_IMAP_MAX_INITIAL_FETCH_LIMIT) as usize;
    let auth_method = if auth_profile.auth_type == crate::auth::AuthType::OAuth {
        let username = resolve_first_profile_field(auth_profile, &["username", "user", "email"])?
            .or_else(|| account.clone())
            .or_else(|| auth_profile.name.clone())
            .ok_or_else(|| {
                anyhow!(
                    "email-imap-idle OAuth auth profile requires a username/user/email field, an account, or the credential id to name the mailbox address"
                )
            })?;
        let token = auth_profile
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.access_token.clone())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "email-imap-idle OAuth auth profile has no access token; run `uxc auth oauth login` first"
                )
            })?;
        ImapAuthMethod::Xoauth2 { username, token }
    } else {
        let username = resolve_first_profile_field(auth_profile, &["username", "user", "email"])?
            .ok_or_else(|| {
            anyhow!("email-imap-idle auth profile requires username/user/email field")
        })?;
        let password =
            resolve_first_profile_field(auth_profile, &["password", "app_password", "secret"])?
                .ok_or_else(|| {
                    anyhow!(
                        "email-imap-idle auth profile requires password/app_password/secret field"
                    )
                })?;
        ImapAuthMethod::Basic { username, password }
    };
    Ok(EmailImapIdleRuntimeConfig {
        endpoint: request.endpoint.clone(),
        host,
        port,
        use_tls,
        auth_method,
        auth_profile: request.options.auth.clone(),
        mailbox,
        account,
        initial_fetch_limit,
    })
}

fn resolve_first_profile_field(profile: &Profile, names: &[&str]) -> Result<Option<String>> {
    for name in names {
        if let Some(value) = profile.resolve_field_value(name)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

pub(crate) async fn run_email_imap_idle_subscription_runtime<R>(
    config: EmailImapIdleRuntimeConfig,
    recorder: &mut R,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<()>
where
    R: SubscriptionEventRecorder,
{
    run_email_imap_idle_subscription_with_connector(config, recorder, stop_rx, connect_imap).await
}

async fn run_email_imap_idle_subscription_with_connector<R, C>(
    config: EmailImapIdleRuntimeConfig,
    recorder: &mut R,
    stop_rx: &mut watch::Receiver<bool>,
    connector: C,
) -> Result<()>
where
    R: SubscriptionEventRecorder,
    C: Fn(EmailImapIdleRuntimeConfig) -> BoxFutureResult<ImapConnection>,
{
    let mut config = config;
    let mut delay_secs = 1u64;
    loop {
        if let ImapAuthMethod::Xoauth2 { token, .. } = &mut config.auth_method {
            refresh_imap_xoauth2_token_if_stale(token, config.auth_profile.as_deref()).await?;
        }
        match run_email_imap_idle_session_once(&config, recorder, stop_rx, &connector).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                let message = err.to_string();
                let permanent = is_imap_permanent_auth_failure(&message);
                recorder
                    .emit(
                        "email_imap_idle",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                if permanent {
                    // Server-side policy rejection: retrying with the same
                    // credential can never succeed, so stop the backoff loop
                    // and surface the failure instead of reconnecting forever.
                    recorder
                        .update_status(Some("failed"), Some(message), false)
                        .await?;
                    return Err(err);
                }
                recorder
                    .update_status(Some("reconnecting"), Some(message), true)
                    .await?;
                recorder
                    .emit(
                        "email_imap_idle",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs })),
                    )
                    .await?;
                if wait_for_stop_or_timeout(stop_rx, Duration::from_secs(delay_secs)).await {
                    close_email_subscription(recorder, "stopped").await?;
                    return Ok(());
                }
                delay_secs = delay_secs.saturating_mul(2).min(60);
            }
        }
    }
}

fn is_imap_basic_auth_disabled_message(message: &str) -> bool {
    message
        .to_lowercase()
        .contains(IMAP_BASIC_AUTH_DISABLED_MARKER)
}

/// Auth failures that retrying with the same credentials can never fix.
/// Used to stop the reconnect backoff instead of hammering the server.
fn is_imap_permanent_auth_failure(message: &str) -> bool {
    is_imap_basic_auth_disabled_message(message) || message.contains(IMAP_XOAUTH2_REJECTED_MARKER)
}

/// Marker embedded in the actionable XOAUTH2 rejection error below.
const IMAP_XOAUTH2_REJECTED_MARKER: &str = "IMAP AUTHENTICATE XOAUTH2 rejected";

/// Refresh skew applied before reconnecting: refresh slightly early so the
/// freshly connected session never carries an about-to-expire token.
pub(crate) const IMAP_XOAUTH2_REFRESH_SKEW_SECS: i64 = 120;

async fn refresh_imap_xoauth2_token_if_stale(
    token: &mut String,
    profile_name: Option<&str>,
) -> Result<()> {
    let Some(name) = profile_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        // Anonymous profile: nothing to refresh on disk; the resolved token
        // from subscribe time is used as-is.
        return Ok(());
    };
    let profiles = crate::auth::Profiles::load_profiles()?;
    let mut profile = match profiles.get_profile(name) {
        Ok(profile) => profile.clone(),
        Err(_) => return Ok(()),
    };
    if profile.auth_type != crate::auth::AuthType::OAuth {
        return Ok(());
    }
    let client = reqwest::Client::new();
    let refreshed = crate::auth::refresh_effective_auth_profile(
        &mut profile,
        &client,
        false,
        IMAP_XOAUTH2_REFRESH_SKEW_SECS,
        None,
    )
    .await;
    match refreshed {
        Ok(true) => {
            crate::auth::persist_profile_if_named(&profile)?;
        }
        Ok(false) => {}
        Err(err) => {
            // Keep the current token when refresh fails transiently; the
            // server rejects it via AUTHENTICATE if it really is dead.
            if token.is_empty() {
                return Err(err);
            }
        }
    }
    if let Some(new_token) = profile
        .oauth
        .as_ref()
        .and_then(|oauth| oauth.access_token.clone())
        .filter(|value| !value.is_empty())
    {
        *token = new_token;
    }
    Ok(())
}

/// Build the XOAUTH2 SASL initial-response string:
/// `user=<u>\x01auth=Bearer <token>\x01\x01`.
pub(crate) fn build_xoauth2_sasl_string(username: &str, token: &str) -> String {
    format!("user={username}\x01auth=Bearer {token}\x01\x01")
}

fn decode_base64_json_challenge(payload: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    if serde_json::from_str::<serde_json::Value>(&text).is_ok() {
        Some(text)
    } else {
        None
    }
}

fn imap_xoauth2_rejected_error(tagged_line: &str, challenge_json: Option<&str>) -> anyhow::Error {
    let mut server_detail = tagged_line.trim().to_string();
    if let Some(challenge) = challenge_json {
        server_detail.push_str("; challenge: ");
        server_detail.push_str(challenge);
    }
    anyhow!(
        "IMAP AUTHENTICATE XOAUTH2 rejected: the access token is missing, expired, or lacks the \
         required IMAP scope (for Microsoft accounts: \
         https://outlook.office.com/IMAP.AccessAsUser.All). Re-run \
         `uxc auth oauth login <credential> --provider outlook` (or with your own --client-id) \
         to consent and refresh the token, then restart the source. Server response: {}",
        server_detail
    )
}

async fn imap_authenticate_xoauth2(
    conn: &mut ImapConnection,
    username: &str,
    token: &str,
) -> Result<()> {
    conn.authenticate_xoauth2(username, token).await
}

fn imap_basic_auth_disabled_error(server_lines: &[String]) -> anyhow::Error {
    anyhow!(
        "IMAP LOGIN rejected: the server disabled basic authentication for this account. \
         Microsoft personal accounts (outlook.com/hotmail.com/live.com) reject IMAP basic auth \
         before validating credentials, so fixing the password cannot help. Use an OAuth-capable \
         transport (IMAP XOAUTH2 or `email-provider-poll` with provider=graph) instead. \
         Server response: {}",
        server_lines.last().cloned().unwrap_or_default()
    )
}

async fn imap_login(conn: &mut ImapConnection, username: &str, password: &str) -> Result<()> {
    let lines = conn
        .command(&format!(
            "LOGIN {} {}",
            quote_imap_string(username),
            quote_imap_string(password)
        ))
        .await?;
    if !lines.last().is_some_and(|line| line.contains(" OK")) {
        if lines
            .iter()
            .any(|line| is_imap_basic_auth_disabled_message(line))
        {
            return Err(imap_basic_auth_disabled_error(&lines));
        }
        bail!(
            "IMAP command failed: {}",
            lines.last().cloned().unwrap_or_default()
        );
    }
    Ok(())
}

async fn run_email_imap_idle_session_once<R, C>(
    config: &EmailImapIdleRuntimeConfig,
    recorder: &mut R,
    stop_rx: &mut watch::Receiver<bool>,
    connector: &C,
) -> Result<()>
where
    R: SubscriptionEventRecorder,
    C: Fn(EmailImapIdleRuntimeConfig) -> BoxFutureResult<ImapConnection>,
{
    if *stop_rx.borrow() {
        close_email_subscription(recorder, "stopped").await?;
        return Ok(());
    }
    let mut conn = connector(config.clone()).await?;
    conn.expect_greeting().await?;
    match &config.auth_method {
        ImapAuthMethod::Basic { username, password } => {
            imap_login(&mut conn, username, password).await?;
        }
        ImapAuthMethod::Xoauth2 { username, token } => {
            imap_authenticate_xoauth2(&mut conn, username, token).await?;
        }
    }
    let select_lines = conn
        .command_ok(&format!("SELECT {}", quote_imap_string(&config.mailbox)))
        .await?;
    let uidvalidity = parse_imap_uidvalidity(&select_lines);
    recorder
        .emit(
            "email_imap_idle",
            "open",
            None,
            Some(json!({
                "url": redact_endpoint(&config.endpoint),
                "mailbox": config.mailbox,
                "account": config.account,
            })),
        )
        .await?;
    recorder.update_status(Some("running"), None, false).await?;

    let mut last_seen_uid = emit_recent_messages(config, &mut conn, recorder, uidvalidity).await?;

    loop {
        if *stop_rx.borrow() {
            let _ = conn.logout().await;
            close_email_subscription(recorder, "stopped").await?;
            return Ok(());
        }
        let idle_tag = conn.start_idle().await?;
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_ok() && *stop_rx.borrow() {
                    conn.done_idle(&idle_tag).await?;
                    let _ = conn.logout().await;
                    close_email_subscription(recorder, "stopped").await?;
                    return Ok(());
                }
            }
            line = conn.read_line() => {
                let line = line?;
                if line.contains(" EXISTS") || line.contains(" RECENT") {
                    conn.done_idle(&idle_tag).await?;
                    let messages = match last_seen_uid {
                        Some(uid) => conn.fetch_since_uid(uid).await?,
                        None => conn.fetch_recent(config.initial_fetch_limit).await?,
                    };
                    if let Some(uid) =
                        emit_messages(config, messages, recorder, uidvalidity).await?
                    {
                        last_seen_uid = Some(last_seen_uid.map_or(uid, |last| last.max(uid)));
                    }
                }
            }
        }
    }
}

pub(crate) fn connect_imap(config: EmailImapIdleRuntimeConfig) -> BoxFutureResult<ImapConnection> {
    Box::pin(async move {
        let tcp = tokio::time::timeout(
            Duration::from_secs(IMAP_CONNECT_TIMEOUT_SECS),
            TcpStream::connect((config.host.as_str(), config.port)),
        )
        .await
        .context("IMAP connect timed out")?
        .with_context(|| format!("failed to connect to {}:{}", config.host, config.port))?;
        let io: BoxedIo = if config.use_tls {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let tls_config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let connector = TlsConnector::from(Arc::new(tls_config));
            let server_name = ServerName::try_from(config.host.clone())
                .map_err(|_| anyhow!("invalid IMAP TLS server name"))?;
            Box::new(
                tokio::time::timeout(
                    Duration::from_secs(IMAP_CONNECT_TIMEOUT_SECS),
                    connector.connect(server_name, tcp),
                )
                .await
                .context("IMAP TLS handshake timed out")?
                .context("IMAP TLS handshake failed")?,
            )
        } else {
            Box::new(tcp)
        };
        Ok(ImapConnection::new(io))
    })
}

async fn emit_recent_messages<R>(
    config: &EmailImapIdleRuntimeConfig,
    conn: &mut ImapConnection,
    recorder: &mut R,
    uidvalidity: Option<u64>,
) -> Result<Option<u64>>
where
    R: SubscriptionEventRecorder,
{
    if config.initial_fetch_limit == 0 {
        // New mail only: establish the high-water UID without fetching or
        // emitting any pre-existing messages.
        return conn.latest_uid().await;
    }
    let messages = conn.fetch_recent(config.initial_fetch_limit).await?;
    emit_messages(config, messages, recorder, uidvalidity).await
}

async fn emit_messages<R>(
    config: &EmailImapIdleRuntimeConfig,
    messages: Vec<ImapFetchedMessage>,
    recorder: &mut R,
    uidvalidity: Option<u64>,
) -> Result<Option<u64>>
where
    R: SubscriptionEventRecorder,
{
    let mut max_uid = None;
    for message in messages {
        if let Some(uid) = parse_uid_number(&message.uid) {
            max_uid = Some(max_uid.map_or(uid, |max: u64| max.max(uid)));
        }
        recorder
            .emit(
                "email_imap_idle",
                "data",
                Some(build_email_event(config, &message, uidvalidity)),
                None,
            )
            .await?;
    }
    Ok(max_uid)
}

fn build_email_event(
    config: &EmailImapIdleRuntimeConfig,
    message: &ImapFetchedMessage,
    uidvalidity: Option<u64>,
) -> Value {
    let raw_len = message.raw.len();
    let raw_text = String::from_utf8_lossy(&message.raw);
    let headers = parse_email_headers(&raw_text);
    let subject = header_value(&headers, "subject");
    let message_id = header_value(&headers, "message-id").unwrap_or_else(|| message.uid.clone());
    let thread_id =
        header_value(&headers, "references").or_else(|| header_value(&headers, "in-reply-to"));
    let account = config
        .account
        .as_deref()
        .unwrap_or(config.auth_method.username());
    let attachments = imap_message_attachments(
        &message.raw,
        &ImapAttachmentContext {
            endpoint: &imap_public_endpoint(config),
            account,
            mailbox: &config.mailbox,
            message_id: &message_id,
            uid: &message.uid,
            uidvalidity,
            auth_profile: config.auth_profile.as_deref(),
        },
    );
    let has_attachments = !attachments.is_empty();
    let attachment_count = attachments.len();
    json!({
        "type": "email_event",
        "version": "v1",
        "provider": "imap",
        "account": account,
        "mailbox": config.mailbox,
        "event_kind": "message_received",
        "message": {
            "uid": message.uid,
            "message_id": message_id,
            "thread_id": thread_id,
            "conversation_id": null,
            "from": header_value(&headers, "from"),
            "to": split_address_header(header_value(&headers, "to").as_deref()),
            "cc": split_address_header(header_value(&headers, "cc").as_deref()),
            "bcc": split_address_header(header_value(&headers, "bcc").as_deref()),
            "subject": subject,
            "date": header_value(&headers, "date"),
            "snippet": body_snippet(&raw_text),
            "attachments": attachments,
            "has_attachments": has_attachments,
            "attachment_count": attachment_count,
            "flags": message.flags,
        },
        "raw": {
            "mime_inline": if raw_len <= EMAIL_RAW_INLINE_BYTES { Some(raw_text.to_string()) } else { None },
            "mime_truncated": raw_len > EMAIL_RAW_INLINE_BYTES,
            "size_bytes": raw_len,
        },
        "reply_handle": {
            "type": "email_imap",
            "provider": "imap",
            "account": account,
            "mailbox": config.mailbox,
            "message_id": message_id,
            "uid": message.uid,
        }
    })
}

pub fn resolve_email_provider_poll_runtime_config(
    request: &SubscribeStartRequest,
    auth_profile: Option<&Profile>,
) -> Result<EmailProviderPollRuntimeConfig> {
    let url = Url::parse(&request.endpoint).context("invalid email provider endpoint")?;
    match url.scheme() {
        "http" | "https" => {}
        other => bail!(
            "email-provider-poll transport requires http:// or https:// endpoint, got '{}'",
            other
        ),
    }
    let args = request.args.as_ref();
    let provider = args
        .and_then(|args| args.get("provider"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("email-provider-poll requires provider=gmail|graph|jmap"))?;
    let provider = match provider.to_ascii_lowercase().as_str() {
        "gmail" => EmailProviderKind::Gmail,
        "graph" | "microsoft_graph" | "msgraph" => EmailProviderKind::Graph,
        "jmap" => EmailProviderKind::Jmap,
        other => bail!(
            "unsupported email provider '{}'; expected gmail, graph, or jmap",
            other
        ),
    };
    let mailbox = args
        .and_then(|args| args.get("mailbox"))
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_MAILBOX)
        .to_string();
    let account = args
        .and_then(|args| args.get("account"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            auth_profile.and_then(|profile| profile.resolve_field_value("account").ok().flatten())
        });
    // The handle references the profile by name only; credentials stay in
    // the auth store and are re-resolved at lazy retrieval time.
    Ok(EmailProviderPollRuntimeConfig {
        endpoint: request.endpoint.clone(),
        provider,
        account,
        mailbox,
        auth_profile: request.options.auth.clone(),
    })
}

pub async fn fetch_email_provider_poll(
    config: &EmailProviderPollRuntimeConfig,
    auth_profile: Option<&Profile>,
    checkpoint: &PollCheckpointState,
) -> Result<PollFetchResult> {
    let started = std::time::Instant::now();
    let client = reqwest::Client::new();
    let request_context = crate::auth::AuthRequestContext::new("GET", &config.endpoint);
    let resolved_auth = match auth_profile {
        Some(profile) => Some(crate::auth::resolve_profile_request_auth_with_context(
            &request_context,
            profile,
        )?),
        None => None,
    };
    let url = resolved_auth
        .as_ref()
        .map(|auth| auth.url.as_str())
        .unwrap_or(config.endpoint.as_str());
    let mut builder = client.get(url);
    if let Some(auth) = resolved_auth.as_ref() {
        builder = builder.headers(header_map_from_pairs(&auth.headers)?);
    }
    if let Some(etag) = checkpoint.etag.as_ref() {
        builder = builder.header("if-none-match", etag);
    }
    let response = builder
        .send()
        .await
        .context("email provider poll request failed")?;
    let status = response.status();
    let response_headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
        })
        .collect::<HashMap<_, _>>();
    if status.as_u16() == 304 {
        return Ok(PollFetchResult {
            data: json!({ "items": [] }),
            duration_ms: Some(started.elapsed().as_millis() as u64),
            status_code: Some(status.as_u16()),
            response_headers,
        });
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!(
            "email provider poll request failed with HTTP {}: {}",
            status.as_u16(),
            body
        );
    }
    let raw = response
        .json::<Value>()
        .await
        .context("email provider poll response was not JSON")?;
    let items = normalize_email_provider_items(config, &raw)?;
    Ok(PollFetchResult {
        data: json!({ "items": items }),
        duration_ms: Some(started.elapsed().as_millis() as u64),
        status_code: Some(status.as_u16()),
        response_headers,
    })
}

fn header_map_from_pairs(headers: &[(String, String)]) -> Result<HeaderMap> {
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        out.insert(
            HeaderName::from_bytes(name.as_bytes())
                .with_context(|| format!("invalid auth header name '{}'", name))?,
            HeaderValue::from_str(value)
                .with_context(|| format!("invalid auth header value for '{}'", name))?,
        );
    }
    Ok(out)
}

pub fn normalize_email_provider_items(
    config: &EmailProviderPollRuntimeConfig,
    raw: &Value,
) -> Result<Vec<Value>> {
    let items = match config.provider {
        EmailProviderKind::Gmail => raw
            .get("messages")
            .or_else(|| raw.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Gmail provider response requires messages/items array"))?,
        EmailProviderKind::Graph => raw
            .get("value")
            .or_else(|| raw.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow!("Microsoft Graph provider response requires value/items array")
            })?,
        EmailProviderKind::Jmap => raw
            .get("list")
            .or_else(|| raw.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("JMAP provider response requires list/items array"))?,
    };
    Ok(items
        .iter()
        .map(|item| build_provider_email_event(config, item))
        .collect())
}

fn build_provider_email_event(config: &EmailProviderPollRuntimeConfig, item: &Value) -> Value {
    let account = config.account.as_deref().unwrap_or("default");
    let provider = match config.provider {
        EmailProviderKind::Gmail => "gmail",
        EmailProviderKind::Graph => "graph",
        EmailProviderKind::Jmap => "jmap",
    };
    let uid = first_string(item, &["id", "blobId"]).unwrap_or_else(|| item.to_string());
    let message_id = match config.provider {
        EmailProviderKind::Gmail => gmail_header(item, "Message-ID").unwrap_or_else(|| uid.clone()),
        EmailProviderKind::Graph => {
            first_string(item, &["internetMessageId"]).unwrap_or_else(|| uid.clone())
        }
        EmailProviderKind::Jmap => {
            first_string(item, &["messageId"]).unwrap_or_else(|| uid.clone())
        }
    };
    let subject = match config.provider {
        EmailProviderKind::Gmail => {
            gmail_header(item, "Subject").or_else(|| first_string(item, &["subject"]))
        }
        _ => first_string(item, &["subject"]),
    };
    let (attachments, has_attachments, attachment_count) =
        provider_message_attachments(config, provider, item, &message_id, &uid);
    json!({
        "type": "email_event",
        "version": "v1",
        "provider": provider,
        "account": account,
        "mailbox": config.mailbox,
        "event_kind": "message_received",
        "message": {
            "uid": uid,
            "message_id": message_id,
            "thread_id": first_string(item, &["threadId", "thread_id", "inReplyTo"]),
            "conversation_id": first_string(item, &["conversationId", "emailId"]),
            "from": provider_from_address(config.provider, item),
            "to": provider_address_list(config.provider, item, "to"),
            "cc": provider_address_list(config.provider, item, "cc"),
            "bcc": provider_address_list(config.provider, item, "bcc"),
            "subject": subject,
            "date": first_string(item, &["receivedDateTime", "receivedAt", "internalDate", "date"]),
            "snippet": first_string(item, &["snippet", "bodyPreview", "preview"]),
            "attachments": attachments,
            "has_attachments": has_attachments,
            "attachment_count": attachment_count,
            "flags": provider_flags(config.provider, item),
        },
        "raw": {
            "provider_payload": item,
        },
        "reply_handle": {
            "type": "email_provider",
            "provider": provider,
            "account": account,
            "mailbox": config.mailbox,
            "message_id": message_id,
            "uid": uid,
        }
    })
}

/// Credential-free context used to stamp opaque provider attachment handles.
struct ProviderAttachmentContext<'a> {
    provider: &'a str,
    endpoint: &'a str,
    account: &'a str,
    mailbox: &'a str,
    message_id: &'a str,
    uid: &'a str,
    auth_profile: Option<&'a str>,
}

/// Map provider attachment metadata onto the neutral event shape.
///
/// Returns `(entries, has_attachments, attachment_count)`. `attachment_count`
/// is `null` when the provider reports attachments but did not expand their
/// metadata (e.g. Microsoft Graph without `$expand=attachments`); consumers
/// detect that case as `has_attachments: true` with an empty list.
fn provider_message_attachments(
    config: &EmailProviderPollRuntimeConfig,
    provider: &str,
    item: &Value,
    message_id: &str,
    uid: &str,
) -> (Vec<Value>, bool, Value) {
    let ctx = ProviderAttachmentContext {
        provider,
        endpoint: &config.endpoint,
        account: config.account.as_deref().unwrap_or("default"),
        mailbox: &config.mailbox,
        message_id,
        uid,
        auth_profile: config.auth_profile.as_deref(),
    };
    let entries = match config.provider {
        EmailProviderKind::Gmail => gmail_attachment_entries(item, &ctx),
        EmailProviderKind::Graph => graph_attachment_entries(item, &ctx),
        EmailProviderKind::Jmap => jmap_attachment_entries(item, &ctx),
    };
    // Gmail exposes no `hasAttachments` boolean; metadata presence is the
    // only signal there.
    let provider_has_attachments = match config.provider {
        EmailProviderKind::Gmail => None,
        EmailProviderKind::Graph | EmailProviderKind::Jmap => {
            item.get("hasAttachments").and_then(Value::as_bool)
        }
    };
    let has_attachments = !entries.is_empty() || provider_has_attachments == Some(true);
    let attachment_count = if entries.is_empty() && provider_has_attachments == Some(true) {
        Value::Null
    } else {
        json!(entries.len())
    };
    (entries, has_attachments, attachment_count)
}

fn provider_attachment_handle(ctx: &ProviderAttachmentContext<'_>, part: Value) -> Value {
    let mut handle = Map::new();
    handle.insert("type".into(), json!("email_attachment"));
    handle.insert("provider".into(), json!(ctx.provider));
    handle.insert("endpoint".into(), json!(ctx.endpoint));
    handle.insert("account".into(), json!(ctx.account));
    handle.insert("mailbox".into(), json!(ctx.mailbox));
    handle.insert("message_id".into(), json!(ctx.message_id));
    handle.insert("uid".into(), json!(ctx.uid));
    if let Some(auth_profile) = ctx.auth_profile {
        handle.insert("auth_profile".into(), json!(auth_profile));
    }
    handle.insert("part".into(), part);
    Value::Object(handle)
}

fn gmail_attachment_entries(item: &Value, ctx: &ProviderAttachmentContext<'_>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut parts_seen = 0usize;
    if let Some(payload) = item.get("payload") {
        walk_gmail_part(payload, None, 0, &mut parts_seen, ctx, &mut out);
    }
    out
}

/// Walk Gmail `payload.parts` recursively. Leaf classification mirrors the
/// IMAP MIME walker (`email_attachment`): `multipart/alternative` text
/// variants stay body content and the disposition/filename/content-id rules
/// are identical. The walk is bounded by the same MIME limits.
fn walk_gmail_part(
    part: &Value,
    parent_subtype: Option<&str>,
    depth: usize,
    parts_seen: &mut usize,
    ctx: &ProviderAttachmentContext<'_>,
    out: &mut Vec<Value>,
) {
    if depth > MIME_MAX_DEPTH || *parts_seen >= MIME_MAX_PARTS {
        return;
    }
    *parts_seen += 1;
    let mime_type = part
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("text/plain")
        .to_ascii_lowercase();
    if let Some(subtype) = mime_type.strip_prefix("multipart/") {
        if let Some(children) = part.get("parts").and_then(Value::as_array) {
            for child in children {
                walk_gmail_part(child, Some(subtype), depth + 1, parts_seen, ctx, out);
            }
        }
        return;
    }
    let disposition = gmail_part_header(part, "Content-Disposition")
        .and_then(|value| value.split(';').next().map(str::to_string))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token == "attachment" || token == "inline");
    let filename = part
        .get("filename")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let content_id = gmail_part_header(part, "Content-ID").map(|value| value.trim().to_string());
    let is_attachment = match disposition.as_deref() {
        Some("attachment") => true,
        Some("inline") => filename.is_some() || content_id.is_some(),
        _ => match &filename {
            Some(_) => true,
            None => !(mime_type.starts_with("text/") || parent_subtype == Some("alternative")),
        },
    };
    if !is_attachment {
        return;
    }
    let body = part.get("body");
    let attachment_id = body
        .and_then(|body| body.get("attachmentId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let id = part
        .get("partId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| attachment_id.clone());
    // Parts without `body.attachmentId` (small inline bodies Gmail inlines)
    // stay metadata-only entries with a null handle.
    let handle = attachment_id.map(|attachment_id| {
        provider_attachment_handle(ctx, json!({ "attachment_id": attachment_id }))
    });
    out.push(json!({
        "id": id,
        "filename": filename,
        "content_type": mime_type,
        "size": body
            .and_then(|body| body.get("size"))
            .and_then(Value::as_u64),
        "disposition": disposition,
        "content_id": content_id,
        "handle": handle,
    }));
}

fn graph_attachment_entries(item: &Value, ctx: &ProviderAttachmentContext<'_>) -> Vec<Value> {
    item.get("attachments")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|att| graph_attachment_entry(att, ctx))
                .collect()
        })
        .unwrap_or_default()
}

fn graph_attachment_entry(att: &Value, ctx: &ProviderAttachmentContext<'_>) -> Value {
    let id = att
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    // Attachments without an `id` cannot be fetched later; keep them as
    // metadata-only entries with a null handle, mirroring Gmail/JMAP.
    let handle = id
        .as_ref()
        .map(|id| provider_attachment_handle(ctx, json!({ "attachment_id": id })));
    let disposition = att
        .get("isInline")
        .and_then(Value::as_bool)
        .map(|inline| if inline { "inline" } else { "attachment" }.to_string());
    json!({
        "id": id.clone(),
        "filename": att.get("name").and_then(Value::as_str).map(str::to_string),
        "content_type": att
            .get("contentType")
            .or_else(|| att.get("@odata.mediaContentType"))
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_lowercase()),
        "size": att.get("size").and_then(Value::as_u64),
        "disposition": disposition,
        "content_id": att.get("contentId").and_then(Value::as_str).map(str::to_string),
        "handle": handle,
    })
}

fn jmap_attachment_entries(item: &Value, ctx: &ProviderAttachmentContext<'_>) -> Vec<Value> {
    item.get("attachments")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|part| jmap_attachment_entry(part, ctx))
                .collect()
        })
        .unwrap_or_default()
}

fn jmap_attachment_entry(part: &Value, ctx: &ProviderAttachmentContext<'_>) -> Value {
    let blob_id = part
        .get("blobId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let id = part
        .get("partId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| blob_id.clone());
    let handle =
        blob_id.map(|blob_id| provider_attachment_handle(ctx, json!({ "blob_id": blob_id })));
    json!({
        "id": id,
        "filename": part
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        "content_type": part
            .get("type")
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_lowercase()),
        "size": part.get("size").and_then(Value::as_u64),
        "disposition": jmap_disposition(part.get("disposition")),
        "content_id": part
            .get("cid")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        "handle": handle,
    })
}

/// JMAP `disposition` is a plain string in RFC 8621 and a
/// `Map<String, Boolean>` in newer revisions; normalize both to the
/// neutral token.
fn jmap_disposition(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(raw)) => raw
            .split(';')
            .next()
            .map(|token| token.trim().to_ascii_lowercase())
            .filter(|token| token == "attachment" || token == "inline"),
        Some(Value::Object(map)) => {
            if map.get("attachment").and_then(Value::as_bool) == Some(true) {
                Some("attachment".to_string())
            } else if map.get("inline").and_then(Value::as_bool) == Some(true) {
                Some("inline".to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn first_string(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .map(str::to_string)
}

fn gmail_header(item: &Value, name: &str) -> Option<String> {
    item.get("payload")
        .and_then(|payload| gmail_part_header(payload, name))
}

fn gmail_part_header(part: &Value, name: &str) -> Option<String> {
    part.get("headers")
        .and_then(Value::as_array)?
        .iter()
        .find(|header| {
            header
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .and_then(|header| header.get("value").and_then(Value::as_str))
        .map(str::to_string)
}

fn provider_from_address(provider: EmailProviderKind, item: &Value) -> Option<Value> {
    match provider {
        EmailProviderKind::Gmail => gmail_header(item, "From").map(|raw| json!({ "raw": raw })),
        EmailProviderKind::Graph => item
            .pointer("/from/emailAddress")
            .cloned()
            .or_else(|| first_string(item, &["from"]).map(|raw| json!({ "raw": raw }))),
        EmailProviderKind::Jmap => item
            .get("from")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .cloned()
            .or_else(|| first_string(item, &["from"]).map(|raw| json!({ "raw": raw }))),
    }
}

fn provider_address_list(provider: EmailProviderKind, item: &Value, field: &str) -> Vec<Value> {
    match provider {
        EmailProviderKind::Gmail => {
            let header = match field {
                "to" => "To",
                "cc" => "Cc",
                "bcc" => "Bcc",
                _ => field,
            };
            split_address_header(gmail_header(item, header).as_deref())
        }
        EmailProviderKind::Graph => item
            .get(format!("{field}Recipients"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.get("emailAddress").cloned())
                    .collect()
            })
            .unwrap_or_default(),
        EmailProviderKind::Jmap => item
            .get(field)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

fn provider_flags(provider: EmailProviderKind, item: &Value) -> Vec<Value> {
    match provider {
        EmailProviderKind::Gmail => item
            .get("labelIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        EmailProviderKind::Graph => {
            let mut flags = Vec::new();
            if item.get("isRead").and_then(Value::as_bool) == Some(true) {
                flags.push(Value::String("read".to_string()));
            }
            if item.get("hasAttachments").and_then(Value::as_bool) == Some(true) {
                flags.push(Value::String("has_attachments".to_string()));
            }
            flags
        }
        EmailProviderKind::Jmap => item
            .get("keywords")
            .and_then(Value::as_object)
            .map(|keywords| keywords.keys().cloned().map(Value::String).collect())
            .unwrap_or_default(),
    }
}

fn parse_email_headers(raw: &str) -> Map<String, Value> {
    let header_text = raw
        .split("\r\n\r\n")
        .next()
        .unwrap_or(raw)
        .split("\n\n")
        .next()
        .unwrap_or(raw);
    let mut headers = Map::new();
    let mut current_name: Option<String> = None;
    let mut current_value = String::new();
    for line in header_text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if !current_value.is_empty() {
                current_value.push(' ');
            }
            current_value.push_str(line.trim());
            continue;
        }
        if let Some(name) = current_name.take() {
            headers.insert(name, Value::String(current_value.trim().to_string()));
            current_value.clear();
        }
        if let Some((name, value)) = line.split_once(':') {
            current_name = Some(name.trim().to_ascii_lowercase());
            current_value.push_str(value.trim());
        }
    }
    if let Some(name) = current_name {
        headers.insert(name, Value::String(current_value.trim().to_string()));
    }
    headers
}

fn header_value(headers: &Map<String, Value>, name: &str) -> Option<String> {
    headers
        .get(&name.to_ascii_lowercase())
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn split_address_header(value: Option<&str>) -> Vec<Value> {
    value
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| json!({ "raw": value }))
        .collect()
}

fn body_snippet(raw: &str) -> Option<String> {
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .or_else(|| raw.split_once("\n\n").map(|(_, body)| body))?;
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.chars().take(EMAIL_SNIPPET_CHARS).collect())
    }
}

pub(crate) fn quote_imap_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Rebuild a credential-free IMAP endpoint (`imap://host:port` or
/// `imaps://host:port`) for embedding in attachment retrieval handles.
/// Userinfo that may be present in the original endpoint URL is dropped.
fn imap_public_endpoint(config: &EmailImapIdleRuntimeConfig) -> String {
    format!(
        "{}://{}:{}",
        if config.use_tls { "imaps" } else { "imap" },
        config.host,
        config.port
    )
}

/// Extract the mailbox UIDVALIDITY from an IMAP SELECT response.
pub(crate) fn parse_imap_uidvalidity(lines: &[String]) -> Option<u64> {
    for line in lines {
        let Some(index) = line.find("[UIDVALIDITY ") else {
            continue;
        };
        let rest = &line[index + "[UIDVALIDITY ".len()..];
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if let Ok(uidvalidity) = digits.parse() {
            return Some(uidvalidity);
        }
    }
    None
}

async fn close_email_subscription<R: SubscriptionEventRecorder>(
    recorder: &mut R,
    reason: &str,
) -> Result<()> {
    recorder
        .emit(
            "email_imap_idle",
            "closed",
            None,
            Some(json!({ "reason": reason })),
        )
        .await?;
    recorder.update_status(Some("stopped"), None, false).await
}

async fn wait_for_stop_or_timeout(stop_rx: &mut watch::Receiver<bool>, duration: Duration) -> bool {
    if *stop_rx.borrow() {
        return true;
    }
    tokio::select! {
        changed = stop_rx.changed() => matches!(changed, Ok(())) && *stop_rx.borrow(),
        _ = tokio::time::sleep(duration) => false,
    }
}

/// Outcome of a `UID FETCH ... BODY.PEEK[<section>]` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImapSectionLiteral {
    /// The server produced no FETCH response for the UID, meaning the
    /// message no longer exists (or never did) in the mailbox.
    Missing,
    /// The server returned `BODY[<section>] NIL` for the section.
    Nil,
    /// Raw (still content-transfer-encoded) literal bytes for the section.
    Bytes(Vec<u8>),
}

pub struct ImapConnection {
    reader: BufReader<ReadHalf<BoxedIo>>,
    writer: WriteHalf<BoxedIo>,
    next_tag: u64,
}

impl ImapConnection {
    pub(crate) fn new(io: BoxedIo) -> Self {
        let (reader, writer) = tokio::io::split(io);
        Self {
            reader: BufReader::new(reader),
            writer,
            next_tag: 1,
        }
    }

    pub(crate) async fn expect_greeting(&mut self) -> Result<()> {
        let line = self.read_line().await?;
        if !line.starts_with("* OK") && !line.starts_with("* PREAUTH") {
            bail!("unexpected IMAP greeting: {}", line);
        }
        Ok(())
    }

    async fn write_line(&mut self, line: &str) -> Result<()> {
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        let read = tokio::time::timeout(
            Duration::from_secs(IMAP_COMMAND_TIMEOUT_SECS),
            self.reader.read_line(&mut line),
        )
        .await
        .context("IMAP read timed out")??;
        if read == 0 {
            bail!("IMAP connection closed");
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    async fn command(&mut self, command: &str) -> Result<Vec<String>> {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        self.write_line(&format!("{tag} {command}")).await?;
        let mut lines = Vec::new();
        loop {
            let line = self.read_response_line_with_literals().await?;
            let done = line.starts_with(&format!("{tag} "));
            lines.push(line);
            if done {
                break;
            }
        }
        Ok(lines)
    }

    async fn read_response_line_with_literals(&mut self) -> Result<String> {
        let mut line = self.read_line().await?;
        while let Some(len) = trailing_literal_len(&line) {
            let mut literal = vec![0u8; len];
            tokio::time::timeout(
                Duration::from_secs(IMAP_COMMAND_TIMEOUT_SECS),
                self.reader.read_exact(&mut literal),
            )
            .await
            .context("IMAP literal read timed out")??;
            let suffix = self.read_line().await?;
            line.push_str("\r\n");
            line.push_str(&String::from_utf8_lossy(&literal));
            line.push_str(&suffix);
        }
        Ok(line)
    }

    pub(crate) async fn command_ok(&mut self, command: &str) -> Result<Vec<String>> {
        let lines = self.command(command).await?;
        if lines.last().is_some_and(|line| line.contains(" OK")) {
            Ok(lines)
        } else {
            bail!(
                "IMAP command failed: {}",
                lines.last().cloned().unwrap_or_default()
            )
        }
    }

    /// Authenticate with SASL XOAUTH2 (RFC 7628 style, OAuth 2.0 SASL).
    ///
    /// The initial response carries the base64 credential string inline. On
    /// rejection most servers (including Microsoft Exchange) answer with a
    /// `+ <base64 JSON>` continuation instead of a tagged NO, so the client
    /// must send an empty line to abort SASL before the tagged status line
    /// arrives; both shapes are handled here and mapped to one actionable
    /// error.
    pub(crate) async fn authenticate_xoauth2(&mut self, username: &str, token: &str) -> Result<()> {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(build_xoauth2_sasl_string(username, token));
        self.write_line(&format!("{tag} AUTHENTICATE XOAUTH2 {encoded}"))
            .await?;
        let mut challenge_json: Option<String> = None;
        let tagged = loop {
            let line = self.read_response_line_with_literals().await?;
            if let Some(payload) = line.strip_prefix('+') {
                challenge_json = decode_base64_json_challenge(payload);
                // Abort SASL with `*` (RFC 3501 section 6.2.2) so the server
                // finishes the exchange with a tagged NO instead of waiting forever.
                self.write_line("*").await?;
                continue;
            }
            if line.starts_with(&format!("{tag} ")) {
                break line;
            }
        };
        if tagged.contains(" OK") {
            return Ok(());
        }
        Err(imap_xoauth2_rejected_error(
            &tagged,
            challenge_json.as_deref(),
        ))
    }

    /// Fetch one MIME section by UID with binary-safe literal handling.
    ///
    /// Unlike [`ImapConnection::command`], literal bytes are returned as raw
    /// `Vec<u8>` instead of being lossily flattened into a UTF-8 string, so
    /// attachment payloads survive the round trip. The caller must validate
    /// that `uid` is decimal digits and `section` is an RFC 3501 section
    /// path before this reaches the wire.
    pub(crate) async fn fetch_body_section_bytes(
        &mut self,
        uid: &str,
        section: &str,
    ) -> Result<ImapSectionLiteral> {
        let command = format!("UID FETCH {uid} (BODY.PEEK[{section}])");
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        self.write_line(&format!("{tag} {command}")).await?;
        let marker = format!("BODY[{section}]");
        let mut result = ImapSectionLiteral::Missing;
        loop {
            let line = self.read_line().await?;
            if line.starts_with(&format!("{tag} ")) {
                if !line.contains(" OK") {
                    bail!("IMAP command failed: {}", line);
                }
                break;
            }
            if !line.contains(" FETCH ") || result != ImapSectionLiteral::Missing {
                continue;
            }
            let Some(index) = line.find(&marker) else {
                continue;
            };
            let tail = line[index + marker.len()..].trim_start();
            let literal_len = if let Some(len) = trailing_literal_len(tail) {
                Some(len)
            } else {
                tail.strip_prefix('~').and_then(trailing_literal_len)
            };
            match literal_len {
                Some(len) => {
                    let mut literal = vec![0u8; len];
                    tokio::time::timeout(
                        Duration::from_secs(IMAP_ATTACHMENT_TIMEOUT_SECS),
                        self.reader.read_exact(&mut literal),
                    )
                    .await
                    .context("IMAP attachment literal read timed out")??;
                    // Consume the remainder of the response line after the
                    // literal (typically `)\r\n`).
                    let _suffix = self.read_line().await?;
                    result = ImapSectionLiteral::Bytes(literal);
                }
                None if tail.starts_with("NIL") => {
                    result = ImapSectionLiteral::Nil;
                }
                None => {}
            }
        }
        Ok(result)
    }

    async fn fetch_recent(&mut self, limit: usize) -> Result<Vec<ImapFetchedMessage>> {
        let search_lines = self.command_ok("UID SEARCH ALL").await?;
        let mut uids = parse_uid_search_uids(&search_lines);
        if limit > 0 && uids.len() > limit {
            uids = uids.split_off(uids.len() - limit);
        }
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.fetch_uid_set(&uids.join(",")).await
    }

    /// Returns the highest message UID in the selected mailbox without
    /// fetching any message bodies. Used to establish the high-water mark for
    /// `initial_fetch_limit = 0` ("new mail only") subscriptions.
    async fn latest_uid(&mut self) -> Result<Option<u64>> {
        let search_lines = self.command_ok("UID SEARCH ALL").await?;
        Ok(parse_uid_search_uids(&search_lines)
            .iter()
            .filter_map(|uid| parse_uid_number(uid))
            .max())
    }

    async fn fetch_since_uid(&mut self, last_uid: u64) -> Result<Vec<ImapFetchedMessage>> {
        self.fetch_uid_set(&format!("{}:*", last_uid.saturating_add(1)))
            .await
    }

    async fn fetch_uid_set(&mut self, uid_set: &str) -> Result<Vec<ImapFetchedMessage>> {
        let lines = self
            .command_ok(&format!("UID FETCH {uid_set} (UID FLAGS BODY.PEEK[])"))
            .await?;
        Ok(parse_uid_fetch_messages(&lines))
    }

    async fn start_idle(&mut self) -> Result<String> {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        self.write_line(&format!("{tag} IDLE")).await?;
        let line = self.read_line().await?;
        if !line.starts_with('+') {
            bail!("IMAP IDLE expected continuation, got: {}", line);
        }
        Ok(tag)
    }

    async fn done_idle(&mut self, tag: &str) -> Result<()> {
        self.write_line("DONE").await?;
        tokio::time::timeout(Duration::from_secs(IMAP_IDLE_DONE_TIMEOUT_SECS), async {
            loop {
                let line = self.read_line().await?;
                if line.starts_with(&format!("{tag} ")) {
                    if line.contains(" OK") {
                        return Ok::<_, anyhow::Error>(());
                    }
                    bail!("IMAP IDLE DONE failed: {}", line);
                }
            }
        })
        .await
        .context("IMAP IDLE DONE timed out")?
    }

    pub(crate) async fn logout(&mut self) -> Result<()> {
        let _ = self.command("LOGOUT").await;
        Ok(())
    }
}

pub fn parse_uid_fetch_messages(lines: &[String]) -> Vec<ImapFetchedMessage> {
    let mut out = Vec::new();
    for line in lines {
        if !line.starts_with('*') || !line.contains(" FETCH ") {
            continue;
        }
        let Some(uid) = extract_imap_atom_after(line, "UID ") else {
            continue;
        };
        let flags = extract_imap_parenthesized_after(line, "FLAGS ")
            .map(|raw| raw.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        let raw = extract_imap_literal_or_quoted_body(line).unwrap_or_default();
        out.push(ImapFetchedMessage {
            uid,
            flags,
            raw: raw.into_bytes(),
        });
    }
    out
}

fn parse_uid_search_uids(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix("* SEARCH "))
        .flat_map(|line| line.split_whitespace())
        .filter(|uid| uid.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

fn parse_uid_number(uid: &str) -> Option<u64> {
    uid.parse().ok()
}

fn trailing_literal_len(line: &str) -> Option<usize> {
    let open = line.rfind('{')?;
    let len = line.get(open + 1..line.len().checked_sub(1)?)?;
    if !line.ends_with('}') || len.is_empty() || !len.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    len.parse().ok()
}

fn extract_imap_atom_after(line: &str, marker: &str) -> Option<String> {
    let start = line.find(marker)? + marker.len();
    let tail = &line[start..];
    let end = tail
        .find(|ch: char| ch.is_whitespace() || ch == ')' || ch == '(')
        .unwrap_or(tail.len());
    Some(tail[..end].to_string())
}

fn extract_imap_parenthesized_after(line: &str, marker: &str) -> Option<String> {
    let start = line.find(marker)? + marker.len();
    let tail = &line[start..];
    let open = tail.find('(')? + 1;
    let close = tail[open..].find(')')? + open;
    Some(tail[open..close].to_string())
}

fn extract_imap_literal_or_quoted_body(line: &str) -> Option<String> {
    if let Some(start) = line.find("BODY[] \"") {
        let tail = &line[start + "BODY[] \"".len()..];
        let end = tail.rfind('"')?;
        return Some(tail[..end].replace("\\\"", "\"").replace("\\\\", "\\"));
    }
    if let Some(start) = line.find("BODY[] {") {
        let len_start = start + "BODY[] {".len();
        let len_end = line[len_start..].find('}')? + len_start;
        let len: usize = line[len_start..len_end].parse().ok()?;
        let raw_start = line[len_end..].find("\r\n")? + len_end + 2;
        let raw_end = raw_start.checked_add(len)?;
        let bytes = line.as_bytes().get(raw_start..raw_end)?;
        return Some(String::from_utf8_lossy(bytes).to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_email_headers_and_snippet() {
        let raw = "Message-ID: <m1@example.com>\r\nSubject: Hello\r\n folded\r\nFrom: A <a@example.com>\r\nTo: b@example.com, c@example.com\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\n\r\nHello world";
        let headers = parse_email_headers(raw);
        assert_eq!(header_value(&headers, "subject").unwrap(), "Hello folded");
        assert_eq!(body_snippet(raw).unwrap(), "Hello world");
    }

    #[test]
    fn builds_provider_neutral_email_event() {
        let config = EmailImapIdleRuntimeConfig {
            endpoint: "imaps://imap.example.com:993".to_string(),
            host: "imap.example.com".to_string(),
            port: 993,
            use_tls: true,
            auth_method: ImapAuthMethod::Basic {
                username: "agent@example.com".to_string(),
                password: "secret".to_string(),
            },
            auth_profile: None,
            mailbox: "INBOX".to_string(),
            account: Some("primary".to_string()),
            initial_fetch_limit: 25,
        };
        let message = ImapFetchedMessage {
            uid: "42".to_string(),
            flags: vec!["\\Seen".to_string()],
            raw: b"Message-ID: <m1@example.com>\r\nSubject: Hi\r\nFrom: sender@example.com\r\n\r\nBody"
                .to_vec(),
        };
        let event = build_email_event(&config, &message, None);
        assert_eq!(event["type"], "email_event");
        assert_eq!(event["provider"], "imap");
        assert_eq!(event["message"]["uid"], "42");
        assert_eq!(event["message"]["subject"], "Hi");
        // Backward-compatible shape for messages without attachments.
        assert_eq!(event["message"]["attachments"], json!([]));
        assert_eq!(event["message"]["has_attachments"], false);
        assert_eq!(event["message"]["attachment_count"], 0);
        assert_eq!(event["raw"]["mime_truncated"], false);
        assert_eq!(event["reply_handle"]["message_id"], "<m1@example.com>");
    }

    #[test]
    fn builds_email_event_with_attachment_metadata_and_handles() {
        let config = EmailImapIdleRuntimeConfig {
            endpoint: "imaps://imap.example.com:993".to_string(),
            host: "imap.example.com".to_string(),
            port: 993,
            use_tls: true,
            auth_method: ImapAuthMethod::Basic {
                username: "agent@example.com".to_string(),
                password: "secret".to_string(),
            },
            auth_profile: Some("imap-primary".to_string()),
            mailbox: "INBOX".to_string(),
            account: Some("primary".to_string()),
            initial_fetch_limit: 25,
        };
        let raw = concat!(
            "Message-ID: <m42@example.com>\r\n",
            "Subject: Contract\r\n",
            "From: sender@example.com\r\n",
            "Content-Type: multipart/mixed; boundary=MIX\r\n",
            "\r\n",
            "--MIX\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Please review.\r\n",
            "--MIX\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "Content-Disposition: attachment;\r\n",
            " filename*=UTF-8''%E5%90%88%E5%90%8C.pdf\r\n",
            "\r\n",
            "JVBERi0xLjQK\r\n",
            "--MIX--\r\n"
        );
        let message = ImapFetchedMessage {
            uid: "42".to_string(),
            flags: Vec::new(),
            raw: raw.as_bytes().to_vec(),
        };
        let event = build_email_event(&config, &message, Some(3857529045));
        assert_eq!(event["message"]["has_attachments"], true);
        assert_eq!(event["message"]["attachment_count"], 1);
        let attachment = &event["message"]["attachments"][0];
        assert_eq!(attachment["id"], "2");
        assert_eq!(attachment["filename"], "合同.pdf");
        assert_eq!(attachment["content_type"], "application/pdf");
        assert_eq!(attachment["disposition"], "attachment");
        assert_eq!(attachment["size"], 9);
        let handle = &attachment["handle"];
        assert_eq!(handle["type"], "email_attachment");
        assert_eq!(handle["provider"], "imap");
        assert_eq!(handle["endpoint"], "imaps://imap.example.com:993");
        assert_eq!(handle["auth_profile"], "imap-primary");
        assert_eq!(handle["uidvalidity"], json!(3857529045u64));
        assert_eq!(handle["part"]["section"], "2");
    }

    #[test]
    fn parses_uidvalidity_from_select_response() {
        let lines = vec![
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen)".to_string(),
            "* OK [PERMANENTFLAGS (\\* \\Answered \\Flagged \\Deleted \\Seen)] Limited".to_string(),
            "* 3 EXISTS".to_string(),
            "* OK [UIDVALIDITY 3857529045] UIDs valid".to_string(),
            "A0003 OK [READ-WRITE] SELECT completed".to_string(),
        ];
        assert_eq!(parse_imap_uidvalidity(&lines), Some(3857529045));
        assert_eq!(parse_imap_uidvalidity(&[]), None);
    }

    #[test]
    fn normalizes_gmail_provider_messages_to_email_events() {
        let config = EmailProviderPollRuntimeConfig {
            endpoint: "https://gmail.googleapis.com/gmail/v1/users/me/messages".to_string(),
            provider: EmailProviderKind::Gmail,
            account: Some("gmail-primary".to_string()),
            mailbox: "INBOX".to_string(),
            auth_profile: None,
        };
        let raw = json!({
            "messages": [{
                "id": "msg-1",
                "threadId": "thread-1",
                "labelIds": ["INBOX", "UNREAD"],
                "snippet": "hello",
                "payload": {"headers": [
                    {"name": "Message-ID", "value": "<m1@example.com>"},
                    {"name": "Subject", "value": "Hi"},
                    {"name": "From", "value": "Sender <sender@example.com>"},
                    {"name": "To", "value": "agent@example.com"}
                ]}
            }]
        });

        let events = normalize_email_provider_items(&config, &raw).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "email_event");
        assert_eq!(events[0]["provider"], "gmail");
        assert_eq!(events[0]["account"], "gmail-primary");
        assert_eq!(events[0]["message"]["uid"], "msg-1");
        assert_eq!(events[0]["message"]["message_id"], "<m1@example.com>");
        assert_eq!(events[0]["message"]["subject"], "Hi");
        assert_eq!(events[0]["reply_handle"]["type"], "email_provider");
    }

    #[test]
    fn normalizes_graph_and_jmap_provider_messages_to_email_events() {
        let graph_config = EmailProviderPollRuntimeConfig {
            endpoint: "https://graph.microsoft.com/v1.0/me/messages".to_string(),
            provider: EmailProviderKind::Graph,
            account: None,
            mailbox: "Inbox".to_string(),
            auth_profile: None,
        };
        let graph_raw = json!({
            "value": [{
                "id": "graph-1",
                "conversationId": "conv-1",
                "internetMessageId": "<graph@example.com>",
                "subject": "Graph",
                "bodyPreview": "preview",
                "from": {"emailAddress": {"address": "sender@example.com"}},
                "toRecipients": [{"emailAddress": {"address": "agent@example.com"}}],
                "isRead": true
            }]
        });
        let graph_events = normalize_email_provider_items(&graph_config, &graph_raw).unwrap();
        assert_eq!(graph_events[0]["provider"], "graph");
        assert_eq!(graph_events[0]["message"]["uid"], "graph-1");
        assert_eq!(
            graph_events[0]["message"]["message_id"],
            "<graph@example.com>"
        );
        assert_eq!(graph_events[0]["message"]["flags"], json!(["read"]));

        let jmap_config = EmailProviderPollRuntimeConfig {
            endpoint: "https://api.fastmail.com/jmap/session".to_string(),
            provider: EmailProviderKind::Jmap,
            account: Some("jmap-primary".to_string()),
            mailbox: "Inbox".to_string(),
            auth_profile: None,
        };
        let jmap_raw = json!({
            "list": [{
                "id": "email-1",
                "messageId": "<jmap@example.com>",
                "subject": "JMAP",
                "preview": "preview",
                "from": [{"email": "sender@example.com"}],
                "to": [{"email": "agent@example.com"}],
                "keywords": {"$seen": true}
            }]
        });
        let jmap_events = normalize_email_provider_items(&jmap_config, &jmap_raw).unwrap();
        assert_eq!(jmap_events[0]["provider"], "jmap");
        assert_eq!(jmap_events[0]["message"]["uid"], "email-1");
        assert_eq!(
            jmap_events[0]["message"]["message_id"],
            "<jmap@example.com>"
        );
        assert_eq!(jmap_events[0]["message"]["flags"], json!(["$seen"]));
    }

    #[test]
    fn maps_gmail_attachment_metadata_from_payload_parts() {
        let config = EmailProviderPollRuntimeConfig {
            endpoint: "https://gmail.googleapis.com/gmail/v1/users/me/messages".to_string(),
            provider: EmailProviderKind::Gmail,
            account: Some("gmail-primary".to_string()),
            mailbox: "INBOX".to_string(),
            auth_profile: Some("gmail-primary".to_string()),
        };
        let raw = json!({
            "messages": [{
                "id": "msg-a1",
                "threadId": "thread-a1",
                "payload": {
                    "mimeType": "multipart/mixed",
                    "headers": [{"name": "Message-ID", "value": "<a1@example.com>"}],
                    "parts": [
                        {
                            "partId": "0",
                            "mimeType": "multipart/alternative",
                            "parts": [
                                {"partId": "0.0", "mimeType": "text/plain", "body": {"size": 12}},
                                {"partId": "0.1", "mimeType": "text/html", "body": {"size": 48}}
                            ]
                        },
                        {
                            "partId": "1",
                            "mimeType": "application/pdf",
                            "filename": "合同.pdf",
                            "headers": [
                                {"name": "Content-Disposition", "value": "attachment; filename=\"合同.pdf\""},
                                {"name": "Content-ID", "value": "<pdf@a1.example.com>"}
                            ],
                            "body": {"attachmentId": "ATT-1", "size": 102400}
                        },
                        {
                            "partId": "2",
                            "mimeType": "image/png",
                            "filename": "logo.png",
                            "headers": [
                                {"name": "Content-Disposition", "value": "inline"},
                                {"name": "Content-ID", "value": "<logo@a1.example.com>"}
                            ],
                            "body": {"size": 2048}
                        }
                    ]
                }
            }]
        });

        let events = normalize_email_provider_items(&config, &raw).unwrap();
        let message = &events[0]["message"];
        assert_eq!(message["has_attachments"], true);
        assert_eq!(message["attachment_count"], 2);
        let pdf = &message["attachments"][0];
        assert_eq!(pdf["id"], "1");
        assert_eq!(pdf["filename"], "合同.pdf");
        assert_eq!(pdf["content_type"], "application/pdf");
        assert_eq!(pdf["size"], 102400);
        assert_eq!(pdf["disposition"], "attachment");
        assert_eq!(pdf["content_id"], "<pdf@a1.example.com>");
        assert_eq!(pdf["handle"]["type"], "email_attachment");
        assert_eq!(pdf["handle"]["provider"], "gmail");
        assert_eq!(pdf["handle"]["uid"], "msg-a1");
        assert_eq!(pdf["handle"]["auth_profile"], "gmail-primary");
        assert_eq!(pdf["handle"]["part"], json!({"attachment_id": "ATT-1"}));
        let logo = &message["attachments"][1];
        assert_eq!(logo["id"], "2");
        assert_eq!(logo["disposition"], "inline");
        assert_eq!(logo["content_id"], "<logo@a1.example.com>");
        // No `body.attachmentId` on the inlined part: metadata-only entry.
        assert_eq!(logo["handle"], json!(null));
    }

    #[test]
    fn maps_graph_attachment_metadata_and_unexpanded_semantics() {
        let config = EmailProviderPollRuntimeConfig {
            endpoint: "https://graph.microsoft.com/v1.0/me/messages".to_string(),
            provider: EmailProviderKind::Graph,
            account: None,
            mailbox: "Inbox".to_string(),
            auth_profile: None,
        };
        let expanded = json!({
            "value": [{
                "id": "graph-a1",
                "internetMessageId": "<g1@example.com>",
                "hasAttachments": true,
                "attachments": [{
                    "@odata.type": "#microsoft.graph.fileAttachment",
                    "id": "graph-att-1",
                    "name": "report.pdf",
                    "contentType": "application/pdf",
                    "size": 20480,
                    "isInline": false,
                    "contentId": "att1@graph.example.com"
                }, {
                    "@odata.type": "#microsoft.graph.fileAttachment",
                    "name": "orphan.bin",
                    "contentType": "application/octet-stream",
                    "size": 8,
                    "isInline": false
                }]
            }]
        });
        let events = normalize_email_provider_items(&config, &expanded).unwrap();
        let message = &events[0]["message"];
        assert_eq!(message["has_attachments"], true);
        assert_eq!(message["attachment_count"], 2);
        let att = &message["attachments"][0];
        assert_eq!(att["id"], "graph-att-1");
        assert_eq!(att["filename"], "report.pdf");
        assert_eq!(att["content_type"], "application/pdf");
        assert_eq!(att["size"], 20480);
        assert_eq!(att["disposition"], "attachment");
        assert_eq!(att["content_id"], "att1@graph.example.com");
        assert_eq!(att["handle"]["provider"], "graph");
        assert_eq!(att["handle"]["uid"], "graph-a1");
        assert_eq!(
            att["handle"]["part"],
            json!({"attachment_id": "graph-att-1"})
        );
        // Attachment without `id`: metadata-only entry with a null handle.
        let orphan = &message["attachments"][1];
        assert_eq!(orphan["id"], json!(null));
        assert_eq!(orphan["handle"], json!(null));

        // Without `$expand=attachments` Graph lists messages with
        // `hasAttachments: true` but no attachment metadata: keep the
        // signal, expose an empty list, and mark the count unknown.
        let unexpanded = json!({
            "value": [{
                "id": "graph-a2",
                "internetMessageId": "<g2@example.com>",
                "hasAttachments": true
            }]
        });
        let events = normalize_email_provider_items(&config, &unexpanded).unwrap();
        let message = &events[0]["message"];
        assert_eq!(message["attachments"], json!([]));
        assert_eq!(message["has_attachments"], true);
        assert_eq!(message["attachment_count"], json!(null));
    }

    #[test]
    fn maps_jmap_attachment_metadata() {
        let config = EmailProviderPollRuntimeConfig {
            endpoint: "https://api.fastmail.com/jmap/session".to_string(),
            provider: EmailProviderKind::Jmap,
            account: Some("jmap-primary".to_string()),
            mailbox: "INBOX".to_string(),
            auth_profile: None,
        };
        let raw = json!({
            "list": [{
                "id": "Mailbox-42-email-7",
                "messageId": "<j1@example.com>",
                "hasAttachments": true,
                "attachments": [
                    {
                        "partId": "2",
                        "blobId": "B-att-1",
                        "type": "application/pdf",
                        "name": "doc.pdf",
                        "size": 4096,
                        "disposition": "attachment",
                        "cid": null
                    },
                    {
                        "partId": "3",
                        "blobId": "B-att-2",
                        "type": "image/png",
                        "name": null,
                        "size": 1024,
                        "disposition": {"inline": true},
                        "cid": "logo@j1.example.com"
                    }
                ]
            }]
        });
        let events = normalize_email_provider_items(&config, &raw).unwrap();
        let message = &events[0]["message"];
        assert_eq!(message["has_attachments"], true);
        assert_eq!(message["attachment_count"], 2);
        let pdf = &message["attachments"][0];
        assert_eq!(pdf["id"], "2");
        assert_eq!(pdf["filename"], "doc.pdf");
        assert_eq!(pdf["content_type"], "application/pdf");
        assert_eq!(pdf["size"], 4096);
        assert_eq!(pdf["disposition"], "attachment");
        assert_eq!(pdf["handle"]["provider"], "jmap");
        assert_eq!(pdf["handle"]["account"], "jmap-primary");
        assert_eq!(pdf["handle"]["uid"], "Mailbox-42-email-7");
        assert_eq!(pdf["handle"]["part"], json!({"blob_id": "B-att-1"}));
        let logo = &message["attachments"][1];
        assert_eq!(logo["id"], "3");
        assert_eq!(logo["filename"], json!(null));
        assert_eq!(logo["disposition"], "inline");
        assert_eq!(logo["content_id"], "logo@j1.example.com");
        assert_eq!(logo["handle"]["part"], json!({"blob_id": "B-att-2"}));
    }

    #[test]
    fn parses_single_line_uid_fetch() {
        let lines = vec![
            "* 1 FETCH (UID 42 FLAGS (\\Seen) BODY[] \"Subject: Hi\\r\\n\\r\\nBody\")".to_string(),
            "A0001 OK done".to_string(),
        ];
        let messages = parse_uid_fetch_messages(&lines);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].uid, "42");
        assert_eq!(messages[0].flags, vec!["\\Seen"]);
    }

    #[test]
    fn parses_uid_fetch_literal_with_embedded_newlines() {
        let raw = "Subject: Hi\r\n\r\nLine one\r\nLine two) end";
        let lines = vec![
            format!(
                "* 1 FETCH (UID 42 FLAGS (\\Seen) BODY[] {{{}}}\r\n{})",
                raw.len(),
                raw
            ),
            "A0001 OK done".to_string(),
        ];
        let messages = parse_uid_fetch_messages(&lines);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].uid, "42");
        assert_eq!(messages[0].raw, raw.as_bytes());
    }

    #[tokio::test]
    async fn imap_command_reads_literal_bytes_before_next_response_line() {
        let raw = "Subject: Hi\r\n\r\nLine one\r\nLine two";
        let (client, server) = tokio::io::duplex(2048);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0001 UID FETCH 42 (UID FLAGS BODY.PEEK[])\r\n");
            writer
                .write_all(
                    format!(
                        "* 1 FETCH (UID 42 FLAGS (\\Seen) BODY[] {{{}}}\r\n{})\r\nA0001 OK done\r\n",
                        raw.len(),
                        raw
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let messages = conn.fetch_uid_set("42").await.unwrap();
        server.await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].raw, raw.as_bytes());
    }

    #[tokio::test]
    async fn fetch_recent_searches_all_uids_and_fetches_last_limit() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();

            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0001 UID SEARCH ALL\r\n");
            writer
                .write_all(b"* SEARCH 1 2 42 43\r\nA0001 OK search done\r\n")
                .await
                .unwrap();

            command.clear();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0002 UID FETCH 42,43 (UID FLAGS BODY.PEEK[])\r\n");
            writer
                .write_all(
                    b"* 3 FETCH (UID 42 FLAGS () BODY[] \"Subject: Old\r\n\r\nOld\")\r\n* 4 FETCH (UID 43 FLAGS () BODY[] \"Subject: New\r\n\r\nNew\")\r\nA0002 OK fetch done\r\n",
                )
                .await
                .unwrap();
        });

        let messages = conn.fetch_recent(2).await.unwrap();
        server.await.unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.uid.as_str())
                .collect::<Vec<_>>(),
            vec!["42", "43"]
        );
    }

    #[derive(Default)]
    struct RecordingRecorder {
        events: Vec<(String, String)>,
        statuses: Vec<(String, Option<String>, bool)>,
    }

    #[async_trait::async_trait]
    impl SubscriptionEventRecorder for RecordingRecorder {
        async fn emit(
            &mut self,
            source_kind: &str,
            event_kind: &str,
            _data: Option<Value>,
            _meta: Option<Value>,
        ) -> Result<()> {
            self.events
                .push((source_kind.to_string(), event_kind.to_string()));
            Ok(())
        }

        async fn update_status(
            &mut self,
            status: Option<&str>,
            last_error: Option<String>,
            increment_reconnect: bool,
        ) -> Result<()> {
            self.statuses.push((
                status.unwrap_or("<none>").to_string(),
                last_error,
                increment_reconnect,
            ));
            Ok(())
        }
    }

    fn imap_idle_test_config(initial_fetch_limit: usize) -> EmailImapIdleRuntimeConfig {
        EmailImapIdleRuntimeConfig {
            endpoint: "imaps://imap.example.com:993".to_string(),
            host: "imap.example.com".to_string(),
            port: 993,
            use_tls: true,
            auth_method: ImapAuthMethod::Basic {
                username: "agent@example.com".to_string(),
                password: "secret".to_string(),
            },
            auth_profile: None,
            mailbox: "INBOX".to_string(),
            account: None,
            initial_fetch_limit,
        }
    }

    #[tokio::test]
    async fn latest_uid_returns_max_search_uid_without_fetching() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0001 UID SEARCH ALL\r\n");
            writer
                .write_all(b"* SEARCH 7 42 43\r\nA0001 OK search done\r\n")
                .await
                .unwrap();
        });

        let latest = conn.latest_uid().await.unwrap();
        server.await.unwrap();
        assert_eq!(latest, Some(43));
    }

    #[tokio::test]
    async fn emit_recent_messages_with_zero_limit_baselines_without_emitting() {
        let config = imap_idle_test_config(0);
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0001 UID SEARCH ALL\r\n");
            writer
                .write_all(b"* SEARCH 7 42 43\r\nA0001 OK search done\r\n")
                .await
                .unwrap();
            // The connection must not receive a UID FETCH command afterwards:
            // dropping the writer closes the pipe and any further read fails.
        });

        let mut recorder = RecordingRecorder::default();
        let last_uid = emit_recent_messages(&config, &mut conn, &mut recorder, None)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(last_uid, Some(43));
        assert!(recorder.events.is_empty());
    }

    #[tokio::test]
    async fn emit_recent_messages_with_positive_limit_fetches_and_emits() {
        let config = imap_idle_test_config(25);
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let raw =
            "Message-ID: <m43@example.com>\r\nSubject: New\r\nFrom: sender@example.com\r\n\r\nBody";
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0001 UID SEARCH ALL\r\n");
            writer
                .write_all(b"* SEARCH 42 43\r\nA0001 OK search done\r\n")
                .await
                .unwrap();

            command.clear();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "A0002 UID FETCH 42,43 (UID FLAGS BODY.PEEK[])\r\n");
            writer
                .write_all(
                    format!(
                        "* 1 FETCH (UID 42 FLAGS () BODY[] \"Subject: Old\\r\\n\\r\\nOld\")\r\n* 2 FETCH (UID 43 FLAGS () BODY[] {{{}}}\r\n{})\r\nA0002 OK fetch done\r\n",
                        raw.len(),
                        raw
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let mut recorder = RecordingRecorder::default();
        let last_uid = emit_recent_messages(&config, &mut conn, &mut recorder, None)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(last_uid, Some(43));
        assert_eq!(recorder.events.len(), 2);
        assert!(recorder
            .events
            .iter()
            .all(|(kind, _)| kind == "email_imap_idle"));
    }

    fn subscribe_request_for_resolve(endpoint: &str) -> crate::daemon::SubscribeStartRequest {
        serde_json::from_value(serde_json::json!({
            "request_id": "test-request",
            "endpoint": endpoint,
            "sink": "memory:",
            "mode": "stream",
            "options": {
                "auth": null,
                "no_cache": false,
                "refresh_schema": false,
                "schema_url": null,
                "link_name": null,
                "schema_mapping_file": null,
            },
        }))
        .unwrap()
    }

    #[test]
    fn xoauth2_sasl_string_has_expected_shape() {
        let value = build_xoauth2_sasl_string("user@example.com", "token1");
        assert_eq!(value, "user=user@example.com\x01auth=Bearer token1\x01\x01");
    }

    #[test]
    fn decode_base64_json_challenge_accepts_json_only() {
        let payload = base64::engine::general_purpose::STANDARD
            .encode(r#"{"status":"401","schemes":"Bearer,XOAUTH2"}"#);
        assert!(decode_base64_json_challenge(&payload).is_some());
        let not_json = base64::engine::general_purpose::STANDARD.encode("plain text");
        assert!(decode_base64_json_challenge(&not_json).is_none());
        assert!(decode_base64_json_challenge("!!!not-base64!!!").is_none());
    }

    #[tokio::test]
    async fn imap_authenticate_xoauth2_accepts_tagged_ok() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert!(command.starts_with("A0001 AUTHENTICATE XOAUTH2 "));
            assert!(command.ends_with("\r\n"));
            writer
                .write_all(b"A0001 OK Authenticated.\r\n")
                .await
                .unwrap();
        });

        conn.authenticate_xoauth2("user@example.com", "token1")
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn imap_authenticate_xoauth2_aborts_challenge_and_maps_actionable_error() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert!(command.starts_with("A0001 AUTHENTICATE XOAUTH2 "));
            let challenge = base64::engine::general_purpose::STANDARD.encode(
                r#"{"status":"401","schemes":"Bearer,XOAUTH2","scope":"https://outlook.office.com/IMAP.AccessAsUser.All"}"#,
            );
            writer
                .write_all(format!("+ {challenge}\r\n").as_bytes())
                .await
                .unwrap();
            // After the SASL abort (`*` per RFC 3501), the server finishes with NO.
            let mut abort = String::new();
            reader.read_line(&mut abort).await.unwrap();
            assert_eq!(abort, "*\r\n");
            writer
                .write_all(b"A0001 NO AUTHENTICATE failed.\r\n")
                .await
                .unwrap();
        });

        let err = conn
            .authenticate_xoauth2("user@hotmail.com", "stale-token")
            .await
            .unwrap_err();
        server.await.unwrap();
        let message = err.to_string();
        assert!(message.contains(IMAP_XOAUTH2_REJECTED_MARKER));
        assert!(message.contains("IMAP.AccessAsUser.All"));
        assert!(message.contains("auth oauth login"));
        assert!(message.contains("AUTHENTICATE failed"));
        // The base64 JSON challenge is decoded into the error detail.
        assert!(message.contains("\"status\":\"401\""));
        assert!(is_imap_permanent_auth_failure(&message));
    }

    #[tokio::test]
    async fn imap_authenticate_xoauth2_maps_direct_no_rejection() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            writer
                .write_all(b"A0001 NO AUTHENTICATE failed.\r\n")
                .await
                .unwrap();
        });

        let err = conn
            .authenticate_xoauth2("user@hotmail.com", "token1")
            .await
            .unwrap_err();
        server.await.unwrap();
        let message = err.to_string();
        assert!(message.contains(IMAP_XOAUTH2_REJECTED_MARKER));
        assert!(is_imap_permanent_auth_failure(&message));
    }

    fn oauth_profile_for_imap(username_field: Option<(&str, &str)>) -> Profile {
        let mut profile = Profile::new(String::new(), crate::auth::AuthType::OAuth);
        profile.name = Some("outlook-imap".to_string());
        let oauth = crate::auth::OAuthProfile {
            access_token: Some("access-token-1".to_string()),
            scopes: vec!["https://outlook.office.com/IMAP.AccessAsUser.All".to_string()],
            ..Default::default()
        };
        profile.oauth = Some(oauth);
        if let Some((name, value)) = username_field {
            profile
                .set_field_source(
                    name.to_string(),
                    crate::auth::SecretSource::Literal {
                        value: value.to_string(),
                    },
                )
                .unwrap();
        }
        profile
    }

    #[test]
    fn resolve_config_uses_xoauth2_for_oauth_profiles() {
        let mut request = subscribe_request_for_resolve("imaps://outlook.office365.com:993");
        request.options.auth = Some("outlook-imap".to_string());
        let profile = oauth_profile_for_imap(Some(("email", "user@hotmail.com")));
        let config = resolve_email_imap_idle_runtime_config(&request, &profile).unwrap();
        assert_eq!(
            config.auth_method,
            ImapAuthMethod::Xoauth2 {
                username: "user@hotmail.com".to_string(),
                token: "access-token-1".to_string(),
            }
        );
        // Without a username field the account arg and finally the profile
        // name act as the mailbox address.
        let profile = oauth_profile_for_imap(None);
        let config = resolve_email_imap_idle_runtime_config(&request, &profile).unwrap();
        assert_eq!(
            config.auth_method,
            ImapAuthMethod::Xoauth2 {
                username: "outlook-imap".to_string(),
                token: "access-token-1".to_string(),
            }
        );
    }

    #[test]
    fn resolve_config_keeps_basic_auth_for_password_profiles() {
        let request = subscribe_request_for_resolve("imaps://imap.example.com:993");
        let mut profile = Profile::new(String::new(), crate::auth::AuthType::Basic);
        profile
            .set_field_source(
                "username".to_string(),
                crate::auth::SecretSource::Literal {
                    value: "agent@example.com".to_string(),
                },
            )
            .unwrap();
        profile
            .set_field_source(
                "password".to_string(),
                crate::auth::SecretSource::Literal {
                    value: "secret".to_string(),
                },
            )
            .unwrap();
        let config = resolve_email_imap_idle_runtime_config(&request, &profile).unwrap();
        assert_eq!(
            config.auth_method,
            ImapAuthMethod::Basic {
                username: "agent@example.com".to_string(),
                password: "secret".to_string(),
            }
        );
    }

    fn basic_auth_disabled_server_script() -> &'static [u8] {
        b"* OK Microsoft Exchange IMAP4 service ready.\r\n"
    }

    #[tokio::test]
    async fn imap_login_maps_m365_basic_auth_disabled_to_actionable_error() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert!(command.starts_with("A0001 LOGIN "));
            writer
                .write_all(b"A0001 NO Basic authentication is disabled.\r\n")
                .await
                .unwrap();
            writer
                .write_all(b"* BYE Microsoft Exchange Server IMAP4 server signing off.\r\n")
                .await
                .unwrap();
        });

        let err = imap_login(&mut conn, "jolestar@hotmail.com", "secret")
            .await
            .unwrap_err();
        server.await.unwrap();
        let message = err.to_string();
        assert!(is_imap_basic_auth_disabled_message(&message));
        assert!(message.contains("outlook.com/hotmail.com/live.com"));
        assert!(message.contains("XOAUTH2"));
        assert!(message.contains("Basic authentication is disabled"));
    }

    #[tokio::test]
    async fn imap_login_keeps_generic_failure_for_other_rejections() {
        let (client, server) = tokio::io::duplex(4096);
        let mut conn = ImapConnection::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut command = String::new();
            let _ = reader.read_line(&mut command).await;
            writer
                .write_all(b"A0001 NO LOGIN failed.\r\n")
                .await
                .unwrap();
        });

        let err = imap_login(&mut conn, "user@example.com", "wrong")
            .await
            .unwrap_err();
        server.await.unwrap();
        let message = err.to_string();
        assert!(!is_imap_basic_auth_disabled_message(&message));
        assert!(message.contains("IMAP command failed"));
    }

    #[tokio::test]
    async fn email_imap_idle_stops_reconnecting_on_basic_auth_disabled() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let config = EmailImapIdleRuntimeConfig {
            endpoint: "imaps://outlook.office365.com:993".to_string(),
            host: "outlook.office365.com".to_string(),
            port: 993,
            use_tls: true,
            auth_method: ImapAuthMethod::Basic {
                username: "jolestar@hotmail.com".to_string(),
                password: "secret".to_string(),
            },
            auth_profile: Some("email-hotmail".to_string()),
            mailbox: "INBOX".to_string(),
            account: Some("primary".to_string()),
            initial_fetch_limit: 25,
        };
        let mut recorder = RecordingRecorder::default();
        let (_stop_tx, mut stop_rx) = watch::channel(false);
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_connector = attempts.clone();
        let connector = move |_config: EmailImapIdleRuntimeConfig| {
            let attempts = attempts_for_connector.clone();
            Box::pin(async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                let (client, server) = tokio::io::duplex(4096);
                tokio::spawn(async move {
                    let (reader, mut writer) = tokio::io::split(server);
                    let mut reader = BufReader::new(reader);
                    writer
                        .write_all(basic_auth_disabled_server_script())
                        .await
                        .unwrap();
                    // Hold the server side open until the client LOGIN has
                    // been consumed: dropping it early turns the client write
                    // into a broken pipe, which is not a permanent rejection.
                    let mut command = String::new();
                    reader.read_line(&mut command).await.unwrap();
                    assert!(command.starts_with("A0001 LOGIN "));
                    writer
                        .write_all(b"A0001 NO Basic authentication is disabled.\r\n")
                        .await
                        .unwrap();
                });
                Ok(ImapConnection::new(Box::new(client)))
            }) as BoxFutureResult<ImapConnection>
        };

        let result = run_email_imap_idle_subscription_with_connector(
            config,
            &mut recorder,
            &mut stop_rx,
            connector,
        )
        .await;
        let message = result.unwrap_err().to_string();
        assert!(is_imap_basic_auth_disabled_message(&message));
        // Exactly one connection attempt: permanent rejections must not retry.
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(recorder.statuses.iter().any(|(status, error, inc)| {
            status == "failed"
                && error
                    .as_deref()
                    .is_some_and(|message| message.contains("outlook.com"))
                && !inc
        }));
        assert!(!recorder
            .statuses
            .iter()
            .any(|(status, _, _)| status == "reconnecting"));
        assert!(!recorder.events.iter().any(|(_, kind)| kind == "reconnect"));
    }
}
