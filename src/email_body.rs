use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use mailparse::{parse_mail, ParsedMail};
use serde::{Deserialize, Serialize};

pub const EMAIL_BODY_SCHEMA_VERSION: u32 = 1;
pub const EMAIL_BODY_PARSER_VERSION: &str = "1";
pub const INLINE_MIME_LIMIT: usize = 4 * 1024 * 1024;
pub const PROVIDER_MIME_LIMIT: usize = 16 * 1024 * 1024;
pub const NORMALIZED_TEXT_LIMIT: usize = 1024 * 1024;
pub const EMAIL_MESSAGE_REF_MAX_BYTES: usize = 8 * 1024;
const EMAIL_MESSAGE_REF_PREFIX: &str = "uxc-email-v1.";
const MIME_PART_LIMIT: usize = 256;
const MIME_DEPTH_LIMIT: usize = 10;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmailSourceRef {
    pub namespace: String,
    pub source_key: String,
    pub spec_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EmailMessageLocator {
    Imap {
        mailbox: String,
        uid: String,
        uidvalidity: Option<u64>,
    },
    Gmail {
        message_id: String,
    },
    Graph {
        immutable_id: String,
    },
    Jmap {
        account_id: String,
        email_id: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmailMessageRef {
    pub source: EmailSourceRef,
    pub locator: EmailMessageLocator,
}

pub(crate) fn encode_email_message_ref(value: &EmailMessageRef) -> Result<String> {
    validate_email_message_ref(value)?;
    let payload = serde_json::to_vec(value)?;
    let encoded = format!(
        "{EMAIL_MESSAGE_REF_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    );
    if encoded.len() > EMAIL_MESSAGE_REF_MAX_BYTES {
        bail!("invalid_input: email message_ref exceeds 8 KiB");
    }
    Ok(encoded)
}

pub(crate) fn decode_email_message_ref(value: &str) -> Result<EmailMessageRef> {
    if value.len() > EMAIL_MESSAGE_REF_MAX_BYTES {
        bail!("invalid_input: email message_ref exceeds 8 KiB");
    }
    let payload = value
        .strip_prefix(EMAIL_MESSAGE_REF_PREFIX)
        .ok_or_else(|| anyhow!("invalid_input: unsupported email message_ref version"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("invalid_input: malformed email message_ref")?;
    let decoded: EmailMessageRef = serde_json::from_slice(&bytes)
        .context("invalid_input: malformed email message_ref payload")?;
    validate_email_message_ref(&decoded)?;
    Ok(decoded)
}

fn validate_email_message_ref(value: &EmailMessageRef) -> Result<()> {
    if value.source.namespace.is_empty()
        || value.source.source_key.is_empty()
        || value.source.spec_key.is_empty()
    {
        bail!("invalid_input: email message_ref source fields must not be empty");
    }
    match &value.locator {
        EmailMessageLocator::Imap { mailbox, uid, .. } => {
            if mailbox.is_empty() || uid.is_empty() || !uid.chars().all(|ch| ch.is_ascii_digit()) {
                bail!("invalid_input: invalid IMAP message_ref");
            }
        }
        EmailMessageLocator::Gmail { message_id } if message_id.is_empty() => {
            bail!("invalid_input: invalid Gmail message_ref")
        }
        EmailMessageLocator::Graph { immutable_id } if immutable_id.is_empty() => {
            bail!("invalid_input: invalid Graph message_ref")
        }
        EmailMessageLocator::Jmap {
            account_id,
            email_id,
        } if account_id.is_empty() || email_id.is_empty() => {
            bail!("invalid_input: invalid JMAP message_ref")
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmailBodyInput {
    InlineMime {
        mime_base64: String,
        original_bytes: usize,
        complete: bool,
    },
    LegacyMime {
        mime_text: String,
        #[serde(default)]
        source_truncated: bool,
    },
    MessageRef {
        message_ref: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmailBodyReadRequest {
    pub input: EmailBodyInput,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailBodyCompleteness {
    Complete,
    Partial,
    Unverified,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmailBodyProvenance {
    pub input_kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmailBodyResult {
    pub schema_version: u32,
    pub parser_version: String,
    pub format: String,
    pub text: String,
    pub bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<usize>,
    pub completeness: EmailBodyCompleteness,
    pub reasons: Vec<String>,
    pub provenance: EmailBodyProvenance,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmailBodyCapability {
    pub schema_version: u32,
    pub parser_version: String,
    pub input_kinds: Vec<String>,
    pub providers: Vec<String>,
    pub inline_mime_max_bytes: usize,
    pub normalized_text_max_bytes: usize,
    pub provider_response_max_bytes: usize,
    pub deadline_seconds: u64,
}

impl Default for EmailBodyCapability {
    fn default() -> Self {
        Self {
            schema_version: EMAIL_BODY_SCHEMA_VERSION,
            parser_version: EMAIL_BODY_PARSER_VERSION.to_string(),
            input_kinds: vec![
                "inline_mime".into(),
                "legacy_mime".into(),
                "message_ref".into(),
            ],
            providers: vec!["imap".into(), "gmail".into(), "graph".into(), "jmap".into()],
            inline_mime_max_bytes: INLINE_MIME_LIMIT,
            normalized_text_max_bytes: NORMALIZED_TEXT_LIMIT,
            provider_response_max_bytes: PROVIDER_MIME_LIMIT,
            deadline_seconds: 30,
        }
    }
}

pub fn read_local(request: EmailBodyReadRequest) -> Result<EmailBodyResult> {
    match request.input {
        EmailBodyInput::InlineMime {
            mime_base64,
            original_bytes,
            complete,
        } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(mime_base64)
                .context("invalid_input: mime_base64 is not strict base64")?;
            if bytes.len() != original_bytes {
                bail!("invalid_input: original_bytes does not match decoded MIME length");
            }
            if bytes.len() > INLINE_MIME_LIMIT {
                bail!("resource_limit: inline MIME exceeds 4 MiB");
            }
            parse_mime(&bytes, complete, "inline_mime", false)
        }
        EmailBodyInput::LegacyMime {
            mime_text,
            source_truncated,
        } => {
            enforce_mime_limit(mime_text.len(), INLINE_MIME_LIMIT, "local")?;
            parse_mime(mime_text.as_bytes(), !source_truncated, "legacy_mime", true)
        }
        EmailBodyInput::MessageRef { .. } => {
            bail!("capability_unavailable: message_ref provider retrieval is unavailable")
        }
    }
}

pub fn parse_mime(
    bytes: &[u8],
    source_complete: bool,
    input_kind: &str,
    legacy: bool,
) -> Result<EmailBodyResult> {
    enforce_mime_limit(bytes.len(), PROVIDER_MIME_LIMIT, "provider")?;
    let parsed = parse_mail(bytes).context("parse_failed: invalid MIME message")?;
    let mut part_count = 0;
    validate_mime_tree(&parsed, 0, &mut part_count)?;
    let mut reasons = Vec::new();
    let mut text = select_body(&parsed, &mut reasons)?;
    if text.is_empty() {
        reasons.push("empty_body".into());
    }
    if text.len() > NORMALIZED_TEXT_LIMIT {
        text.truncate(floor_char_boundary(&text, NORMALIZED_TEXT_LIMIT));
        reasons.push("resource_limit".into());
    }
    let completeness = if !source_complete || reasons.iter().any(|r| r == "resource_limit") {
        if !source_complete {
            reasons.push("source_truncated".into());
        }
        EmailBodyCompleteness::Partial
    } else if legacy || !reasons.is_empty() {
        if legacy {
            reasons.push("legacy_text_input".into());
        }
        EmailBodyCompleteness::Unverified
    } else {
        EmailBodyCompleteness::Complete
    };
    reasons.sort();
    reasons.dedup();
    let output_bytes = text.len();
    Ok(EmailBodyResult {
        schema_version: EMAIL_BODY_SCHEMA_VERSION,
        parser_version: EMAIL_BODY_PARSER_VERSION.into(),
        format: "text".into(),
        text,
        bytes: output_bytes,
        total_bytes: (completeness == EmailBodyCompleteness::Complete).then_some(output_bytes),
        completeness,
        reasons,
        provenance: EmailBodyProvenance {
            input_kind: input_kind.into(),
        },
    })
}

fn enforce_mime_limit(bytes: usize, limit: usize, scope: &str) -> Result<()> {
    if bytes > limit {
        bail!("resource_limit: MIME input exceeds {scope} limit");
    }
    Ok(())
}

fn validate_mime_tree(part: &ParsedMail<'_>, depth: usize, part_count: &mut usize) -> Result<()> {
    if depth > MIME_DEPTH_LIMIT {
        bail!("resource_limit: MIME nesting exceeds limit");
    }
    *part_count += 1;
    if *part_count > MIME_PART_LIMIT {
        bail!("resource_limit: MIME part count exceeds limit");
    }
    for child in &part.subparts {
        validate_mime_tree(child, depth + 1, part_count)?;
    }
    Ok(())
}

fn select_body(part: &ParsedMail<'_>, reasons: &mut Vec<String>) -> Result<String> {
    let disposition = part.get_content_disposition();
    if disposition.disposition == mailparse::DispositionType::Attachment
        || disposition.params.contains_key("filename")
    {
        return Ok(String::new());
    }
    if part.subparts.is_empty() {
        let mime = part.ctype.mimetype.to_ascii_lowercase();
        return match mime.as_str() {
            "text/plain" => part
                .get_body()
                .context("parse_failed: cannot decode text/plain body"),
            "text/html" => {
                let html = part
                    .get_body()
                    .context("parse_failed: cannot decode text/html body")?;
                Ok(html2text::from_read(html.as_bytes(), 120))
            }
            _ => Ok(String::new()),
        };
    }
    let mime = part.ctype.mimetype.to_ascii_lowercase();
    if mime == "multipart/alternative" {
        for child in &part.subparts {
            if child.ctype.mimetype.eq_ignore_ascii_case("text/plain") {
                let body = select_body(child, reasons)?;
                if !body.is_empty() {
                    return Ok(body);
                }
            }
        }
        for child in &part.subparts {
            if child.ctype.mimetype.eq_ignore_ascii_case("text/html") {
                let body = select_body(child, reasons)?;
                if !body.is_empty() {
                    return Ok(body);
                }
            }
        }
        reasons.push("unsupported_multipart".into());
        return Ok(String::new());
    }
    if mime == "multipart/related" {
        return part
            .subparts
            .first()
            .ok_or_else(|| anyhow!("parse_failed: empty multipart/related"))
            .and_then(|child| select_body(child, reasons));
    }
    if !mime.starts_with("multipart/") {
        reasons.push("unsupported_structure".into());
    }
    let mut bodies = Vec::new();
    for child in &part.subparts {
        let body = select_body(child, reasons)?;
        if !body.is_empty() {
            bodies.push(body);
        }
    }
    Ok(bodies.join("\n"))
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_html_without_attachments() {
        let mime = b"Content-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hello <b>world</b></p><script>bad()</script>\r\n--x\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=a.txt\r\n\r\nsecret\r\n--x--\r\n";
        let result = parse_mime(mime, true, "inline_mime", false).unwrap();
        assert!(result.text.contains("Hello"));
        assert!(!result.text.contains("secret"));
        assert!(!result.text.contains("bad()"));
    }

    #[test]
    fn alternative_prefers_plain_and_ignores_attachment() {
        let mime = b"Content-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>html fallback</p>\r\n--x\r\nContent-Type: text/plain\r\n\r\nplain preferred\r\n--x\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=secret.txt\r\n\r\nattachment\r\n--x--\r\n";
        let result = parse_mime(mime, true, "inline_mime", false).unwrap();
        assert_eq!(result.text.trim(), "plain preferred");
        assert!(!result.text.contains("attachment"));
        assert!(!result.text.contains("html fallback"));
    }

    #[test]
    fn alternative_uses_html_fallback() {
        let mime = b"Content-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: application/octet-stream\r\n\r\nignored\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>html fallback</p>\r\n--x--\r\n";
        let result = parse_mime(mime, true, "inline_mime", false).unwrap();
        assert!(result.text.contains("html fallback"));
        assert!(!result.text.contains("ignored"));
    }

    #[test]
    fn provider_and_local_mime_budgets_are_distinct() {
        assert!(enforce_mime_limit(INLINE_MIME_LIMIT + 1, PROVIDER_MIME_LIMIT, "provider").is_ok());
        assert!(
            enforce_mime_limit(PROVIDER_MIME_LIMIT + 1, PROVIDER_MIME_LIMIT, "provider")
                .unwrap_err()
                .to_string()
                .starts_with("resource_limit:")
        );
        assert!(enforce_mime_limit(INLINE_MIME_LIMIT + 1, INLINE_MIME_LIMIT, "local").is_err());
    }

    #[test]
    fn rejects_total_part_count_across_nested_multiparts() {
        let mut mime = String::from("Content-Type: multipart/mixed; boundary=outer\r\n\r\n");
        for group in 0..16 {
            mime.push_str("--outer\r\nContent-Type: multipart/mixed; boundary=inner");
            mime.push_str(&group.to_string());
            mime.push_str("\r\n\r\n");
            for part in 0..16 {
                mime.push_str("--inner");
                mime.push_str(&group.to_string());
                mime.push_str("\r\nContent-Type: text/plain\r\n\r\n");
                mime.push_str(&part.to_string());
                mime.push_str("\r\n");
            }
            mime.push_str("--inner");
            mime.push_str(&group.to_string());
            mime.push_str("--\r\n");
        }
        mime.push_str("--outer--\r\n");
        let error = parse_mime(mime.as_bytes(), true, "message_ref", false).unwrap_err();
        assert!(error
            .to_string()
            .starts_with("resource_limit: MIME part count"));
    }

    #[test]
    fn truncates_normalized_text_at_utf8_boundary() {
        let body = "a".repeat(NORMALIZED_TEXT_LIMIT - 1) + "😀tail";
        let mime = format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{body}");
        let result = parse_mime(mime.as_bytes(), true, "message_ref", false).unwrap();
        assert_eq!(result.text.len(), NORMALIZED_TEXT_LIMIT - 1);
        assert!(result.text.is_char_boundary(result.text.len()));
        assert_eq!(result.completeness, EmailBodyCompleteness::Partial);
        assert!(result.reasons.contains(&"resource_limit".to_string()));
    }

    #[test]
    fn legacy_is_never_complete() {
        let result = read_local(EmailBodyReadRequest {
            input: EmailBodyInput::LegacyMime {
                mime_text: "Content-Type: text/plain\r\n\r\nhello".into(),
                source_truncated: false,
            },
        })
        .unwrap();
        assert_eq!(result.completeness, EmailBodyCompleteness::Unverified);
    }

    #[test]
    fn validates_inline_length() {
        let error = read_local(EmailBodyReadRequest {
            input: EmailBodyInput::InlineMime {
                mime_base64: "aGVsbG8=".into(),
                original_bytes: 4,
                complete: true,
            },
        })
        .unwrap_err();
        assert!(error.to_string().contains("original_bytes"));
    }

    #[test]
    fn message_ref_round_trips() {
        let value = EmailMessageRef {
            source: EmailSourceRef {
                namespace: "n".into(),
                source_key: "s".into(),
                spec_key: "k".into(),
            },
            locator: EmailMessageLocator::Jmap {
                account_id: "a".into(),
                email_id: "e".into(),
            },
        };
        assert_eq!(
            decode_email_message_ref(&encode_email_message_ref(&value).unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn message_ref_rejects_invalid_long_versioned_and_unknown_fields() {
        for value in [
            "not-a-reference".to_string(),
            "uxc-email-v2.e30".to_string(),
            format!("{EMAIL_MESSAGE_REF_PREFIX}***"),
            "x".repeat(EMAIL_MESSAGE_REF_MAX_BYTES + 1),
        ] {
            assert!(decode_email_message_ref(&value)
                .unwrap_err()
                .to_string()
                .starts_with("invalid_input:"));
        }

        let payload = serde_json::json!({
            "source": {
                "namespace": "n",
                "source_key": "s",
                "spec_key": "k",
                "unknown": true
            },
            "locator": {
                "provider": "gmail",
                "message_id": "m"
            }
        });
        let encoded = format!(
            "{EMAIL_MESSAGE_REF_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).unwrap())
        );
        assert!(decode_email_message_ref(&encoded)
            .unwrap_err()
            .to_string()
            .starts_with("invalid_input:"));
    }
}
