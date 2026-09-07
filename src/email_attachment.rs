//! Provider-neutral MIME attachment metadata extraction for email events.
//!
//! This module contains a small hand-written, bounded MIME walker (the
//! repository intentionally avoids a mail parser dependency) that extracts
//! attachment metadata from raw RFC 5322 messages. Leaf parts are numbered
//! with IMAP `BODY[<section>]` semantics (RFC 3501) so each metadata entry
//! can later be retrieved lazily by section without provider-specific logic.
//!
//! Events only ever carry metadata and an opaque retrieval handle; attachment
//! content is never inlined into event payloads.

use base64::Engine;
use serde_json::{json, Map, Value};

/// Maximum MIME nesting depth accepted by the walker.
pub const MIME_MAX_DEPTH: usize = 10;
/// Maximum number of parts visited per message.
pub const MIME_MAX_PARTS: usize = 256;

/// Provider-neutral attachment metadata extracted from a MIME part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailAttachmentMeta {
    /// Provider part identifier. For IMAP this is the RFC 3501 section
    /// number of the part inside the message.
    pub id: String,
    /// IMAP `BODY[<section>]` section number; equals `id` for IMAP events.
    pub section: String,
    /// Decoded filename (RFC 2047 / RFC 2231 aware); `None` when absent.
    pub filename: Option<String>,
    /// Lowercase `type/subtype`; defaults to `text/plain` per RFC 2045.
    pub content_type: Option<String>,
    /// Decoded content byte length; `None` when it cannot be determined.
    pub size: Option<u64>,
    /// `attachment` / `inline` when the header says so, else `None`.
    pub disposition: Option<String>,
    /// `Content-ID` header value (e.g. `<logo@cid>`) when present.
    pub content_id: Option<String>,
}

/// Context used to stamp opaque IMAP attachment handles on event entries.
pub struct ImapAttachmentContext<'a> {
    /// Credential-free reconnection endpoint (`imap://host:port` or
    /// `imaps://host:port`). Never carries userinfo.
    pub endpoint: &'a str,
    pub account: &'a str,
    pub mailbox: &'a str,
    pub message_id: &'a str,
    pub uid: &'a str,
    /// Mailbox UIDVALIDITY captured at SELECT time, used to detect stale
    /// UIDs during lazy retrieval.
    pub uidvalidity: Option<u64>,
    /// Auth profile name referenced by the handle (never credentials).
    pub auth_profile: Option<&'a str>,
}

/// Parse raw MIME bytes and return attachment metadata entries.
///
/// Walking is bounded by [`MIME_MAX_DEPTH`] and [`MIME_MAX_PARTS`]. Parts
/// that are classified as message body (e.g. `text/plain` / `text/html`
/// inside `multipart/alternative`) are not reported.
pub fn parse_mime_attachments(raw: &[u8]) -> Vec<EmailAttachmentMeta> {
    let text = String::from_utf8_lossy(raw);
    let (header_block, body) = split_header_block(&text);
    let headers = parse_header_block(header_block);
    let mut out = Vec::new();
    let mut parts_seen = 0usize;
    walk_mime_part("", &headers, body, None, 0, &mut parts_seen, &mut out);
    out
}

/// Parse raw MIME bytes and return event-ready attachment JSON entries with
/// opaque IMAP retrieval handles.
pub fn imap_message_attachments(raw: &[u8], ctx: &ImapAttachmentContext<'_>) -> Vec<Value> {
    parse_mime_attachments(raw)
        .into_iter()
        .map(|meta| attachment_entry(&meta, imap_attachment_handle(ctx, &meta.section)))
        .collect()
}

fn attachment_entry(meta: &EmailAttachmentMeta, handle: Value) -> Value {
    json!({
        "id": meta.id,
        "filename": meta.filename,
        "content_type": meta.content_type,
        "size": meta.size,
        "disposition": meta.disposition,
        "content_id": meta.content_id,
        "handle": handle,
    })
}

