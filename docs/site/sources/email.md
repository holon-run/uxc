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

These subscriptions resolve credentials inside the daemon process, so
env-sourced secrets (`--secret-env`) cannot see variables exported in the
shell that runs `uxc source ensure`. `uxc source ensure` warns on stderr in
that case; see [Secret Sources](../auth/secret-sources.md) for recommended
alternatives such as literal secrets or 1Password references.

> **Microsoft personal accounts require OAuth (XOAUTH2).**
> Personal Microsoft accounts (`outlook.com`, `hotmail.com`, `live.com`)
> reject IMAP `LOGIN` with `NO Basic authentication is disabled` before
> validating credentials, so no password fix can make basic auth work for
> them. UXC detects this server-side policy rejection, reports the source as
> `failed` with an actionable error, and stops reconnecting. Use the
> [XOAUTH2 flow below](#microsoft-personal-accounts-xoauth2) to keep IMAP
> IDLE push delivery, or fall back to [Provider Polling](#provider-polling)
> with `provider=graph` for minute-level polling instead.

### Microsoft Personal Accounts (XOAUTH2)

IMAP for personal Microsoft accounts authenticates with OAuth via the
XOAUTH2 SASL mechanism. UXC ships a provider preset that borrows the public
Thunderbird client id, so a personal account needs two commands and one
browser consent — no Azure app registration:

```bash
# 1. Device-code login; opens a consent page in the browser
uxc auth oauth login outlook-imap --provider outlook

# 2. Subscribe over IMAP IDLE with the OAuth credential
uxc source ensure agentinbox email:primary imaps://outlook.office365.com:993 \
  --transport email-imap-idle \
  --auth outlook-imap mailbox=INBOX
```

The `--provider outlook` preset fills in:

| Preset default | Value |
| --- | --- |
| Client id | Mozilla Thunderbird public client `9e5f94bc-e8a4-4e73-b8be-63364c29d753` |
| Issuer / tenant | `https://login.microsoftonline.com/consumers` |
| Scopes | `https://outlook.office.com/IMAP.AccessAsUser.All`, `https://outlook.office.com/SMTP.Send`, `offline_access` |

Notes:

- Pass `--client-id <id>` to use your own Azure app registration instead of
  the Thunderbird client id; organization (work/school) accounts typically
  need their own registration and admin consent.
- The OAuth profile's mailbox address is resolved from the profile's
  `username`/`user`/`email` field, the `account=` argument, or finally the
  credential id. Set a `username` field (for example
  `uxc auth credential set outlook-imap --field username=literal:user@hotmail.com`)
  if the credential id is not the mailbox address.
- UXC refreshes the access token before every (re)connect and persists the
  refreshed token, so long-lived IDLE sessions survive token expiry without
  manual `auth oauth refresh`.
- If the server rejects the token, the source fails with an actionable error
  (re-consent via `uxc auth oauth login ... --provider outlook`) instead of
  retrying forever.
- Attachment retrieval (`uxc email attachment get`) uses the same XOAUTH2
  path with the same credential.

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

### Microsoft Graph with Personal Accounts

The Graph mail API (`/v1.0/me/messages`, delegated `Mail.Read`) also serves
personal Microsoft accounts, so `provider=graph` polling works for them as a
no-IDLE alternative to IMAP XOAUTH2:

- Use an OAuth credential with Graph scopes (`Mail.Read offline_access`) —
  note the Thunderbird client id registered IMAP/SMTP resource permissions
  and cannot consent Graph scopes, so this path needs your own Azure app
  registration (`--client-id`) or another client authorized for Graph.
- Expect minute-level latency (poll interval) instead of IMAP IDLE's
  second-level push, and keep intervals >= 60s to stay clear of Graph
  throttling.

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
- `smtp://` upgrades to TLS via STARTTLS automatically when the server
  advertises it; `smtps://` (implicit TLS, port 465) is also supported.
- Password AUTH (`AUTH PLAIN`) over a `smtp://` connection that cannot be
  upgraded requires explicit `--allow-insecure-auth`; prefer a trusted local
  relay or STARTTLS-capable submission endpoint.
- OAuth credentials authenticate with `AUTH XOAUTH2`. This is how Microsoft
  personal accounts send mail:

```bash
uxc auth oauth login outlook-imap --provider outlook   # same credential as IMAP

uxc email send --smtp smtp://smtp-mail.outlook.com:587 \
  --from user@hotmail.com --to someone@example.com \
  --subject 'Hello' --text-body 'Hi' \
  --auth outlook-imap
```

  OAuth tokens are never sent over cleartext: `smtp://` must offer STARTTLS
  (Microsoft's submission endpoint does) or use `smtps://`.
- Add `--dry-run` to preview a message without delivering it.
