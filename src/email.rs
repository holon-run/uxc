use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use url::Url;

use crate::auth;
use crate::subscription_email::AsyncReadWrite;

type SmtpIo = Box<dyn AsyncReadWrite>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailReplyHandle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct EmailSendRequest {
    pub smtp_url: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub text: Option<String>,
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub auth: Option<String>,
    /// Optional caller-supplied Message-ID. Used for outbound idempotency:
    /// retries with the same correlation key produce the same Message-ID.
    pub message_id: Option<String>,
    pub allow_insecure_auth: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailSendResult {
    pub smtp_url: String,
    pub from: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<String>,
    pub subject: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    pub dry_run: bool,
    pub accepted_recipients: usize,
}

/// How `send_email` authenticates against the SMTP server.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SmtpAuth {
    /// `AUTH PLAIN` with username and password.
    Plain { username: String, password: String },
    /// `AUTH XOAUTH2` with an OAuth access token (for example a Microsoft
    /// personal account via `uxc auth oauth login --provider outlook`).
    Xoauth2 { username: String, token: String },
}

pub async fn send_email(request: EmailSendRequest) -> Result<EmailSendResult> {
    validate_request(&request)?;
    let smtp_url = parse_smtp_url(&request.smtp_url)?;
    let message_id = request
        .message_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "<uxc-{}-{}@localhost>",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        });
    let body = build_message(&request, &message_id)?;
    let accepted_recipients = request.to.len() + request.cc.len() + request.bcc.len();

    if !request.dry_run {
        let auth = resolve_smtp_auth(&request.smtp_url, request.auth.clone()).await?;
        validate_smtp_auth_transport(&smtp_url, auth.as_ref(), request.allow_insecure_auth)?;
        send_smtp(
            &smtp_url,
            auth.as_ref(),
            &request.from,
            all_recipients(&request),
            &body,
        )
        .await?;
    }

    Ok(EmailSendResult {
        smtp_url: request.smtp_url,
        from: request.from,
        to: request.to,
        cc: request.cc,
        bcc: request.bcc,
        subject: request.subject,
        message_id,
        in_reply_to: request.in_reply_to,
        references: request.references,
        dry_run: request.dry_run,
        accepted_recipients,
    })
}

pub fn parse_reply_handle_json(raw: &str) -> Result<EmailReplyHandle> {
    serde_json::from_str(raw).context("invalid --reply-handle JSON")
}

pub fn references_with_reply_handle(
    reply_handle: Option<&EmailReplyHandle>,
    explicit_in_reply_to: Option<String>,
    mut references: Vec<String>,
) -> (Option<String>, Vec<String>) {
    let in_reply_to = explicit_in_reply_to.or_else(|| {
        reply_handle
            .and_then(|handle| handle.message_id.clone())
            .filter(|value| !value.trim().is_empty())
    });
    if let Some(message_id) = &in_reply_to {
        if !references.iter().any(|value| value == message_id) {
            references.push(message_id.clone());
        }
    }
    (in_reply_to, references)
}

fn validate_request(request: &EmailSendRequest) -> Result<()> {
    if request.from.trim().is_empty() {
        bail!("--from is required");
    }
    if request.to.is_empty() && request.cc.is_empty() && request.bcc.is_empty() {
        bail!("at least one --to, --cc, or --bcc recipient is required");
    }
    if request.subject.trim().is_empty() {
        bail!("--subject is required");
    }
    if request.text.is_none() && request.html.is_none() {
        bail!("one of --text-body or --html-body is required");
    }
    Ok(())
}

fn parse_smtp_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).context("invalid SMTP endpoint URL")?;
    match url.scheme() {
        "smtp" => Ok(url),
        "smtps" => Ok(url),
        other => bail!("unsupported SMTP URL scheme '{}'; expected smtp://", other),
    }
}

