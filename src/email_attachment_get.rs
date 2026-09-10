//! Lazy attachment retrieval for email events.
//!
//! `uxc email attachment get --handle <json>` consumes the opaque
//! `email_attachment` handles stamped on `email_event` messages and fetches
//! the referenced attachment content from IMAP, Gmail, Microsoft Graph, or
//! JMAP. Handles never carry credentials; they reference an auth profile by
//! name, which is re-resolved from the local auth store at retrieval time.
//!
//! Content is always written to a file (`--output` or a cache path under
//! `~/.uxc/email-attachments/`); it is never inlined into the JSON envelope.

use anyhow::{Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use url::Url;

use crate::auth::{
    auth_base_dir, resolve_profile_request_auth_with_context, AuthRequestContext, Profile, Profiles,
};
use crate::email_attachment::{
    decode_part_body_bytes, parse_part_headers, part_content_type, part_filename,
};
use crate::error::StructuredError;
use crate::subscription_email::{
    connect_imap, quote_imap_string, EmailImapIdleRuntimeConfig, ImapConnection, ImapSectionLiteral,
};

/// Default attachment size guard for `--max-bytes` (25 MiB).
pub const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
const ATTACHMENT_CACHE_DIR: &str = "email-attachments";
const JMAP_MAIL_CAPABILITY: &str = "urn:ietf:params:jmap:mail";

/// CLI request for lazy attachment retrieval.
#[derive(Debug, Clone)]
pub struct EmailAttachmentGetRequest {
    /// Raw JSON handle (or `@path` to read the handle from a file).
    pub handle: String,
    /// Auth profile override; takes precedence over the handle reference.
    pub profile: Option<String>,
    /// Explicit output path; defaults to a cache path.
    pub output: Option<String>,
    /// Reject attachments larger than this many bytes; 0 disables the limit.
    pub max_bytes: u64,
}

/// Parsed and validated `email_attachment` handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailAttachmentHandle {
    pub provider: EmailAttachmentProvider,
    /// Credential-free provider endpoint used to reach the message.
    pub endpoint: String,
    pub account: Option<String>,
    pub mailbox: Option<String>,
    pub message_id: Option<String>,
    pub uid: Option<String>,
    pub uidvalidity: Option<u64>,
    pub auth_profile: Option<String>,
    pub part: AttachmentPart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailAttachmentProvider {
    Imap,
    Gmail,
    Graph,
    Jmap,
}

impl EmailAttachmentProvider {
    fn as_str(self) -> &'static str {
        match self {
            EmailAttachmentProvider::Imap => "imap",
            EmailAttachmentProvider::Gmail => "gmail",
            EmailAttachmentProvider::Graph => "graph",
            EmailAttachmentProvider::Jmap => "jmap",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentPart {
    /// IMAP `BODY[<section>]` section number.
    Section(String),
    /// Gmail/Graph attachment id.
    AttachmentId(String),
    /// JMAP blob id.
    BlobId(String),
}

impl AttachmentPart {
    fn id(&self) -> &str {
        match self {
            AttachmentPart::Section(value)
            | AttachmentPart::AttachmentId(value)
            | AttachmentPart::BlobId(value) => value,
        }
    }
}

/// Successful retrieval result (mirrors the JSON envelope data payload).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EmailAttachmentGetResult {
    pub provider: String,
    pub account: Option<String>,
    pub mailbox: Option<String>,
    pub message_id: Option<String>,
    pub uid: Option<String>,
    pub attachment_id: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub saved_path: String,
}

/// Parse a raw handle argument (`@file` or inline JSON) into a validated
/// [`EmailAttachmentHandle`].
pub fn parse_attachment_handle(raw: &str) -> Result<EmailAttachmentHandle> {
    let json_text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).map_err(|err| {
            invalid_handle(&format!(
                "failed to read attachment handle file '{path}': {err}"
            ))
        })?
    } else {
        raw.to_string()
    };
    let value: Value = serde_json::from_str(&json_text)
        .map_err(|err| invalid_handle(&format!("attachment handle is not valid JSON: {err}")))?;
    if value.get("type").and_then(Value::as_str) != Some("email_attachment") {
        return Err(invalid_handle(
            "attachment handle requires type='email_attachment'",
        ));
    }
    let provider = match value.get("provider").and_then(Value::as_str) {
        Some("imap") => EmailAttachmentProvider::Imap,
        Some("gmail") => EmailAttachmentProvider::Gmail,
        Some("graph") => EmailAttachmentProvider::Graph,
        Some("jmap") => EmailAttachmentProvider::Jmap,
        Some(other) => {
            return Err(invalid_handle(&format!(
                "unsupported attachment handle provider '{}'",
                other
            )))
        }
        None => {
            return Err(invalid_handle(
                "attachment handle requires a provider field",
            ))
        }
    };
    let endpoint = value
        .get("endpoint")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_handle("attachment handle requires an endpoint field"))?
        .to_string();
    let part_object = value.get("part").ok_or_else(|| {
        invalid_handle("attachment handle requires a part field (section/attachment_id/blob_id)")
    })?;
    let part = match provider {
        EmailAttachmentProvider::Imap => {
            let section = part_object
                .get("section")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_handle("IMAP attachment handle requires part.section"))?;
            if !valid_imap_section(section) {
                return Err(invalid_handle(&format!(
                    "IMAP attachment section '{}' is not a valid RFC 3501 section path",
                    section
                )));
            }
            AttachmentPart::Section(section.to_string())
        }
        EmailAttachmentProvider::Gmail | EmailAttachmentProvider::Graph => {
            let id = part_object
                .get("attachment_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    invalid_handle("gmail/graph attachment handle requires part.attachment_id")
                })?;
            if !valid_url_path_segment(id) {
                return Err(invalid_handle(
                    "gmail/graph attachment id contains characters not allowed in a URL path",
                ));
            }
            AttachmentPart::AttachmentId(id.to_string())
        }
        EmailAttachmentProvider::Jmap => {
            let id = part_object
                .get("blob_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid_handle("JMAP attachment handle requires part.blob_id"))?;
            if !valid_url_path_segment(id) {
                return Err(invalid_handle(
                    "JMAP blob id contains characters not allowed in a URL path",
                ));
            }
            AttachmentPart::BlobId(id.to_string())
        }
    };
    let uid = value
        .get("uid")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    if matches!(
        provider,
        EmailAttachmentProvider::Gmail | EmailAttachmentProvider::Graph
    ) && uid.is_none()
    {
        return Err(invalid_handle(
            "gmail/graph attachment handle requires a uid field",
        ));
    }
    if provider == EmailAttachmentProvider::Imap {
        let Some(uid) = uid.as_deref() else {
            return Err(invalid_handle(
                "IMAP attachment handle requires a uid field",
            ));
        };
        if !uid.chars().all(|ch| ch.is_ascii_digit()) || uid.is_empty() {
            return Err(invalid_handle("IMAP attachment uid must be decimal digits"));
        }
    }
    Ok(EmailAttachmentHandle {
        provider,
        endpoint,
        account: string_field(&value, "account"),
        mailbox: string_field(&value, "mailbox"),
        message_id: string_field(&value, "message_id"),
        uid,
        uidvalidity: value.get("uidvalidity").and_then(Value::as_u64),
        auth_profile: string_field(&value, "auth_profile"),
        part,
    })
}

