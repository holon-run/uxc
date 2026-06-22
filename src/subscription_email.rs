use crate::auth::Profile;
use crate::daemon::{SubscribeStartRequest, SubscriptionEventRecorder};
use crate::daemon_log::redact_endpoint;
use anyhow::{anyhow, bail, Context, Result};
use rustls_pki_types::ServerName;
use serde_json::{json, Map, Value};
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
const EMAIL_SNIPPET_CHARS: usize = 512;
const EMAIL_RAW_INLINE_BYTES: usize = 32 * 1024;

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type BoxedIo = Box<dyn AsyncReadWrite>;
type BoxFutureResult<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

#[derive(Debug, Clone)]
pub struct EmailImapIdleRuntimeConfig {
    pub endpoint: String,
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    pub username: String,
    pub password: String,
    pub mailbox: String,
    pub account: Option<String>,
    pub initial_fetch_limit: usize,
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
    let initial_fetch_limit = args
        .and_then(|args| args.get("initial_fetch_limit"))
        .and_then(Value::as_u64)
        .unwrap_or(25)
        .min(100) as usize;
    let username = resolve_first_profile_field(auth_profile, &["username", "user", "email"])?
        .ok_or_else(|| {
            anyhow!("email-imap-idle auth profile requires username/user/email field")
        })?;
    let password =
        resolve_first_profile_field(auth_profile, &["password", "app_password", "secret"])?
            .ok_or_else(|| {
                anyhow!("email-imap-idle auth profile requires password/app_password/secret field")
            })?;
    Ok(EmailImapIdleRuntimeConfig {
        endpoint: request.endpoint.clone(),
        host,
        port,
        use_tls,
        username,
        password,
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
    let mut delay_secs = 1u64;
    loop {
        match run_email_imap_idle_session_once(&config, recorder, stop_rx, connect_imap).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                let message = err.to_string();
                recorder
                    .emit(
                        "email_imap_idle",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
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

async fn run_email_imap_idle_session_once<R, C>(
    config: &EmailImapIdleRuntimeConfig,
    recorder: &mut R,
    stop_rx: &mut watch::Receiver<bool>,
    connector: C,
) -> Result<()>
where
    R: SubscriptionEventRecorder,
    C: Fn(EmailImapIdleRuntimeConfig) -> BoxFutureResult<ImapConnection> + Copy,
{
    if *stop_rx.borrow() {
        close_email_subscription(recorder, "stopped").await?;
        return Ok(());
    }
    let mut conn = connector(config.clone()).await?;
    conn.expect_greeting().await?;
    conn.command_ok(&format!(
        "LOGIN {} {}",
        quote_imap_string(&config.username),
        quote_imap_string(&config.password)
    ))
    .await?;
    conn.command_ok(&format!("SELECT {}", quote_imap_string(&config.mailbox)))
        .await?;
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

    let mut last_seen_uid = emit_recent_messages(config, &mut conn, recorder).await?;

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
                    if let Some(uid) = emit_messages(config, messages, recorder).await? {
                        last_seen_uid = Some(last_seen_uid.map_or(uid, |last| last.max(uid)));
                    }
                }
            }
        }
    }
}

fn connect_imap(config: EmailImapIdleRuntimeConfig) -> BoxFutureResult<ImapConnection> {
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
) -> Result<Option<u64>>
where
    R: SubscriptionEventRecorder,
{
    let messages = conn.fetch_recent(config.initial_fetch_limit).await?;
    emit_messages(config, messages, recorder).await
}

async fn emit_messages<R>(
    config: &EmailImapIdleRuntimeConfig,
    messages: Vec<ImapFetchedMessage>,
    recorder: &mut R,
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
                Some(build_email_event(config, &message)),
                None,
            )
            .await?;
    }
    Ok(max_uid)
}

fn build_email_event(config: &EmailImapIdleRuntimeConfig, message: &ImapFetchedMessage) -> Value {
    let raw_len = message.raw.len();
    let raw_text = String::from_utf8_lossy(&message.raw);
    let headers = parse_email_headers(&raw_text);
    let subject = header_value(&headers, "subject");
    let message_id = header_value(&headers, "message-id").unwrap_or_else(|| message.uid.clone());
    let thread_id =
        header_value(&headers, "references").or_else(|| header_value(&headers, "in-reply-to"));
    let account = config.account.as_deref().unwrap_or(&config.username);
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
            "attachments": [],
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

fn quote_imap_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
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

pub struct ImapConnection {
    reader: BufReader<ReadHalf<BoxedIo>>,
    writer: WriteHalf<BoxedIo>,
    next_tag: u64,
}

impl ImapConnection {
    fn new(io: BoxedIo) -> Self {
        let (reader, writer) = tokio::io::split(io);
        Self {
            reader: BufReader::new(reader),
            writer,
            next_tag: 1,
        }
    }

    async fn expect_greeting(&mut self) -> Result<()> {
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

    async fn command_ok(&mut self, command: &str) -> Result<Vec<String>> {
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

    async fn logout(&mut self) -> Result<()> {
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
            username: "agent@example.com".to_string(),
            password: "secret".to_string(),
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
        let event = build_email_event(&config, &message);
        assert_eq!(event["type"], "email_event");
        assert_eq!(event["provider"], "imap");
        assert_eq!(event["message"]["uid"], "42");
        assert_eq!(event["message"]["subject"], "Hi");
        assert_eq!(event["raw"]["mime_truncated"], false);
        assert_eq!(event["reply_handle"]["message_id"], "<m1@example.com>");
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
}