fn validate_smtp_auth_transport(
    url: &Url,
    auth: Option<&SmtpAuth>,
    allow_insecure_auth: bool,
) -> Result<()> {
    let Some(auth) = auth else {
        return Ok(());
    };
    match (url.scheme(), auth) {
        // Passwords over smtp:// stay opt-in: the local-relay workflow keeps
        // working only when the operator acknowledges the cleartext risk.
        ("smtp", SmtpAuth::Plain { .. }) if !allow_insecure_auth => bail!(
            "refusing to send SMTP password over unencrypted smtp://; configure a trusted local relay without --auth, pass --allow-insecure-auth to acknowledge the cleartext credential risk, or use smtps://"
        ),
        // OAuth tokens never travel in cleartext: smtp:// upgrades via
        // STARTTLS (enforced at send time) and smtps:// is implicit TLS.
        ("smtp" | "smtps", SmtpAuth::Xoauth2 { .. }) => Ok(()),
        _ => Ok(()),
    }
}

async fn resolve_smtp_auth(
    endpoint: &str,
    explicit_auth: Option<String>,
) -> Result<Option<SmtpAuth>> {
    let Some(mut profile) = auth::resolve_auth_for_endpoint(endpoint, explicit_auth)? else {
        return Ok(None);
    };
    if profile.auth_type == auth::AuthType::OAuth {
        // One-shot send: refresh a stale token up front and persist it so
        // the next send reuses the fresh token without another login.
        let client = reqwest::Client::new();
        let refreshed = auth::refresh_effective_auth_profile(
            &mut profile,
            &client,
            false,
            crate::subscription_email::IMAP_XOAUTH2_REFRESH_SKEW_SECS,
            None,
        )
        .await?;
        if refreshed {
            auth::persist_profile_if_named(&profile)?;
        }
        let username = first_field(&profile, &["username", "user", "email", "account"])?
            .or_else(|| profile.name.clone())
            .ok_or_else(|| {
                anyhow!(
                    "SMTP OAuth auth profile requires a username/user/email/account field, or the credential id to name the mailbox address"
                )
            })?;
        let token = profile
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.access_token.clone())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "SMTP OAuth auth profile has no access token; run `uxc auth oauth login` first"
                )
            })?;
        return Ok(Some(SmtpAuth::Xoauth2 { username, token }));
    }
    let username = first_field(&profile, &["username", "user", "account"])?
        .or_else(|| profile.name.clone())
        .ok_or_else(|| anyhow!("SMTP auth profile requires username/user/account field"))?;
    let password = first_field(&profile, &["password", "secret"])?
        .ok_or_else(|| anyhow!("SMTP auth profile requires password or secret field"))?;
    Ok(Some(SmtpAuth::Plain { username, password }))
}