/// Retrieve an attachment using the production IMAP connector.
pub async fn get_email_attachment(
    request: &EmailAttachmentGetRequest,
) -> Result<EmailAttachmentGetResult> {
    get_email_attachment_with(request, connect_imap).await
}

/// Retrieve an attachment with an injectable IMAP connector (tests supply a
/// duplex-backed connector).
pub async fn get_email_attachment_with<F>(
    request: &EmailAttachmentGetRequest,
    imap_connector: F,
) -> Result<EmailAttachmentGetResult>
where
    F: Fn(EmailImapIdleRuntimeConfig) -> crate::subscription_email::BoxFutureResult<ImapConnection>,
{
    let handle = parse_attachment_handle(&request.handle)?;
    let fetched = match handle.provider {
        EmailAttachmentProvider::Imap => {
            fetch_imap_attachment(&handle, request, imap_connector).await?
        }
        EmailAttachmentProvider::Gmail => fetch_gmail_attachment(&handle, request).await?,
        EmailAttachmentProvider::Graph => fetch_graph_attachment(&handle, request).await?,
        EmailAttachmentProvider::Jmap => fetch_jmap_attachment(&handle, request).await?,
    };
    let bytes = fetched.bytes;
    let max_bytes = if request.max_bytes == 0 {
        u64::MAX
    } else {
        request.max_bytes
    };
    if bytes.len() as u64 > max_bytes {
        return Err(structured(
            "size_limit_exceeded",
            format!(
                "attachment is {} bytes which exceeds the --max-bytes limit of {}",
                bytes.len(),
                request.max_bytes
            ),
            Some(serde_json::json!({
                "size_bytes": bytes.len(),
                "max_bytes": request.max_bytes,
            })),
        ));
    }
    let saved_path = write_attachment(&handle, &bytes, request.output.as_deref())?;
    let sha256 = Sha256::digest(&bytes);
    Ok(EmailAttachmentGetResult {
        provider: handle.provider.as_str().to_string(),
        account: handle.account.clone(),
        mailbox: handle.mailbox.clone(),
        message_id: handle.message_id.clone(),
        uid: handle.uid.clone(),
        attachment_id: handle.part.id().to_string(),
        filename: fetched.filename,
        content_type: fetched.content_type,
        size_bytes: bytes.len() as u64,
        sha256: sha256.iter().map(|b| format!("{:02x}", b)).collect(),
        saved_path,
    })
}

/// Attachment bytes plus any provider-known metadata for the result payload.
struct FetchedAttachment {
    bytes: Vec<u8>,
    filename: Option<String>,
    content_type: Option<String>,
}

fn fetched(bytes: Vec<u8>) -> FetchedAttachment {
    FetchedAttachment {
        bytes,
        filename: None,
        content_type: None,
    }
}

