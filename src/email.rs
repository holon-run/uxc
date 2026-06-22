use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use url::Url;

use crate::auth;

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

#[derive(Debug, Clone)]
struct SmtpAuth {
    username: String,
    password: String,
}

pub async fn send_email(request: EmailSendRequest) -> Result<EmailSendResult> {
    validate_request(&request)?;
    let smtp_url = parse_smtp_url(&request.smtp_url)?;
    let message_id = format!(
        "<uxc-{}-{}@localhost>",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
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
        "smtps" => bail!("smtps:// is not supported yet; use smtp:// with a trusted local relay"),
        other => bail!("unsupported SMTP URL scheme '{}'; expected smtp://", other),
    }
}

fn validate_smtp_auth_transport(
    url: &Url,
    auth: Option<&SmtpAuth>,
    allow_insecure_auth: bool,
) -> Result<()> {
    if auth.is_some() && url.scheme() == "smtp" && !allow_insecure_auth {
        bail!(
            "refusing to send SMTP credentials over unencrypted smtp://; configure a trusted local relay without --auth, or pass --allow-insecure-auth to acknowledge the cleartext credential risk"
        );
    }
    Ok(())
}

async fn resolve_smtp_auth(
    endpoint: &str,
    explicit_auth: Option<String>,
) -> Result<Option<SmtpAuth>> {
    let Some(profile) = auth::resolve_auth_for_endpoint(endpoint, explicit_auth)? else {
        return Ok(None);
    };
    let username = first_field(&profile, &["username", "user", "account"])?
        .or_else(|| profile.name.clone())
        .ok_or_else(|| anyhow!("SMTP auth profile requires username/user/account field"))?;
    let password = first_field(&profile, &["password", "secret"])?
        .ok_or_else(|| anyhow!("SMTP auth profile requires password or secret field"))?;
    Ok(Some(SmtpAuth { username, password }))
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
    let port = url.port_or_known_default().unwrap_or(25);
    let stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("failed to connect SMTP server {}:{}", host, port))?;
    let mut client = SmtpClient::new(stream);
    client.expect_code(&[220]).await?;
    client
        .command(&format!("EHLO {}\r\n", local_hostname()), &[250])
        .await?;
    if let Some(auth) = auth {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("\0{}\0{}", auth.username, auth.password));
        client
            .command(&format!("AUTH PLAIN {}\r\n", encoded), &[235])
            .await?;
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
    reader: BufReader<TcpStream>,
}

impl SmtpClient {
    fn new(stream: TcpStream) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.reader.get_mut().write_all(bytes).await?;
        self.reader.get_mut().flush().await?;
        Ok(())
    }

    async fn command(&mut self, command: &str, expected: &[u16]) -> Result<u16> {
        self.write_all(command.as_bytes()).await?;
        self.expect_code(expected).await
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
            allow_insecure_auth: false,
            dry_run: true,
        };
        assert!(build_message(&request, "<id@example.com>").is_err());
    }

    #[test]
    fn rejects_plaintext_smtp_auth_without_opt_in() {
        let url = parse_smtp_url("smtp://localhost:2525").unwrap();
        let auth = SmtpAuth {
            username: "user".to_string(),
            password: "password".to_string(),
        };
        assert!(validate_smtp_auth_transport(&url, Some(&auth), false).is_err());
        assert!(validate_smtp_auth_transport(&url, Some(&auth), true).is_ok());
        assert!(validate_smtp_auth_transport(&url, None, false).is_ok());
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