fn first_field(profile: &auth::Profile, names: &[&str]) -> Result<Option<String>> {
    for name in names {
        if let Some(value) = profile.resolve_field_value(name)? {
            if !value.is_empty() {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}

fn all_recipients(request: &EmailSendRequest) -> Vec<String> {
    request
        .to
        .iter()
        .chain(request.cc.iter())
        .chain(request.bcc.iter())
        .cloned()
        .collect()
}

fn build_message(request: &EmailSendRequest, message_id: &str) -> Result<String> {
    let mut headers = Vec::new();
    headers.push(("From", request.from.clone()));
    headers.push(("To", request.to.join(", ")));
    if !request.cc.is_empty() {
        headers.push(("Cc", request.cc.join(", ")));
    }
    headers.push(("Subject", request.subject.clone()));
    headers.push(("Message-ID", message_id.to_string()));
    if let Some(in_reply_to) = &request.in_reply_to {
        headers.push(("In-Reply-To", in_reply_to.clone()));
    }
    if !request.references.is_empty() {
        headers.push(("References", request.references.join(" ")));
    }
    headers.push(("MIME-Version", "1.0".to_string()));

    let boundary = format!("uxc-boundary-{}", message_id.trim_matches(&['<', '>'][..]));
    let body = match (&request.text, &request.html) {
        (Some(text), Some(html)) => {
            headers.push((
                "Content-Type",
                format!("multipart/alternative; boundary=\"{}\"", boundary),
            ));
            format!(
                "--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{text}\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{html}\r\n--{boundary}--\r\n"
            )
        }
        (Some(text), None) => {
            headers.push(("Content-Type", "text/plain; charset=utf-8".to_string()));
            headers.push(("Content-Transfer-Encoding", "8bit".to_string()));
            format!("{text}\r\n")
        }
        (None, Some(html)) => {
            headers.push(("Content-Type", "text/html; charset=utf-8".to_string()));
            headers.push(("Content-Transfer-Encoding", "8bit".to_string()));
            format!("{html}\r\n")
        }
        (None, None) => unreachable!("validated before message construction"),
    };

    let mut message = String::new();
    for (name, value) in headers {
        if !value.is_empty() {
            message.push_str(name);
            message.push_str(": ");
            message.push_str(&sanitize_header_value(&value)?);
            message.push_str("\r\n");
        }
    }
    message.push_str("\r\n");
    message.push_str(&body);
    Ok(message)
}

fn sanitize_header_value(value: &str) -> Result<String> {
    if value.contains('\r') || value.contains('\n') {
        bail!("email header values must not contain newlines");
    }
    Ok(value.to_string())
}

async fn send_smtp(
    url: &Url,
    auth: Option<&SmtpAuth>,
    from: &str,
    recipients: Vec<String>,
    message: &str,
) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("SMTP URL must include a host"))?;
    // `Url` knows smtp but not smtps, so map the implicit-TLS port manually.
    let default_port = if url.scheme() == "smtps" { 465 } else { 25 };
    let port = url.port().unwrap_or(default_port);
    let tcp = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("failed to connect SMTP server {}:{}", host, port))?;
    let mut client = match url.scheme() {
        // smtps:// upgrades the TCP stream to TLS before the greeting.
        "smtps" => SmtpClient::new(Box::new(tls_upgrade(host, tcp).await?)),
        _ => SmtpClient::new(Box::new(tcp)),
    };
    client.expect_code(&[220]).await?;
    let capabilities = client.ehlo().await?;
    // OAuth tokens must never travel over cleartext smtp://: upgrade via
    // STARTTLS when the server advertises it, refuse otherwise.
    if url.scheme() == "smtp" && matches!(auth, Some(SmtpAuth::Xoauth2 { .. })) {
        if capabilities
            .iter()
            .any(|cap| cap.eq_ignore_ascii_case("STARTTLS"))
        {
            client = client.starttls(host).await?;
            client.ehlo().await?;
        } else {
            bail!(
                "SMTP server {} does not advertise STARTTLS; refusing to send the OAuth token over unencrypted smtp://. Use smtps:// or a STARTTLS-capable submission endpoint (for Microsoft personal accounts: smtp-mail.outlook.com:587)",
                host
            );
        }
    }
    if let Some(auth) = auth {
        match auth {
            SmtpAuth::Plain { username, password } => {
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("\0{username}\0{password}"));
                client
                    .command(&format!("AUTH PLAIN {encoded}\r\n"), &[235])
                    .await?;
            }
            SmtpAuth::Xoauth2 { username, token } => {
                client.auth_xoauth2(username, token).await?;
            }
        }
    }
    client
        .command(&format!("MAIL FROM:<{}>\r\n", from), &[250])
        .await?;
    for recipient in recipients {
        client
            .command(&format!("RCPT TO:<{}>\r\n", recipient), &[250, 251])
            .await?;
    }
    client.command("DATA\r\n", &[354]).await?;
    client.write_all(dot_stuff(message).as_bytes()).await?;
    client.write_all(b"\r\n.\r\n").await?;
    client.expect_code(&[250]).await?;
    let _ = client.command("QUIT\r\n", &[221]).await;
    Ok(())
}

