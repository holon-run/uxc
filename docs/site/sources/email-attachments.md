# Email Attachments

`email_event` messages expose provider-neutral attachment metadata plus an
opaque retrieval handle. Events never inline attachment bytes; use
`uxc email attachment get` to download content on demand.

## Attachment Metadata

Each entry under `message.attachments` carries:

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Provider part/attachment id (IMAP section number, Gmail/Graph attachment id, JMAP part id) |
| `filename` | string \| null | Decoded from RFC 2047 / RFC 2231 forms; `null` when absent |
| `content_type` | string \| null | MIME type of the part |
| `size` | number \| null | Decoded byte size when known |
| `disposition` | string \| null | `attachment`, `inline`, or `null` |
| `content_id` | string \| null | Content-ID for inline HTML resources |
| `handle` | object | Opaque retrieval handle for lazy download |

A part counts as an attachment when:

- it carries `Content-Disposition: attachment`, or
- it is `inline` with a filename or Content-ID, or
- it is a non-text part (for example unnamed `application/octet-stream`)
  outside the `multipart/alternative` body.

Plain `text/plain` and `text/html` alternatives of the message body are never
attachments.

## Provider Notes

| Provider | Mapping |
| --- | --- |
| IMAP | Parsed from the raw MIME tree; `id` is the RFC 3501 section number |
| Gmail | Recursed over `payload.parts`; `id` is the `body.attachmentId` |
| Microsoft Graph | Mapped from `attachments[]`; requires `$expand=attachments` on the poll endpoint |
| JMAP | Mapped from `attachments[]` (`partId`, `blobId`, `type`, `size`) |

Fields a provider does not report stay `null`; the original provider JSON is
preserved verbatim under `raw.provider_payload`.

### Microsoft Graph Without Expansion

Graph list endpoints do not return attachment metadata by default. When the
poll endpoint lacks `$expand=attachments` but the message reports
`hasAttachments: true`, the event keeps the provider's signal without
inventing metadata:

```json
{
  "attachments": [],
  "has_attachments": true,
  "attachment_count": null
}
```

`attachment_count: null` means "the provider says attachments exist but their
metadata was not expanded", as opposed to `0` for a message without
attachments. Consumers that need per-attachment entries should subscribe with
`$expand=attachments`.

## Retrieval Handles

`handle` objects are credential-free. They reference an auth profile by name,
and that profile is re-resolved from the local auth store at download time:

```json
{
  "type": "email_attachment",
  "provider": "gmail",
  "endpoint": "https://gmail.googleapis.com/gmail/v1/users/me/messages",
  "account": "user@example.com",
  "mailbox": "INBOX",
  "message_id": "<msg@example.com>",
  "uid": "18c2f4a9e3b1d000",
  "auth_profile": "gmail-primary",
  "part": { "attachment_id": "ANGjdJ_..." }
}
```

The `part` field is provider-specific: IMAP uses `{"section": "2.1"}` (plus
`uidvalidity` guarding against stale UIDs), Gmail and Graph use
`{"attachment_id": "..."}`, and JMAP uses `{"blob_id": "..."}`.

Treat handles as opaque values: copy them from the event and pass them back
to UXC. Do not parse or construct them by hand; the fields above are
documented for transparency only.

## Lazy Download

```bash
uxc email attachment get --handle '{"type":"email_attachment", ...}'
uxc email attachment get --handle @handle.json --output ./report.pdf
```

| Option | Description |
| --- | --- |
| `--handle <json\|@file>` | The `attachments[].handle` object from an `email_event`, or a file containing it |
| `--profile <name>` | Auth profile override; takes precedence over the handle reference |
| `--output <path>` | Explicit output path; defaults to a cache path under `~/.uxc/email-attachments/` |
| `--max-bytes <n>` | Reject larger attachments; defaults to 25 MiB, `0` disables the limit |

Successful downloads return a `kind=email_attachment_get_result` envelope:

```json
{
  "ok": true,
  "kind": "email_attachment_get_result",
  "data": {
    "provider": "gmail",
    "account": "user@example.com",
    "message_id": "<msg@example.com>",
    "attachment_id": "ANGjdJ_...",
    "filename": "report.pdf",
    "content_type": "application/pdf",
    "size_bytes": 102400,
    "sha256": "9f2b...",
    "saved_path": "/home/user/.uxc/email-attachments/report.pdf"
  }
}
```

Content is always written to a file; it is never inlined into the JSON
envelope. The response includes `sha256` and `size_bytes` so callers can
verify the download.

## Errors

| Code | Meaning |
| --- | --- |
| `invalid_attachment_handle` | The handle is not valid JSON, has the wrong `type`, or misses required provider fields |
| `attachment_not_found` | The provider no longer has the referenced attachment |
| `message_not_found` | The referenced message no longer exists |
| `uid_invalid` | IMAP mailbox `UIDVALIDITY` changed since the event; re-fetch the message |
| `auth_profile_missing` | The referenced auth profile is absent from the local store |
| `auth_failed` | The provider rejected the re-resolved credentials |
| `size_limit_exceeded` | The attachment exceeds `--max-bytes` |
| `provider_request_failed` | The provider request itself failed |

## Compatibility

Messages without attachments keep `attachments: []`,
`has_attachments: false`, and `attachment_count: 0`, so existing
`email_event` consumers are unaffected by attachment support.
