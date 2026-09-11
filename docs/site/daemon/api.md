# Daemon API

UXC exposes a stable local daemon control plane over a Unix socket using
`Content-Length` framed JSON-RPC 2.0.

## Current Methods

- `daemon.status`
- `daemon.sessions`
- `runtime.invoke`
- `source.ensure`
- `source.status`
- `source.list`
- `source.stop`
- `source.delete`
- `stream.read`
- `stream.info`
- `stream.trim`
- `email.send`
- `email.reply`
- `email.body.read`
- `email.attachment.get`

## Transport

Socket path follows the same daemon rules as the CLI:

- `$HOME/.uxc/daemon/uxc.sock`
- fallback when `HOME` is unavailable: the OS temporary directory under a per-user `uxc-<user>/daemon/` directory

Frame format:

```text
Content-Length: <bytes>\r\n
\r\n
<json body>
```

## Managed Source Reads

`stream.read` reads event batches for a managed source stream.

Typical request shape:

```json
{
  "stream_id": "stream_abc123",
  "after_offset": 0,
  "limit": 100
}
```

## Email RPCs

Email operations share the same JSON-RPC surface. `email.send`, `email.reply`,
and `email.attachment.get` mirror the `uxc email` CLI commands.
`email.body.read` is daemon-only: local app integrations use it to extract
normalized message text.

- `email.send` sends a new message over SMTP. Params mirror the CLI flags:
  `smtp_url`, `from`, `to`/`cc`/`bcc`, `subject`, `text`/`html`, plus an
  optional `auth` profile and a caller-supplied `message_id` for outbound
  idempotency.
- `email.reply` flattens the same send params and accepts the `reply_handle`
  JSON carried by an `email_event` envelope.
- `email.body.read` takes one `input`:
  - `inline_mime`: `mime_base64` payload with `original_bytes` and `complete`
  - `legacy_mime`: inline `mime_text` with optional `source_truncated`
  - `message_ref`: opaque provider message reference

  Results return normalized plain text with a `completeness` value
  (`complete`, `partial`, or `unverified`) and parser provenance. Size limits
  and the read deadline are advertised in `daemon.status` under `email_body`.
- `email.attachment.get` downloads an attachment referenced by an
  `email_attachment` handle. Params: `handle` (raw handle JSON or `@file`),
  optional `profile` override, optional `output` path (defaults to the
  attachment cache), and `max_bytes` (`0` disables the limit). Results include
  `saved_path`, `size_bytes`, and `sha256`.

The TypeScript client exposes these as `emailSend`, `emailReply`,
`emailBodyRead`, and `emailAttachmentGet`; see
[Generated TypeScript Clients](../ecosystem/typescript-client.md).

## TypeScript Client

The first-party Node package is `@holon-run/uxc-daemon-client`.

See also:

- [Generated TypeScript Clients](../ecosystem/typescript-client.md)