async fn tls_upgrade<S: AsyncReadWrite>(
    host: &str,
    stream: S,
) -> Result<tokio_rustls::client::TlsStream<S>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(tls_config));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| anyhow!("invalid SMTP TLS server name"))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        connector.connect(server_name, stream),
    )
    .await
    .context("SMTP TLS handshake timed out")?
    .context("SMTP TLS handshake failed")
}

fn local_hostname() -> String {
    sanitize_local_hostname(std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "localhost".to_string())
}

fn sanitize_local_hostname(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && !trimmed.contains('\r') && !trimmed.contains('\n')
    })
}

fn dot_stuff(message: &str) -> String {
    message
        .replace("\r\n.", "\r\n..")
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

struct SmtpClient {
    reader: BufReader<tokio::io::ReadHalf<SmtpIo>>,
    writer: tokio::io::WriteHalf<SmtpIo>,
}

impl SmtpClient {
    fn new(io: SmtpIo) -> Self {
        let (reader, writer) = tokio::io::split(io);
        Self {
            reader: BufReader::new(reader),
            writer,
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Write a command and return the full multiline response when the code
    /// matches, so callers can inspect capability lines.
    async fn command_lines(
        &mut self,
        command: &str,
        expected: &[u16],
    ) -> Result<(u16, Vec<String>)> {
        self.write_all(command.as_bytes()).await?;
        let (code, lines) = self.read_response().await?;
        if expected.contains(&code) {
            Ok((code, lines))
        } else {
            bail!(
                "SMTP server returned {}, expected {:?}: {}",
                code,
                expected,
                lines.join(" | ")
            )
        }
    }

    /// Send EHLO and return the advertised capability keywords (upper-cased
    /// verb per line, e.g. `STARTTLS`, `AUTH`).
    async fn ehlo(&mut self) -> Result<Vec<String>> {
        let (_, lines) = self
            .command_lines(&format!("EHLO {}\r\n", local_hostname()), &[250])
            .await?;
        Ok(lines
            .iter()
            // Skip the response code prefix ("250 " or "250-") on each
            .skip(1)
            .filter_map(|line| line.get(4..))
            .map(|rest| rest.split(' ').next().unwrap_or("").to_string())
            .filter(|verb| !verb.is_empty())
            .collect())
    }

    /// Upgrade the current plaintext connection with STARTTLS. Returns a new
    /// client wrapping the TLS stream; the plaintext client is consumed.
    async fn starttls(self, host: &str) -> Result<Self> {
        let mut plaintext = self;
        if !plaintext.reader.buffer().is_empty() {
            bail!("SMTP STARTTLS refused: buffered plaintext data would be discarded");
        }
        plaintext.command("STARTTLS\r\n", &[220]).await?;
        let read_half = plaintext.reader.into_inner();
        // Both halves come from the same `split`, so `unsplit` cannot fail.
        let io = read_half.unsplit(plaintext.writer);
        let tls = tls_upgrade(host, io).await?;
        Ok(Self::new(Box::new(tls)))
    }

    /// `AUTH XOAUTH2` with an inline initial response. On a `334` challenge
    /// (base64 JSON error) the client aborts SASL with `*` per RFC 4954.
    async fn auth_xoauth2(&mut self, username: &str, token: &str) -> Result<()> {
        let sasl = crate::subscription_email::build_xoauth2_sasl_string(username, token);
        let encoded = base64::engine::general_purpose::STANDARD.encode(sasl);
        self.write_all(format!("AUTH XOAUTH2 {encoded}\r\n").as_bytes())
            .await?;
        let (code, lines) = self.read_response().await?;
        if code == 235 {
            return Ok(());
        }
        let mut detail = lines.join(" | ");
        if code == 334 {
            self.write_all(b"*\r\n").await?;
            let (_, abort_lines) = self.read_response().await?;
            detail.push_str(" | abort: ");
            detail.push_str(&abort_lines.join(" | "));
        }
        bail!(
            "SMTP AUTH XOAUTH2 rejected ({}): the access token is missing, expired, or lacks the required SMTP scope (for Microsoft accounts: https://outlook.office.com/SMTP.Send). Re-run `uxc auth oauth login <credential> --provider outlook` to refresh the token. Server response: {}",
            code,
            detail
        )
    }

    async fn command(&mut self, command: &str, expected: &[u16]) -> Result<u16> {
        Ok(self.command_lines(command, expected).await?.0)
    }

    async fn expect_code(&mut self, expected: &[u16]) -> Result<u16> {
        let (code, lines) = self.read_response().await?;
        if expected.contains(&code) {
            Ok(code)
        } else {
            bail!(
                "SMTP server returned {}, expected {:?}: {}",
                code,
                expected,
                lines.join(" | ")
            )
        }
    }

    async fn read_response(&mut self) -> Result<(u16, Vec<String>)> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).await?;
            if read == 0 {
                bail!("SMTP server closed connection");
            }
            let code: u16 = line
                .get(0..3)
                .ok_or_else(|| anyhow!("malformed SMTP response: {}", line.trim_end()))?
                .parse()
                .context("malformed SMTP status code")?;
            let continued = line.as_bytes().get(3) == Some(&b'-');
            lines.push(line.trim_end().to_string());
            if !continued {
                return Ok((code, lines));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_handle_sets_reply_headers() {
        let handle = EmailReplyHandle {
            message_id: Some("<original@example.com>".to_string()),
            account: None,
            mailbox: None,
            uid: None,
        };
        let (in_reply_to, references) = references_with_reply_handle(Some(&handle), None, vec![]);
        assert_eq!(in_reply_to.as_deref(), Some("<original@example.com>"));
        assert_eq!(references, vec!["<original@example.com>"]);
    }

    #[test]
    fn build_message_rejects_header_injection() {
        let request = EmailSendRequest {
            smtp_url: "smtp://localhost:25".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: vec![],
            bcc: vec![],
            subject: "hello\r\nBcc: attacker@example.com".to_string(),
            text: Some("body".to_string()),
            html: None,
            in_reply_to: None,
            references: vec![],
            auth: None,
            message_id: None,
            allow_insecure_auth: false,
            dry_run: true,
        };
        assert!(build_message(&request, "<id@example.com>").is_err());
    }

    #[tokio::test]
    async fn send_email_honors_caller_message_id_in_dry_run() {
        let result = send_email(EmailSendRequest {
            smtp_url: "smtp://localhost:25".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: vec![],
            bcc: vec![],
            subject: "hello".to_string(),
            text: Some("body".to_string()),
            html: None,
            in_reply_to: None,
            references: vec![],
            auth: None,
            message_id: Some("<agentinbox-correlation-1@localhost>".to_string()),
            allow_insecure_auth: false,
            dry_run: true,
        })
        .await
        .unwrap();
        assert_eq!(result.message_id, "<agentinbox-correlation-1@localhost>");
    }

    #[test]
    fn rejects_plaintext_smtp_auth_without_opt_in() {
        let url = parse_smtp_url("smtp://localhost:2525").unwrap();
        let auth = SmtpAuth::Plain {
            username: "user".to_string(),
            password: "password".to_string(),
        };
        assert!(validate_smtp_auth_transport(&url, Some(&auth), false).is_err());
        assert!(validate_smtp_auth_transport(&url, Some(&auth), true).is_ok());
        assert!(validate_smtp_auth_transport(&url, None, false).is_ok());
    }

    #[test]
    fn smtps_and_xoauth2_are_allowed_without_insecure_opt_in() {
        let smtps = parse_smtp_url("smtps://smtp.example.com:465").unwrap();
        let auth = SmtpAuth::Xoauth2 {
            username: "user@hotmail.com".to_string(),
            token: "token".to_string(),
        };
        // OAuth tokens ride inside TLS (STARTTLS upgrade or implicit TLS),
        // so the cleartext opt-in flag does not apply to them.
        assert!(validate_smtp_auth_transport(&smtps, Some(&auth), false).is_ok());
        let smtp = parse_smtp_url("smtp://smtp-mail.outlook.com:587").unwrap();
        assert!(validate_smtp_auth_transport(&smtp, Some(&auth), false).is_ok());
        // Passwords still need the flag even on smtps://-shaped URLs? No:
        // smtps is implicit TLS, so passwords are encrypted there.
        let plain = SmtpAuth::Plain {
            username: "user".to_string(),
            password: "password".to_string(),
        };
        assert!(validate_smtp_auth_transport(&smtps, Some(&plain), false).is_ok());
    }

    #[tokio::test]
    async fn smtp_auth_xoauth2_accepts_235() {
        let (client, server) = tokio::io::duplex(4096);
        let mut smtp = SmtpClient::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let expected = base64::engine::general_purpose::STANDARD
                .encode("user=user@hotmail.com\x01auth=Bearer token1\x01\x01");
            assert_eq!(line, format!("AUTH XOAUTH2 {expected}\r\n"));
            writer.write_all(b"235 2.7.0 Accepted\r\n").await.unwrap();
        });

        smtp.auth_xoauth2("user@hotmail.com", "token1")
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn smtp_auth_xoauth2_aborts_challenge_with_actionable_error() {
        let (client, server) = tokio::io::duplex(4096);
        let mut smtp = SmtpClient::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("AUTH XOAUTH2 "));
            writer.write_all(b"334 ").await.unwrap();
            let challenge = base64::engine::general_purpose::STANDARD
                .encode(r#"{"status":"401","schemes":"XOAUTH2","scope":"https://outlook.office.com/SMTP.Send"}"#);
            writer
                .write_all(format!("{challenge}\r\n").as_bytes())
                .await
                .unwrap();
            // Client aborts SASL with `*`, then the server reports failure.
            let mut abort = String::new();
            reader.read_line(&mut abort).await.unwrap();
            assert_eq!(abort, "*\r\n");
            writer
                .write_all(b"500 Authentication failed\r\n")
                .await
                .unwrap();
        });

        let err = smtp
            .auth_xoauth2("user@hotmail.com", "stale")
            .await
            .unwrap_err();
        server.await.unwrap();
        let message = err.to_string();
        assert!(message.contains("SMTP AUTH XOAUTH2 rejected"));
        assert!(message.contains("SMTP.Send"));
        assert!(message.contains("auth oauth login"));
    }

    #[tokio::test]
    async fn smtp_ehlo_collects_capability_verbs() {
        let (client, server) = tokio::io::duplex(4096);
        let mut smtp = SmtpClient::new(Box::new(client));
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(line.starts_with("EHLO "));
            writer
                .write_all(
                    b"250-smtp.example.com\r\n250-STARTTLS\r\n250-AUTH XOAUTH2 PLAIN\r\n250 OK\r\n",
                )
                .await
                .unwrap();
        });

        let capabilities = smtp.ehlo().await.unwrap();
        server.await.unwrap();
        assert!(capabilities.iter().any(|cap| cap == "STARTTLS"));
        assert!(capabilities.iter().any(|cap| cap == "AUTH"));
        assert!(!capabilities.iter().any(|cap| cap == "250-smtp.example.com"));
    }

    #[test]
    fn parse_smtp_url_accepts_smtps() {
        let url = parse_smtp_url("smtps://smtp.example.com").unwrap();
        assert_eq!(url.scheme(), "smtps");
        assert_eq!(url.port(), None);
        assert!(parse_smtp_url("smtpx://host").is_err());
    }

    #[test]
    fn local_hostname_rejects_newlines() {
        assert_eq!(
            sanitize_local_hostname(Some(
                "localhost\r\nRCPT TO:<attacker@example.com>".to_string()
            )),
            None
        );
        assert_eq!(
            sanitize_local_hostname(Some("smtp-client.example.com".to_string())).as_deref(),
            Some("smtp-client.example.com")
        );
    }
}