fn imap_attachment_handle(ctx: &ImapAttachmentContext<'_>, section: &str) -> Value {
    let mut handle = Map::new();
    handle.insert("type".into(), json!("email_attachment"));
    handle.insert("provider".into(), json!("imap"));
    handle.insert("endpoint".into(), json!(ctx.endpoint));
    handle.insert("account".into(), json!(ctx.account));
    handle.insert("mailbox".into(), json!(ctx.mailbox));
    handle.insert("message_id".into(), json!(ctx.message_id));
    handle.insert("uid".into(), json!(ctx.uid));
    if let Some(uidvalidity) = ctx.uidvalidity {
        handle.insert("uidvalidity".into(), json!(uidvalidity));
    }
    if let Some(auth_profile) = ctx.auth_profile {
        handle.insert("auth_profile".into(), json!(auth_profile));
    }
    handle.insert("part".into(), json!({ "section": section }));
    Value::Object(handle)
}

fn walk_mime_part(
    section: &str,
    headers: &[(String, String)],
    body: &str,
    parent_subtype: Option<&str>,
    depth: usize,
    parts_seen: &mut usize,
    out: &mut Vec<EmailAttachmentMeta>,
) {
    if depth > MIME_MAX_DEPTH || *parts_seen >= MIME_MAX_PARTS {
        return;
    }
    let (content_type, params) = content_type_and_params(headers);
    let is_multipart = content_type.starts_with("multipart/");
    if is_multipart {
        let subtype = content_type
            .split_once('/')
            .map(|(_, sub)| sub)
            .unwrap_or("");
        if subtype.is_empty() {
            return;
        }
        let Some((_, boundary)) = params.iter().find(|(key, _)| key == "boundary") else {
            return;
        };
        if boundary.is_empty() {
            return;
        }
        let children = split_multipart_body(body, boundary);
        for (index, child) in children.iter().enumerate() {
            if *parts_seen >= MIME_MAX_PARTS {
                return;
            }
            *parts_seen += 1;
            let child_section = if section.is_empty() {
                (index + 1).to_string()
            } else {
                format!("{}.{}", section, index + 1)
            };
            let (child_headers_block, child_body) = split_header_block(child);
            let child_headers = parse_header_block(child_headers_block);
            walk_mime_part(
                &child_section,
                &child_headers,
                child_body,
                Some(subtype),
                depth + 1,
                parts_seen,
                out,
            );
        }
        return;
    }
    // RFC 3501: the body of a non-multipart message is section 1.
    let section = if section.is_empty() { "1" } else { section };
    if let Some(meta) = classify_leaf_attachment(section, headers, body, parent_subtype) {
        out.push(meta);
    }
}

/// Classify a leaf MIME part as an attachment (or `None` for message body).
///
/// Rules:
/// - `Content-Disposition: attachment` is always an attachment.
/// - `Content-Disposition: inline` is an inline attachment when it carries a
///   filename or a Content-ID (typical for CID-referenced images).
/// - A part with any resolvable filename is an attachment.
/// - `text/*` parts without filename are message body.
/// - Inside `multipart/alternative`, unnamed non-text parts are treated as
///   body candidates instead of attachments.
/// - Any other unnamed non-text part (e.g. bare `application/octet-stream`
///   in `multipart/mixed`) is an attachment with `filename: null`.
fn classify_leaf_attachment(
    section: &str,
    headers: &[(String, String)],
    body: &str,
    parent_subtype: Option<&str>,
) -> Option<EmailAttachmentMeta> {
    let (content_type, ct_params) = content_type_and_params(headers);
    let disposition_kind = header_value_of(headers, "content-disposition")
        .and_then(|value| value.split(';').next())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token == "attachment" || token == "inline");
    let filename = attachment_filename(headers, &ct_params).filter(|name| !name.is_empty());
    let content_id = header_value_of(headers, "content-id").map(|v| v.trim().to_string());
    let is_attachment = match disposition_kind.as_deref() {
        Some("attachment") => true,
        Some("inline") => filename.is_some() || content_id.is_some(),
        _ => {
            if filename.is_some() {
                true
            } else {
                !(content_type.starts_with("text/") || parent_subtype == Some("alternative"))
            }
        }
    };
    if !is_attachment {
        return None;
    }
    let size = decoded_body_size(headers, body);
    Some(EmailAttachmentMeta {
        id: section.to_string(),
        section: section.to_string(),
        filename,
        content_type: Some(content_type),
        size,
        disposition: disposition_kind,
        content_id,
    })
}