async fn fetch_imap_attachment<F>(
    handle: &EmailAttachmentHandle,
    request: &EmailAttachmentGetRequest,
    connector: F,
) -> Result<FetchedAttachment>
where
    F: Fn(EmailImapIdleRuntimeConfig) -> crate::subscription_email::BoxFutureResult<ImapConnection>,
{
    let profile_name = required_profile_name(handle, request)?;
    let profile = load_profile(&profile_name)?;
    let mut profile = profile;
    let auth_method = if profile.auth_type == crate::auth::AuthType::OAuth {
        refresh_oauth_profile_for_retrieval(&mut profile).await?;
        let username = resolve_first_profile_field(&profile, &["username", "user", "email"])
            .or_else(|| profile.name.clone())
            .ok_or_else(|| {
                structured(
                    "auth_profile_missing",
                    format!(
                        "auth profile '{}' has no username/user/email field for IMAP retrieval",
                        profile_name
                    ),
                    None,
                )
            })?;
        let token = profile
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.access_token.clone())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                structured(
                    "auth_profile_missing",
                    format!(
                        "auth profile '{}' has no OAuth access token for IMAP retrieval",
                        profile_name
                    ),
                    None,
                )
            })?;
        crate::subscription_email::ImapAuthMethod::Xoauth2 { username, token }
    } else {
        let username = resolve_first_profile_field(&profile, &["username", "user", "email"])
            .ok_or_else(|| {
                structured(
                    "auth_profile_missing",
                    format!(
                        "auth profile '{}' has no username/user/email field for IMAP retrieval",
                        profile_name
                    ),
                    None,
                )
            })?;
        let password =
            resolve_first_profile_field(&profile, &["password", "app_password", "secret"])
                .ok_or_else(|| {
                    structured(
                        "auth_profile_missing",
                        format!(
                            "auth profile '{}' has no password/app_password/secret field for IMAP retrieval",
                            profile_name
                        ),
                        None,
                    )
                })?;
        crate::subscription_email::ImapAuthMethod::Basic { username, password }
    };
    let url = Url::parse(&handle.endpoint).map_err(|err| {
        invalid_handle(&format!(
            "invalid IMAP endpoint '{}': {}",
            handle.endpoint, err
        ))
    })?;
    let use_tls = match url.scheme() {
        "imaps" => true,
        "imap" => false,
        other => {
            return Err(invalid_handle(&format!(
                "IMAP attachment endpoint must be imap:// or imaps://, got '{}'",
                other
            )))
        }
    };
    let host = url
        .host_str()
        .ok_or_else(|| invalid_handle("IMAP attachment endpoint missing host"))?
        .to_string();
    let port = url.port().unwrap_or(if use_tls { 993 } else { 143 });
    let mailbox = handle
        .mailbox
        .clone()
        .unwrap_or_else(|| "INBOX".to_string());
    let config = EmailImapIdleRuntimeConfig {
        endpoint: handle.endpoint.clone(),
        host,
        port,
        use_tls,
        auth_method: auth_method.clone(),
        mailbox: mailbox.clone(),
        account: handle.account.clone(),
        auth_profile: Some(profile_name),
        initial_fetch_limit: 0,
        source_ref: None,
    };
    let mut conn = connector(config)
        .await
        .map_err(|err| provider_request_failed(format!("IMAP connect failed: {err}")))?;
    let fetch_limit = if request.max_bytes == 0 {
        usize::MAX
    } else {
        request.max_bytes.min(usize::MAX as u64) as usize
    };
    let attachment =
        run_imap_retrieval(&mut conn, handle, &auth_method, &mailbox, fetch_limit).await;
    let _ = conn.logout().await;
    attachment
}

/// Refresh the OAuth access token (when stale) before a one-shot retrieval
/// and persist it so subsequent retrievals reuse the fresh token.
async fn refresh_oauth_profile_for_retrieval(profile: &mut crate::auth::Profile) -> Result<()> {
    let client = reqwest::Client::new();
    let refreshed = crate::auth::refresh_effective_auth_profile(
        profile,
        &client,
        false,
        crate::subscription_email::IMAP_XOAUTH2_REFRESH_SKEW_SECS,
        None,
    )
    .await
    .map_err(|err| {
        structured(
            "auth_failed",
            format!("OAuth token refresh failed: {err}"),
            None,
        )
    })?;
    if refreshed {
        crate::auth::persist_profile_if_named(profile)?;
    }
    Ok(())
}

async fn run_imap_retrieval(
    conn: &mut ImapConnection,
    handle: &EmailAttachmentHandle,
    auth_method: &crate::subscription_email::ImapAuthMethod,
    mailbox: &str,
    fetch_limit: usize,
) -> Result<FetchedAttachment> {
    conn.expect_greeting()
        .await
        .map_err(|err| provider_request_failed(format!("IMAP greeting failed: {err}")))?;
    let auth_result = match auth_method {
        crate::subscription_email::ImapAuthMethod::Basic { username, password } => conn
            .command_ok(&format!(
                "LOGIN {} {}",
                quote_imap_string(username),
                quote_imap_string(password)
            ))
            .await
            .map(|_| ()),
        crate::subscription_email::ImapAuthMethod::Xoauth2 { username, token } => {
            conn.authenticate_xoauth2(username, token).await
        }
    };
    if let Err(err) = auth_result {
        return Err(structured(
            "auth_failed",
            format!("IMAP auth failed: {err}"),
            None,
        ));
    }
    let select_lines = conn
        .command_ok(&format!("SELECT {}", quote_imap_string(mailbox)))
        .await
        .map_err(|err| provider_request_failed(format!("IMAP SELECT failed: {err}")))?;
    if let (Some(expected), Some(current)) = (
        handle.uidvalidity,
        parse_imap_uidvalidity_from_select(&select_lines),
    ) {
        if expected != current {
            return Err(structured(
                "uid_invalid",
                format!(
                    "mailbox UIDVALIDITY changed from {} to {}; the attachment handle is stale",
                    expected, current
                ),
                Some(serde_json::json!({
                    "expected_uidvalidity": expected,
                    "current_uidvalidity": current,
                })),
            ));
        }
    }
    let uid = handle.uid.as_deref().unwrap_or_default();
    let AttachmentPart::Section(section) = &handle.part else {
        return Err(invalid_handle(
            "IMAP attachment handle requires part.section",
        ));
    };
    let mime = conn
        .fetch_body_section_bytes(uid, &format!("{section}.MIME"), fetch_limit)
        .await
        .map_err(|err| provider_request_failed(format!("IMAP fetch failed: {err}")))?;
    let ImapSectionLiteral::Bytes(mime_bytes) = mime else {
        return Err(structured(
            "message_not_found",
            format!("IMAP message with UID {} no longer exists", uid),
            None,
        ));
    };
    let body = conn
        .fetch_body_section_bytes(uid, section, fetch_limit)
        .await
        .map_err(|err| provider_request_failed(format!("IMAP fetch failed: {err}")))?;
    let ImapSectionLiteral::Bytes(body_bytes) = body else {
        return Err(structured(
            "attachment_not_found",
            format!(
                "IMAP section '{}' of message UID {} is no longer available",
                section, uid
            ),
            None,
        ));
    };
    let headers = parse_part_headers(&String::from_utf8_lossy(&mime_bytes));
    let filename = part_filename(&headers).filter(|name| !name.is_empty());
    let content_type = Some(part_content_type(&headers));
    Ok(FetchedAttachment {
        bytes: decode_part_body_bytes(&headers, &body_bytes),
        filename,
        content_type,
    })
}

