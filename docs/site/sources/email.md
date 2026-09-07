# Email

UXC exposes email as an event source plus direct outbound commands. Inbound
messages stream into daemon-backed subscriptions as normalized `email_event`
envelopes; replies and new mail go out through `uxc email send` and
`uxc email reply` over SMTP.

## Inbound Subscriptions

Two transports cover self-hosted IMAP mailboxes and hosted providers:

| Transport | Mode | Target | Example endpoint |
| --- | --- | --- | --- |
| `email-imap-idle` | `stream` | Any IMAP server with IDLE | `imaps://imap.example.com:993` |
| `email-provider-poll` | `poll` | Gmail, Microsoft Graph, or JMAP HTTP APIs | `https://gmail.googleapis.com/gmail/v1/users/me/messages` |

### IMAP IDLE

```bash
uxc source ensure agentinbox email:primary imaps://imap.example.com:993 \
  --transport email-imap-idle \
  --auth email-primary mailbox=INBOX
```

The subscription holds the connection open with IMAP IDLE and emits one
`email_event` per newly seen message. Credentials come from the referenced
auth profile (`--auth email-primary`); store the IMAP username and password in
that profile with your preferred auth mechanism.

### Provider Polling

```bash
uxc source ensure agentinbox gmail:primary \
  https://gmail.googleapis.com/gmail/v1/users/me/messages \
  --transport email-provider-poll --mode poll \
  --auth gmail-primary provider=gmail
```

`provider=gmail|graph|jmap` selects the normalization path. The endpoint must
return the provider's message-list JSON, so UXC can map it onto the same
`email_event` envelope:

| Provider | Endpoint example | Response shape |
| --- | --- | --- |
| Gmail | `https://gmail.googleapis.com/gmail/v1/users/me/messages` | `messages` array |
| Microsoft Graph | `https://graph.microsoft.com/v1.0/me/messages?$expand=attachments` | `value` array |
| JMAP | JMAP API endpoint returning `Email/get` results | `list` array |

Microsoft Graph only returns attachment metadata when the endpoint includes
`$expand=attachments`; see [Email Attachments](./email-attachments.md) for the
partial-metadata semantics.

## Event Envelope

Every transport emits the same envelope shape:

```json
{
  "type": "email_event",
  "version": "v1",
  "provider": "imap",
  "account": "user@example.com",
  "mailbox": "INBOX",
  "event_kind": "message_received",
  "message": {
    "uid": "42",
    "message_id": "<msg@example.com>",
    "thread_id": null,
    "from": "sender@example.com",
    "to": ["user@example.com"],
    "subject": "Quarterly report",
    "date": "Mon, 07 Sep 2026 12:00:00 +0000",
    "snippet": "Please review the attached report...",
    "attachments": [],
    "has_attachments": false,
    "attachment_count": 0,
    "flags": []
  },
  "raw": {
    "mime_inline": null,
    "mime_truncated": true,
    "size_bytes": 204800
  },
  "reply_handle": {
    "type": "email_imap",
    "provider": "imap",
    "account": "user@example.com",
    "mailbox": "INBOX",
    "message_id": "<msg@example.com>",
    "uid": "42"
  }
}
```

Provider-polled events use `provider: "gmail" | "graph" | "jmap"`, keep the
provider's original JSON under `raw.provider_payload`, and stamp a
`reply_handle` of type `email_provider`.

| Field | Notes |
| --- | --- |
| `message.attachments` | Neutral attachment metadata plus retrieval handles; see [Email Attachments](./email-attachments.md) |
| `message.has_attachments` | `true` when the message has (or the provider reports) attachments |
| `raw.mime_inline` | Raw MIME text inlined when the message is small enough; `null` otherwise |
| `raw.provider_payload` | Original provider JSON for polled sources |
| `reply_handle` | Opaque handle consumed by `uxc email reply --reply-handle` |

## Outbound Mail

Send a new message:

```bash
uxc email send --smtp smtp://localhost:2525 \
  --from bot@example.com --to user@example.com \
  --subject 'Hello' --text-body 'Hi'
```

Reply to a received event using its `reply_handle`:

```bash
uxc email reply --smtp smtp://localhost:2525 \
  --reply-handle '{"message_id":"<msg@example.com>"}' \
  --from bot@example.com --to user@example.com \
  --subject 'Re: Quarterly report' --text-body 'Thanks'
```

Notes:

- Use `--auth` with credentials containing `username`/`user`/`account` and
  `password`/`secret` fields when SMTP AUTH is required.
- SMTP AUTH over cleartext `smtp://` requires explicit
  `--allow-insecure-auth`; prefer a trusted local relay until
  STARTTLS/TLS is supported.
- Add `--dry-run` to preview a message without delivering it.