fn attachment_filename(
    headers: &[(String, String)],
    ct_params: &[(String, String)],
) -> Option<String> {
    if let Some(value) = header_value_of(headers, "content-disposition") {
        let (_, params) = split_header_params(value);
        if let Some(name) = decode_header_param(&params, "filename") {
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    // Legacy RFC 2045 `name` parameter on Content-Type.
    decode_header_param(ct_params, "name")
}

/// Decoded byte length of a part body, or `None` when the encoding is
/// unreadable (e.g. malformed base64). Base64 and quoted-printable sizes are
/// exact; passthrough sizes operate on the lossy UTF-8 projection of the
/// raw message, which is exact for textual content.
fn decoded_body_size(headers: &[(String, String)], body: &str) -> Option<u64> {
    let encoding = header_value_of(headers, "content-transfer-encoding")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match encoding.as_str() {
        "base64" => decode_base64_lenient(body).map(|bytes| bytes.len() as u64),
        "quoted-printable" => Some(decode_quoted_printable(body).len() as u64),
        _ => Some(body.len() as u64),
    }
}

/// Split a raw message (or MIME part) into its header block and body at the
/// first blank line. Handles both CRLF and bare LF separators.
fn split_header_block(text: &str) -> (&str, &str) {
    if let Some(index) = text.find("\r\n\r\n") {
        (&text[..index], &text[index + 4..])
    } else if let Some(index) = text.find("\n\n") {
        (&text[..index], &text[index + 2..])
    } else {
        (text, "")
    }
}

/// Parse a header block into unfolded `(lowercase-name, value)` pairs.
fn parse_header_block(block: &str) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        if (line.starts_with(' ') || line.starts_with('\t')) && !headers.is_empty() {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    headers
}

fn header_value_of<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Content type (lowercase `type/subtype`, defaulting to `text/plain` per
/// RFC 2045) together with its raw parameters.
fn content_type_and_params(headers: &[(String, String)]) -> (String, Vec<(String, String)>) {
    match header_value_of(headers, "content-type") {
        Some(value) => split_header_params(value),
        None => ("text/plain".to_string(), Vec::new()),
    }
}

/// Split a structured header value into its lowercase main value and raw
/// parameters, honoring quoted strings.
fn split_header_params(value: &str) -> (String, Vec<(String, String)>) {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => {
                current.push(ch);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ';' if !in_quotes => segments.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    segments.push(current);

    let main = segments
        .first()
        .map(|segment| segment.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let mut params = Vec::new();
    for segment in segments.iter().skip(1) {
        let Some((key, raw)) = segment.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let mut value = raw.trim();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = &value[1..value.len() - 1];
            params.push((key, unescape_quoted(value)));
        } else {
            params.push((key, value.to_string()));
        }
    }
    (main, params)
}

fn unescape_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Decode an RFC 2231 encoded parameter (`name*` or `name*0*` continuations),
/// falling back to the plain `name` parameter with RFC 2047 decoding.
fn decode_header_param(params: &[(String, String)], name: &str) -> Option<String> {
    if let Some((_, value)) = params.iter().find(|(key, _)| key == &format!("{name}*")) {
        return Some(decode_rfc2231_extended_value(value));
    }
    let mut segments: Vec<(usize, bool, &str)> = Vec::new();
    for (key, value) in params {
        let Some(after) = key.strip_prefix(name) else {
            continue;
        };
        let Some(digits) = after.strip_prefix('*') else {
            continue;
        };
        let (num, extended) = match digits.strip_suffix('*') {
            Some(num) => (num, true),
            None => (digits, false),
        };
        let Ok(index) = num.parse::<usize>() else {
            continue;
        };
        segments.push((index, extended, value.as_str()));
    }
    if segments.is_empty() {
        return params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| decode_rfc2047(value));
    }
    segments.sort_by_key(|(index, _, _)| *index);
    let mut charset: Option<String> = None;
    let mut bytes: Vec<u8> = Vec::new();
    for (position, (_, extended, value)) in segments.iter().enumerate() {
        if *extended && position == 0 {
            if let Some((parsed_charset, decoded)) = split_rfc2231_charset(value) {
                charset = Some(parsed_charset);
                bytes.extend_from_slice(&decoded);
                continue;
            }
        }
        if *extended {
            bytes.extend(percent_decode(value));
        } else {
            bytes.extend_from_slice(value.as_bytes());
        }
    }
    Some(decode_charset_bytes(
        charset.as_deref().unwrap_or("utf-8"),
        &bytes,
    ))
}

fn decode_rfc2231_extended_value(value: &str) -> String {
    match split_rfc2231_charset(value) {
        Some((charset, bytes)) => decode_charset_bytes(&charset, &bytes),
        None => decode_charset_bytes("utf-8", &percent_decode(value)),
    }
}

/// Split `charset'lang'percent-encoded` into charset and decoded bytes.
fn split_rfc2231_charset(value: &str) -> Option<(String, Vec<u8>)> {
    let first = value.find('\'')?;
    let rest = &value[first + 1..];
    let second = rest.find('\'')? + first + 1;
    let charset = if value[..first].is_empty() {
        "utf-8".to_string()
    } else {
        value[..first].to_ascii_lowercase()
    };
    Some((charset, percent_decode(&value[second + 1..])))
}

fn percent_decode(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_digit(byte: u8) -> Option<u32> {
    (byte as char).to_digit(16)
}

/// Decode RFC 2047 encoded words (`=?charset?B/Q?...?=`) in a header value.
/// Linear whitespace between adjacent encoded words is dropped per RFC 2047.
fn decode_rfc2047(value: &str) -> String {
    let mut out = String::new();
    let mut plain = String::new();
    let mut rest = value;
    let mut last_was_encoded = false;
    while let Some(start) = rest.find("=?") {
        match decode_encoded_word_at(&rest[start..]) {
            Some((decoded, consumed)) => {
                let between = &rest[..start];
                let drop_between = last_was_encoded
                    && !between.is_empty()
                    && between.chars().all(|ch| ch == ' ' || ch == '\t');
                if !drop_between {
                    plain.push_str(between);
                }
                out.push_str(&plain);
                plain.clear();
                out.push_str(&decoded);
                last_was_encoded = true;
                rest = &rest[start + consumed..];
            }
            None => {
                plain.push_str(&rest[..start + 2]);
                last_was_encoded = false;
                rest = &rest[start + 2..];
            }
        }
    }
    out.push_str(&plain);
    out.push_str(rest);
    out
}

/// Decode one encoded word starting at `input` (which begins with `=?`).
/// Returns the decoded text and the number of bytes consumed.
fn decode_encoded_word_at(input: &str) -> Option<(String, usize)> {
    let after_marker = &input[2..];
    let charset_end = after_marker.find('?')?;
    let charset = &after_marker[..charset_end];
    let after_charset = &after_marker[charset_end + 1..];
    let encoding = after_charset.chars().next()?;
    let after_encoding = &after_charset[encoding.len_utf8()..];
    let after_encoding = after_encoding.strip_prefix('?')?;
    let text_end = after_encoding.find("?=")?;
    let encoded_text = &after_encoding[..text_end];
    let bytes = match encoding.to_ascii_uppercase() {
        'B' => decode_base64_lenient(encoded_text)?,
        'Q' => decode_q_encoded(encoded_text),
        _ => return None,
    };
    let consumed = 2 + charset_end + 1 + encoding.len_utf8() + 1 + text_end + 2;
    Some((decode_charset_bytes(charset, &bytes), consumed))
}

/// Decode RFC 2047 Q encoding: `_` is space and `=XX` are hex escapes.
fn decode_q_encoded(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => out.push(b' '),
            b'=' if i + 2 < bytes.len() => {
                if let (Some(high), Some(low)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2]))
                {
                    out.push((high * 16 + low) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    out
}

/// Split a multipart body into child parts at `--boundary` delimiter lines.
/// Preamble before the first delimiter and the epilogue after the close
/// delimiter are ignored. Child parts are line-normalized to LF.
/// Per RFC 2046, the line break preceding a delimiter belongs to the
/// delimiter, so it is stripped from the end of each child part.
fn split_multipart_body(body: &str, boundary: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in body.lines() {
        let is_delimiter =
            line.starts_with("--") && line[2..].starts_with(boundary) && !boundary.is_empty();
        if is_delimiter {
            let after = &line[2 + boundary.len()..];
            let closes = after.starts_with("--");
            let pads = after.is_empty() || after.starts_with(' ') || after.starts_with('\t');
            if closes || pads {
                if let Some(mut part) = current.take() {
                    if part.ends_with('\n') {
                        part.pop();
                    }
                    parts.push(part);
                }
                if closes {
                    break;
                }
                current = Some(String::new());
                continue;
            }
        }
        if let Some(part) = current.as_mut() {
            part.push_str(line);
            part.push('\n');
        }
    }
    if let Some(mut part) = current {
        if part.ends_with('\n') {
            part.pop();
        }
        parts.push(part);
    }
    parts
}

fn decode_base64_lenient(input: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
    let cleaned: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
    if let Ok(bytes) = STANDARD.decode(&cleaned) {
        return Some(bytes);
    }
    STANDARD_NO_PAD.decode(cleaned.trim_end_matches('=')).ok()
}

fn decode_quoted_printable(body: &str) -> Vec<u8> {
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'=' {
            // Soft line breaks (trailing `=` before a line ending).
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            if i + 2 < bytes.len() && bytes[i + 1] == b'\r' && bytes[i + 2] == b'\n' {
                i += 3;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'\r' {
                i += 2;
                continue;
            }
            if i + 2 < bytes.len() {
                if let (Some(high), Some(low)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2]))
                {
                    out.push((high * 16 + low) as u8);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(byte);
        i += 1;
    }
    out
}

/// Decode bytes in a MIME charset label. UTF-8/US-ASCII use lossy UTF-8;
/// ISO-8859-1 maps bytes directly to code points; unknown charsets fall
/// back to lossy UTF-8.
fn decode_charset_bytes(charset: &str, bytes: &[u8]) -> String {
    match charset.to_ascii_lowercase().as_str() {
        "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" => {
            bytes.iter().map(|byte| *byte as char).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_email_event_attachments(raw: &str) -> Vec<EmailAttachmentMeta> {
        parse_mime_attachments(raw.as_bytes())
    }

    #[test]
    fn parses_multipart_mixed_named_and_unnamed_attachments() {
        let raw = concat!(
            "From: sender@example.com\r\n",
            "Subject: Contract\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n",
            "\r\n",
            "preamble is ignored\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "\r\n",
            "See attached.\r\n",
            "--BOUND\r\n",
            "Content-Type: application/pdf; name=\"contract.pdf\"\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "Content-Disposition: attachment; filename=\"contract.pdf\"\r\n",
            "\r\n",
            "JVBERi0xLjQK\r\n",
            "--BOUND\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "aGVsbG8=\r\n",
            "--BOUND-- epilogue ignored\r\n"
        );
        let attachments = build_email_event_attachments(raw);
        assert_eq!(attachments.len(), 2);

        assert_eq!(attachments[0].section, "2");
        assert_eq!(attachments[0].filename.as_deref(), Some("contract.pdf"));
        assert_eq!(
            attachments[0].content_type.as_deref(),
            Some("application/pdf")
        );
        assert_eq!(attachments[0].disposition.as_deref(), Some("attachment"));
        assert_eq!(attachments[0].size, Some(9)); // %PDF-1.4\n

        assert_eq!(attachments[1].section, "3");
        assert_eq!(attachments[1].filename, None);
        assert_eq!(
            attachments[1].content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(attachments[1].disposition, None);
        assert_eq!(attachments[1].size, Some(5)); // hello
    }

    #[test]
    fn treats_alternative_text_as_body_and_inline_cid_image_as_attachment() {
        let raw = concat!(
            "Content-Type: multipart/related; boundary=REL\r\n",
            "\r\n",
            "--REL\r\n",
            "Content-Type: multipart/alternative; boundary=ALT\r\n",
            "\r\n",
            "--ALT\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "plain body\r\n",
            "--ALT\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<b>html body</b><img src=\"cid:logo@cid\">\r\n",
            "--ALT--\r\n",
            "--REL\r\n",
            "Content-Type: image/png\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "Content-Disposition: inline\r\n",
            "Content-ID: <logo@cid>\r\n",
            "\r\n",
            "aVbW==\r\n",
            "--REL--\r\n"
        );
        let attachments = build_email_event_attachments(raw);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].section, "2");
        assert_eq!(attachments[0].disposition.as_deref(), Some("inline"));
        assert_eq!(attachments[0].content_id.as_deref(), Some("<logo@cid>"));
        assert_eq!(attachments[0].content_type.as_deref(), Some("image/png"));
    }

    #[test]
    fn keeps_duplicate_filenames_as_distinct_parts() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=MIX\r\n",
            "\r\n",
            "--MIX\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--MIX\r\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
            "Content-Type: application/pdf\r\n",
            "\r\n",
            "one\r\n",
            "--MIX\r\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
            "Content-Type: application/pdf\r\n",
            "\r\n",
            "two\r\n",
            "--MIX--\r\n"
        );
        let attachments = build_email_event_attachments(raw);
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].section, "2");
        assert_eq!(attachments[1].section, "3");
        assert_eq!(attachments[0].filename, attachments[1].filename);
        assert_eq!(attachments[0].id, "2");
        assert_eq!(attachments[1].id, "3");
    }

    #[test]
    fn numbers_nested_multipart_sections_like_rfc3501() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=OUTER\r\n",
            "\r\n",
            "--OUTER\r\n",
            "Content-Type: multipart/alternative; boundary=INNER\r\n",
            "\r\n",
            "--INNER\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "plain\r\n",
            "--INNER\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "html\r\n",
            "--INNER--\r\n",
            "--OUTER\r\n",
            "Content-Type: multipart/mixed; boundary=NEST\r\n",
            "\r\n",
            "--NEST\r\n",
            "Content-Type: application/zip\r\n",
            "\r\n",
            "zip-bytes\r\n",
            "--NEST\r\n",
            "Content-Type: application/pdf; name=\"doc.pdf\"\r\n",
            "\r\n",
            "pdf-bytes\r\n",
            "--NEST--\r\n",
            "--OUTER--\r\n"
        );
        let attachments = build_email_event_attachments(raw);
        let sections: Vec<&str> = attachments.iter().map(|a| a.section.as_str()).collect();
        assert_eq!(sections, vec!["2.1", "2.2"]);
        assert_eq!(attachments[0].filename, None);
        assert_eq!(attachments[1].filename.as_deref(), Some("doc.pdf"));
    }

    #[test]
    fn decodes_rfc2231_and_rfc2047_filenames() {
        // RFC 2231 single extended value.
        let raw_2231 = concat!(
            "Content-Type: multipart/mixed; boundary=B1\r\n",
            "\r\n",
            "--B1\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--B1\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename*=UTF-8''%E5%90%88%E5%90%8C.pdf\r\n",
            "\r\n",
            "data\r\n",
            "--B1--\r\n"
        );
        let attachments = build_email_event_attachments(raw_2231);
        assert_eq!(attachments[0].filename.as_deref(), Some("合同.pdf"));

        // RFC 2231 continuations.
        let raw_2231_cont = concat!(
            "Content-Type: multipart/mixed; boundary=B2\r\n",
            "\r\n",
            "--B2\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--B2\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment;\r\n",
            " filename*0*=UTF-8''%E5%90%88;\r\n",
            " filename*1*=%E5%90%8C.txt\r\n",
            "\r\n",
            "data\r\n",
            "--B2--\r\n"
        );
        let attachments = build_email_event_attachments(raw_2231_cont);
        assert_eq!(attachments[0].filename.as_deref(), Some("合同.txt"));

        // RFC 2047 B-encoded word inside a quoted filename.
        let raw_2047 = concat!(
            "Content-Type: multipart/mixed; boundary=B3\r\n",
            "\r\n",
            "--B3\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--B3\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename=\"=?UTF-8?B?5ZCI5ZCMLnBkZg==?=\"\r\n",
            "\r\n",
            "data\r\n",
            "--B3--\r\n"
        );
        let attachments = build_email_event_attachments(raw_2047);
        assert_eq!(attachments[0].filename.as_deref(), Some("合同.pdf"));
    }

    #[test]
    fn computes_decoded_sizes_for_base64_and_quoted_printable() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=S\r\n",
            "\r\n",
            "--S\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--S\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "\r\n",
            "caf=C3=A9=\r\n",
            "tail\r\n",
            "--S--\r\n"
        );
        let attachments = build_email_event_attachments(raw);
        // "café" (5 bytes) + "tail" (4 bytes), soft break removed.
        assert_eq!(attachments[0].size, Some(9));
    }

    #[test]
    fn returns_no_attachments_for_plain_text_messages() {
        let raw = "Subject: Hi\r\nFrom: a@example.com\r\nContent-Type: text/plain\r\n\r\nJust text";
        assert!(build_email_event_attachments(raw).is_empty());
    }

    #[test]
    fn bounds_deeply_nested_mime_structures() {
        let depth = MIME_MAX_DEPTH + 4;
        let mut raw = String::new();
        for level in 0..depth {
            raw.push_str(&format!(
                "Content-Type: multipart/mixed; boundary=B{level}\r\n\r\n--B{level}\r\n"
            ));
        }
        raw.push_str("Content-Type: application/pdf\r\n\r\ndata\r\n");
        for level in (0..depth).rev() {
            raw.push_str(&format!("--B{level}--\r\n"));
        }
        assert!(build_email_event_attachments(&raw).is_empty());
    }

    #[test]
    fn stamps_imap_retrieval_handles_on_attachments() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=HB\r\n",
            "\r\n",
            "--HB\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "body\r\n",
            "--HB\r\n",
            "Content-Disposition: attachment; filename=\"a.pdf\"\r\n",
            "Content-Type: application/pdf\r\n",
            "\r\n",
            "data\r\n",
            "--HB--\r\n"
        );
        let ctx = ImapAttachmentContext {
            endpoint: "imaps://imap.example.com:993",
            account: "primary",
            mailbox: "INBOX",
            message_id: "<m1@example.com>",
            uid: "42",
            uidvalidity: Some(3857529045),
            auth_profile: Some("imap-primary"),
        };
        let attachments = imap_message_attachments(raw.as_bytes(), &ctx);
        assert_eq!(attachments.len(), 1);
        let handle = &attachments[0]["handle"];
        assert_eq!(handle["type"], "email_attachment");
        assert_eq!(handle["provider"], "imap");
        assert_eq!(handle["endpoint"], "imaps://imap.example.com:993");
        assert_eq!(handle["account"], "primary");
        assert_eq!(handle["mailbox"], "INBOX");
        assert_eq!(handle["message_id"], "<m1@example.com>");
        assert_eq!(handle["uid"], "42");
        assert_eq!(handle["uidvalidity"], json!(3857529045u64));
        assert_eq!(handle["auth_profile"], "imap-primary");
        assert_eq!(handle["part"]["section"], "2");
        assert_eq!(attachments[0]["id"], "2");
        assert_eq!(attachments[0]["filename"], "a.pdf");
    }
}