async fn fetch_gmail_attachment(
    handle: &EmailAttachmentHandle,
    request: &EmailAttachmentGetRequest,
) -> Result<FetchedAttachment> {
    let profile = load_profile(&required_profile_name(handle, request)?)?;
    let AttachmentPart::AttachmentId(attachment_id) = &handle.part else {
        return Err(invalid_handle(
            "gmail attachment handle requires part.attachment_id",
        ));
    };
    let uid = handle.uid.as_deref().unwrap_or_default();
    let url = format!(
        "{}/{}/attachments/{}",
        handle.endpoint.trim_end_matches('/'),
        encode_path_segment(uid),
        encode_path_segment(attachment_id)
    );
    let body = provider_get_json(&profile, &url).await?;
    let data = body.get("data").and_then(Value::as_str).ok_or_else(|| {
        structured(
            "attachment_not_found",
            format!(
                "gmail attachment '{}' has no downloadable data",
                attachment_id
            ),
            None,
        )
    })?;
    let bytes = decode_gmail_base64url(data).map_err(|err| {
        structured(
            "provider_request_failed",
            format!("gmail attachment data could not be decoded: {err}"),
            None,
        )
    })?;
    Ok(FetchedAttachment {
        filename: body
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_type: body
            .get("mimeType")
            .and_then(Value::as_str)
            .map(str::to_string),
        bytes,
    })
}

async fn fetch_graph_attachment(
    handle: &EmailAttachmentHandle,
    request: &EmailAttachmentGetRequest,
) -> Result<FetchedAttachment> {
    let profile = load_profile(&required_profile_name(handle, request)?)?;
    let AttachmentPart::AttachmentId(attachment_id) = &handle.part else {
        return Err(invalid_handle(
            "graph attachment handle requires part.attachment_id",
        ));
    };
    let uid = handle.uid.as_deref().unwrap_or_default();
    let url = format!(
        "{}/{}/attachments/{}/$value",
        handle.endpoint.trim_end_matches('/'),
        encode_path_segment(uid),
        encode_path_segment(attachment_id)
    );
    let bytes = provider_get_bytes(&profile, &url).await?;
    Ok(fetched(bytes))
}

async fn fetch_jmap_attachment(
    handle: &EmailAttachmentHandle,
    request: &EmailAttachmentGetRequest,
) -> Result<FetchedAttachment> {
    let profile = load_profile(&required_profile_name(handle, request)?)?;
    let AttachmentPart::BlobId(blob_id) = &handle.part else {
        return Err(invalid_handle(
            "JMAP attachment handle requires part.blob_id",
        ));
    };
    let endpoint = Url::parse(&handle.endpoint).map_err(|err| {
        invalid_handle(&format!(
            "invalid JMAP endpoint '{}': {}",
            handle.endpoint, err
        ))
    })?;
    let session_url = format!(
        "{}://{}/.well-known/jmap",
        endpoint.scheme(),
        endpoint
            .host_str()
            .ok_or_else(|| invalid_handle("JMAP endpoint missing host"))?
            .to_string()
            + &endpoint
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
    );
    let session = provider_get_json(&profile, &session_url).await?;
    let download_template = session
        .get("downloadUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            structured(
                "provider_request_failed",
                "JMAP session response has no downloadUrl template",
                None,
            )
        })?;
    let account_id = session
        .get("primaryAccounts")
        .and_then(|value| value.get(JMAP_MAIL_CAPABILITY))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            session
                .get("accounts")
                .and_then(Value::as_object)
                .and_then(|accounts| accounts.keys().next().cloned())
        })
        .ok_or_else(|| {
            structured(
                "provider_request_failed",
                "JMAP session response exposes no mail accountId",
                None,
            )
        })?
        .to_string();
    let download_url = download_template
        .replace("{accountId}", &encode_path_segment(&account_id))
        .replace("{blobId}", &encode_path_segment(blob_id))
        .replace("{name}", "attachment");
    let bytes = provider_get_bytes(&profile, &download_url).await?;
    Ok(fetched(bytes))
}

/// Resolve the auth profile name for a retrieval, preferring the CLI
/// override over the handle reference.
fn required_profile_name(
    handle: &EmailAttachmentHandle,
    request: &EmailAttachmentGetRequest,
) -> Result<String> {
    request
        .profile
        .clone()
        .or_else(|| handle.auth_profile.clone())
        .ok_or_else(|| {
            structured(
                "auth_profile_missing",
                "attachment handle references no auth profile; pass --profile <name>",
                None,
            )
        })
}

fn load_profile(name: &str) -> Result<Profile> {
    let profiles = Profiles::load_profiles().map_err(|err| {
        structured(
            "auth_profile_missing",
            format!("failed to load auth profiles: {err}"),
            None,
        )
    })?;
    profiles.get_profile(name).cloned().map_err(|_| {
        structured(
            "auth_profile_missing",
            format!("auth profile '{}' not found in the local auth store", name),
            None,
        )
    })
}

fn resolve_first_profile_field(profile: &Profile, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| profile.resolve_field_value(name).ok().flatten())
}

async fn authorized_get(profile: &Profile, url: &str) -> Result<reqwest::Response> {
    let request_context = AuthRequestContext::new("GET", url);
    let resolved =
        resolve_profile_request_auth_with_context(&request_context, profile).map_err(|err| {
            structured(
                "auth_failed",
                format!("auth resolution failed: {err}"),
                None,
            )
        })?;
    let client = reqwest::Client::new();
    let mut builder = client.get(&resolved.url);
    for (name, value) in &resolved.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
        .send()
        .await
        .map_err(|err| provider_request_failed(format!("provider request failed: {err}")))
}

async fn provider_get_json(profile: &Profile, url: &str) -> Result<Value> {
    let response = authorized_get(profile, url).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(provider_status_error(status.as_u16(), url, response).await);
    }
    response
        .json::<Value>()
        .await
        .map_err(|err| provider_request_failed(format!("provider response was not JSON: {err}")))
}

async fn provider_get_bytes(profile: &Profile, url: &str) -> Result<Vec<u8>> {
    let response = authorized_get(profile, url).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(provider_status_error(status.as_u16(), url, response).await);
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| provider_request_failed(format!("provider response body failed: {err}")))
}

async fn provider_status_error(
    status: u16,
    url: &str,
    response: reqwest::Response,
) -> anyhow::Error {
    let body = response.text().await.unwrap_or_default();
    let trimmed_body = body.chars().take(400).collect::<String>();
    let provider_error = if status == 401 || status == 403 {
        structured(
            "auth_failed",
            format!(
                "provider rejected the request with HTTP {}: {}",
                status, trimmed_body
            ),
            None,
        )
    } else if status == 404 || status == 410 {
        structured(
            "attachment_not_found",
            format!("provider returned HTTP {} for {}", status, url),
            None,
        )
    } else {
        provider_request_failed(format!(
            "provider request failed with HTTP {}: {}",
            status, trimmed_body
        ))
    };
    provider_error
}

fn decode_gmail_base64url(data: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
    let cleaned: String = data.chars().filter(|ch| !ch.is_whitespace()).collect();
    URL_SAFE_NO_PAD
        .decode(cleaned.trim_end_matches('='))
        .or_else(|_| URL_SAFE.decode(&cleaned))
        .context("gmail attachment data is not valid base64url")
}

fn parse_imap_uidvalidity_from_select(lines: &[String]) -> Option<u64> {
    crate::subscription_email::parse_imap_uidvalidity(lines)
}

fn valid_imap_section(section: &str) -> bool {
    !section.is_empty()
        && section
            .split('.')
            .all(|segment| !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit()))
}

fn valid_url_path_segment(value: &str) -> bool {
    !value.is_empty() && !value.contains('/') && !value.contains('?') && !value.contains('#')
}

fn encode_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => ch.to_string(),
            _ => format!("%{:02X}", ch as u32 as u8),
        })
        .collect()
}

fn write_attachment(
    handle: &EmailAttachmentHandle,
    bytes: &[u8],
    output: Option<&str>,
) -> Result<String> {
    let path = match output {
        Some(explicit) => PathBuf::from(explicit),
        None => default_attachment_path(handle)?,
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create attachment output directory '{}'",
                    parent.display()
                )
            })?;
        }
    }
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write attachment to '{}'", path.display()))?;
    Ok(path.display().to_string())
}

/// Default cache output path: `~/.uxc/email-attachments/<provider>-<uid>-<id>`
/// with the filename extension preserved when the provider reports one.
fn default_attachment_path(handle: &EmailAttachmentHandle) -> Result<PathBuf> {
    let base = auth_base_dir().context("failed to resolve attachment cache directory")?;
    let uid = handle.uid.as_deref().unwrap_or("message");
    let stem = format!(
        "{}-{}-{}",
        handle.provider.as_str(),
        encode_path_segment(uid),
        encode_path_segment(handle.part.id())
    );
    let mut candidate = base.join(ATTACHMENT_CACHE_DIR).join(stem.clone());
    let mut counter = 1u32;
    while candidate.exists() {
        candidate.set_file_name(format!("{}-{}", stem, counter));
        counter += 1;
    }
    Ok(candidate)
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn structured(code: &str, message: impl Into<String>, details: Option<Value>) -> anyhow::Error {
    anyhow::Error::new(StructuredError::new(code, message, details))
}

fn invalid_handle(message: &str) -> anyhow::Error {
    structured("invalid_attachment_handle", message, None)
}

fn provider_request_failed(message: impl Into<String>) -> anyhow::Error {
    structured("provider_request_failed", message, None)
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::error::structured_error_from_anyhow;
    use base64::Engine;
    use mockito::Server;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use crate::test_support::credentials_env_lock;

    const GMAIL_CREDENTIALS: &str = r#"{
        "version": 1,
        "credentials": {
            "gmail-test": {
                "auth_type": "bearer",
                "api_key": "tok-123"
            },
            "imap-test": {
                "auth_type": "basic",
                "fields": {
                    "username": {"kind": "literal", "value": "user@example.com"},
                    "password": {"kind": "literal", "value": "app-pass"}
                }
            },
            "imap-oauth-test": {
                "auth_type": "oauth",
                "fields": {
                    "username": {"kind": "literal", "value": "user@hotmail.com"}
                },
                "oauth": {
                    "access_token": "imap-oauth-token",
                    "scopes": ["https://outlook.office.com/IMAP.AccessAsUser.All"]
                }
            }
        }
    }"#;

    fn set_credentials_file(contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("credentials.json"), contents).expect("write credentials");
        std::env::set_var("UXC_CREDENTIALS_FILE", dir.path().join("credentials.json"));
        dir
    }

    /// Kept temp directory for attachment output paths.
    fn test_output_dir() -> std::path::PathBuf {
        tempfile::tempdir().expect("temp dir").into_path()
    }

    fn gmail_handle(endpoint: &str) -> String {
        json!({
            "type": "email_attachment",
            "provider": "gmail",
            "endpoint": endpoint,
            "account": "user@example.com",
            "mailbox": "INBOX",
            "message_id": "<m1@x>",
            "uid": "msg-1",
            "auth_profile": "gmail-test",
            "part": {"attachment_id": "ATT-1"}
        })
        .to_string()
    }

    fn request(
        handle: String,
        output: Option<String>,
        max_bytes: u64,
    ) -> EmailAttachmentGetRequest {
        EmailAttachmentGetRequest {
            handle,
            profile: None,
            output,
            max_bytes,
        }
    }

    fn error_code_of(err: &anyhow::Error) -> String {
        structured_error_from_anyhow(err)
            .expect("error should be structured")
            .code
    }

    fn imap_handle(uidvalidity: Option<u64>) -> String {
        json!({
            "type": "email_attachment",
            "provider": "imap",
            "endpoint": "imap://127.0.0.1:1143",
            "account": "user@example.com",
            "mailbox": "INBOX",
            "message_id": "<m1@x>",
            "uid": "42",
            "uidvalidity": uidvalidity,
            "auth_profile": "imap-test",
            "part": {"section": "2"}
        })
        .to_string()
    }

    /// Scripted IMAP server over tokio duplex.
    fn imap_scripted_connector(
        script: Vec<(String, String)>,
    ) -> impl Fn(EmailImapIdleRuntimeConfig) -> crate::subscription_email::BoxFutureResult<ImapConnection>
    {
        move |_config| {
            let script = script.clone();
            Box::pin(async move {
                let (client, server) = tokio::io::duplex(64 * 1024);
                tokio::spawn(async move {
                    let (reader, mut writer) = tokio::io::split(server);
                    let mut lines = BufReader::new(reader).lines();
                    writer.write_all(b"* OK test ready\r\n").await.unwrap();
                    for (expect_prefix, response) in script {
                        let line = lines.next_line().await.unwrap().unwrap_or_default();
                        assert!(
                            line.starts_with(&expect_prefix),
                            "expected command starting with {:?}, got {:?}",
                            expect_prefix,
                            line
                        );
                        writer.write_all(response.as_bytes()).await.unwrap();
                        writer.write_all(b"\r\n").await.unwrap();
                    }
                }) as tokio::task::JoinHandle<()>;
                Ok(ImapConnection::new(Box::new(client)))
            })
        }
    }

    fn imap_login_select_script(
        uidvalidity: u64,
        fetch_responses: Vec<String>,
    ) -> Vec<(String, String)> {
        let fetch_count = fetch_responses.len();
        let mut script = vec![
            (
                "A0001 LOGIN".to_string(),
                "A0001 OK LOGIN done".to_string(),
            ),
            (
                "A0002 SELECT".to_string(),
                format!(
                    "* 3 EXISTS\r\n* OK [UIDVALIDITY {uidvalidity}] UIDs valid\r\nA0002 OK [READ-WRITE] SELECT completed"
                ),
            ),
        ];
        for (index, response) in fetch_responses.into_iter().enumerate() {
            script.push((
                format!("A000{:}", index + 3),
                format!("{}\r\nA000{:} OK fetch done", response, index + 3),
            ));
        }
        script.push((format!("A000{} LOGOUT", fetch_count + 3), String::new()));
        script
    }

    fn literal_response(section: &str, payload: &[u8]) -> String {
        format!(
            "* 1 FETCH (UID 42 BODY[{}] {{{}}}\r\n{}\r\n)",
            section,
            payload.len(),
            String::from_utf8_lossy(payload)
        )
    }

    fn imap_mime_headers() -> Vec<u8> {
        b"Content-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n".to_vec()
    }

    fn base64_body_payload() -> Vec<u8> {
        b"aGVsbG8gcGF5bG9hZA==".to_vec()
    }

    #[test]
    fn parses_and_validates_handles() {
        let handle = parse_attachment_handle(&imap_handle(Some(3857529045))).unwrap();
        assert_eq!(handle.provider, EmailAttachmentProvider::Imap);
        assert_eq!(handle.endpoint, "imap://127.0.0.1:1143");
        assert_eq!(handle.uid.as_deref(), Some("42"));
        assert_eq!(handle.uidvalidity, Some(3857529045));
        assert_eq!(handle.part, AttachmentPart::Section("2".to_string()));
        assert_eq!(handle.auth_profile.as_deref(), Some("imap-test"));

        let gmail = parse_attachment_handle(&gmail_handle("https://x")).unwrap();
        assert_eq!(
            gmail.part,
            AttachmentPart::AttachmentId("ATT-1".to_string())
        );

        let invalid = [
            "not json",
            "{\"type\":\"email_reply\"}",
            "{\"type\":\"email_attachment\",\"provider\":\"pop3\",\"endpoint\":\"x\",\"part\":{}}",
            "{\"type\":\"email_attachment\",\"provider\":\"imap\",\"endpoint\":\"imap://h\",\"uid\":\"42\",\"part\":{}}",
            "{\"type\":\"email_attachment\",\"provider\":\"imap\",\"endpoint\":\"imap://h\",\"uid\":\"42; DROP\",\"part\":{\"section\":\"2\"}}",
            "{\"type\":\"email_attachment\",\"provider\":\"imap\",\"endpoint\":\"imap://h\",\"uid\":\"42\",\"part\":{\"section\":\"2; DROP\"}}",
            "{\"type\":\"email_attachment\",\"provider\":\"gmail\",\"endpoint\":\"https://x\",\"part\":{\"attachment_id\":\"A\"}}",
            "{\"type\":\"email_attachment\",\"provider\":\"gmail\",\"endpoint\":\"https://x\",\"uid\":\"m\",\"part\":{}}",
            "{\"type\":\"email_attachment\",\"provider\":\"jmap\",\"endpoint\":\"https://x\",\"part\":{\"blob_id\":\"../escape\"}}",
        ];
        for raw in invalid {
            let err = parse_attachment_handle(raw).unwrap_err();
            assert_eq!(
                error_code_of(&err),
                "invalid_attachment_handle",
                "raw: {raw}"
            );
        }
    }

    #[tokio::test]
    async fn imap_download_decodes_base64_section() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let output = test_output_dir();
        let output_path = output.join("report.bin").display().to_string();
        let mime = imap_mime_headers();
        let body = base64_body_payload();
        let mime_response = format!(
            "* 1 FETCH (UID 42 BODY[2.MIME] {{{}}}\r\n{}",
            mime.len(),
            String::from_utf8_lossy(&mime)
        );
        // The MIME literal is followed by `)\r\n` on its own line, and the
        // body literal likewise; the connector script appends CRLF per entry
        // so the suffix line is embedded directly.
        let mime_entry = format!("{mime_response}\r\n)");
        let body_entry = literal_response("2", &body);
        let script = imap_login_select_script(3857529045, vec![mime_entry, body_entry]);
        let connector = imap_scripted_connector(script);
        let request = request(imap_handle(Some(3857529045)), Some(output_path), 0);
        let result = get_email_attachment_with(&request, connector)
            .await
            .unwrap();
        assert_eq!(result.provider, "imap");
        assert_eq!(result.attachment_id, "2");
        assert_eq!(result.filename.as_deref(), Some("report.pdf"));
        assert_eq!(result.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(result.size_bytes, 13);
        assert_eq!(
            std::fs::read(&result.saved_path).unwrap(),
            b"hello payload".to_vec()
        );
    }

    #[tokio::test]
    async fn imap_download_authenticates_with_xoauth2_for_oauth_profiles() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let output = test_output_dir();
        let mime = imap_mime_headers();
        // The MIME section advertises base64 transfer encoding, so the body
        // literal must be the encoded payload.
        let body = base64::engine::general_purpose::STANDARD
            .encode("oauth payload")
            .into_bytes();
        let mime_entry = format!(
            "* 1 FETCH (UID 42 BODY[2.MIME] {{{}}}\r\n{}\r\n)",
            mime.len(),
            String::from_utf8_lossy(&mime)
        );
        let body_entry = literal_response("2", &body);
        let script = vec![
            (
                "A0001 AUTHENTICATE XOAUTH2".to_string(),
                "A0001 OK Authenticated.".to_string(),
            ),
            (
                "A0002 SELECT".to_string(),
                "* OK [UIDVALIDITY 3857529045] UIDs valid\r\n* 0 EXISTS\r\nA0002 OK [READ-WRITE] SELECT done".to_string(),
            ),
            (
                "A0003 UID FETCH".to_string(),
                format!("{mime_entry}\r\nA0003 OK fetch done"),
            ),
            (
                "A0004 UID FETCH".to_string(),
                format!("{body_entry}\r\nA0004 OK fetch done"),
            ),
            ("A0005 LOGOUT".to_string(), "A0005 OK logout".to_string()),
        ];
        let connector = imap_scripted_connector(script);
        let mut handle =
            serde_json::from_str::<serde_json::Value>(&imap_handle(Some(3857529045))).unwrap();
        handle["auth_profile"] = json!("imap-oauth-test");
        let request = request(
            handle.to_string(),
            Some(output.join("oauth.bin").display().to_string()),
            0,
        );
        let result = get_email_attachment_with(&request, connector)
            .await
            .unwrap();
        assert_eq!(result.provider, "imap");
        assert_eq!(
            std::fs::read(&result.saved_path).unwrap(),
            b"oauth payload".to_vec()
        );
    }

    #[tokio::test]
    async fn imap_uidvalidity_mismatch_maps_to_uid_invalid() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let output = test_output_dir();
        let script = imap_login_select_script(999999, vec![]);
        let connector = imap_scripted_connector(script);
        let request = request(
            imap_handle(Some(3857529045)),
            Some(output.join("x").display().to_string()),
            0,
        );
        let err = get_email_attachment_with(&request, connector)
            .await
            .unwrap_err();
        assert_eq!(error_code_of(&err), "uid_invalid");
    }

    #[tokio::test]
    async fn imap_missing_uid_maps_to_message_not_found() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let output = test_output_dir();
        let script = imap_login_select_script(
            3857529045,
            vec!["* 1 FETCH (UID 41 BODY[2.MIME] \"x\")".to_string()],
        );
        let connector = imap_scripted_connector(script);
        let request = request(
            imap_handle(Some(3857529045)),
            Some(output.join("x").display().to_string()),
            0,
        );
        let err = get_email_attachment_with(&request, connector)
            .await
            .unwrap_err();
        assert_eq!(error_code_of(&err), "message_not_found");
    }

    #[tokio::test]
    async fn missing_profile_reference_maps_to_auth_profile_missing() {
        let _guard = credentials_env_lock().lock().unwrap();
        let credentials = tempfile::tempdir().unwrap();
        std::env::set_var(
            "UXC_CREDENTIALS_FILE",
            credentials.path().join("credentials.json"),
        );
        let request = request(gmail_handle("https://x"), None, 0);
        let err = get_email_attachment(&request).await.unwrap_err();
        assert_eq!(error_code_of(&err), "auth_profile_missing");
    }

    #[tokio::test]
    async fn gmail_download_decodes_base64url_and_reports_metadata() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/gmail/v1/users/me/messages", server.url());
        let data = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello attachment");
        let mock = server
            .mock("GET", "/gmail/v1/users/me/messages/msg-1/attachments/ATT-1")
            .match_header("authorization", "Bearer tok-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "size": 16,
                    "filename": "report.pdf",
                    "mimeType": "application/pdf",
                    "data": data
                })
                .to_string(),
            )
            .create_async()
            .await;
        let output = test_output_dir();
        let request = request(
            gmail_handle(&endpoint),
            Some(output.join("gmail.bin").display().to_string()),
            0,
        );
        let result = get_email_attachment(&request).await.unwrap();
        mock.assert_async().await;
        assert_eq!(result.provider, "gmail");
        assert_eq!(result.filename.as_deref(), Some("report.pdf"));
        assert_eq!(result.size_bytes, 16);
        assert_eq!(
            std::fs::read(&result.saved_path).unwrap(),
            b"hello attachment".to_vec()
        );
    }

    #[tokio::test]
    async fn gmail_404_maps_to_attachment_not_found() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/gmail/v1/users/me/messages", server.url());
        server
            .mock("GET", mockito::Matcher::Any)
            .with_status(404)
            .create_async()
            .await;
        let output = test_output_dir();
        let request = request(
            gmail_handle(&endpoint),
            Some(output.join("x.bin").display().to_string()),
            0,
        );
        let err = get_email_attachment(&request).await.unwrap_err();
        assert_eq!(error_code_of(&err), "attachment_not_found");
    }

    #[tokio::test]
    async fn gmail_401_maps_to_auth_failed() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/gmail/v1/users/me/messages", server.url());
        server
            .mock("GET", mockito::Matcher::Any)
            .with_status(401)
            .create_async()
            .await;
        let output = test_output_dir();
        let request = request(
            gmail_handle(&endpoint),
            Some(output.join("x.bin").display().to_string()),
            0,
        );
        let err = get_email_attachment(&request).await.unwrap_err();
        assert_eq!(error_code_of(&err), "auth_failed");
    }

    #[tokio::test]
    async fn size_limit_maps_to_size_limit_exceeded() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/gmail/v1/users/me/messages", server.url());
        let data = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello attachment");
        server
            .mock("GET", "/gmail/v1/users/me/messages/msg-1/attachments/ATT-1")
            .with_status(200)
            .with_body(json!({"size": 16, "data": data}).to_string())
            .create_async()
            .await;
        let output = test_output_dir();
        let request = request(
            gmail_handle(&endpoint),
            Some(output.join("x.bin").display().to_string()),
            10,
        );
        let err = get_email_attachment(&request).await.unwrap_err();
        assert_eq!(error_code_of(&err), "size_limit_exceeded");
    }

    #[tokio::test]
    async fn graph_download_returns_raw_bytes() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/v1.0/users/me/messages", server.url());
        let payload: Vec<u8> = vec![0x89, 0x50, 0x4e, 0x47, 0x00, 0xff, 0x01];
        let mock = server
            .mock("GET", "/v1.0/users/me/messages/g1/attachments/A1/$value")
            .match_header("authorization", "Bearer tok-123")
            .with_status(200)
            .with_body(payload.clone())
            .create_async()
            .await;
        let handle = json!({
            "type": "email_attachment",
            "provider": "graph",
            "endpoint": endpoint,
            "uid": "g1",
            "auth_profile": "gmail-test",
            "part": {"attachment_id": "A1"}
        })
        .to_string();
        let output = test_output_dir();
        let request = request(
            handle,
            Some(output.join("graph.bin").display().to_string()),
            0,
        );
        let result = get_email_attachment(&request).await.unwrap();
        mock.assert_async().await;
        assert_eq!(result.provider, "graph");
        assert_eq!(result.size_bytes, 7);
        assert_eq!(std::fs::read(&result.saved_path).unwrap(), payload);
    }

    #[tokio::test]
    async fn jmap_download_resolves_session_download_template() {
        let _guard = credentials_env_lock().lock().unwrap();
        let _credentials = set_credentials_file(GMAIL_CREDENTIALS);
        let mut server = Server::new_async().await;
        let endpoint = format!("{}/api", server.url());
        let session_mock = server
            .mock("GET", "/.well-known/jmap")
            .match_header("authorization", "Bearer tok-123")
            .with_status(200)
            .with_body(
                json!({
                    "downloadUrl": format!("{}/download/{{accountId}}/{{blobId}}/{{name}}", server.url()),
                    "primaryAccounts": {"urn:ietf:params:jmap:mail": "acc-1"},
                    "accounts": {"acc-1": {"name": "Test"}}
                })
                .to_string(),
            )
            .create_async()
            .await;
        let payload = b"jmap blob bytes".to_vec();
        let download_mock = server
            .mock("GET", "/download/acc-1/B123/attachment")
            .with_status(200)
            .with_body(payload.clone())
            .create_async()
            .await;
        let handle = json!({
            "type": "email_attachment",
            "provider": "jmap",
            "endpoint": endpoint,
            "uid": "m1",
            "auth_profile": "gmail-test",
            "part": {"blob_id": "B123"}
        })
        .to_string();
        let output = test_output_dir();
        let request = request(
            handle,
            Some(output.join("jmap.bin").display().to_string()),
            0,
        );
        let result = get_email_attachment(&request).await.unwrap();
        session_mock.assert_async().await;
        download_mock.assert_async().await;
        assert_eq!(result.provider, "jmap");
        assert_eq!(result.attachment_id, "B123");
        assert_eq!(std::fs::read(&result.saved_path).unwrap(), payload);
    }
}
