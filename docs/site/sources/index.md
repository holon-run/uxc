# Sources

Inbound event sources turn external systems into daemon-backed subscription
streams. UXC normalizes each provider into a typed event envelope so agents
and scripts consume one stable shape per system.

## Available Sources

- [Email](./email.md) — IMAP IDLE and Gmail, Microsoft Graph, and JMAP provider polling, plus SMTP send and reply
- [Email Attachments](./email-attachments.md) — provider-neutral attachment metadata and lazy downloads

<!-- INDEX:START -->

- [Email](./email.md)
  UXC exposes email as an event source plus direct outbound commands. Inbound messages stream into daemon-backed subscriptions as normalized `email_event` envelopes; replies and new mail go out through `uxc email send` and `uxc email reply` over SMTP.
  <!-- mdorigin:index kind=article -->

- [Email Attachments](./email-attachments.md)
  `email_event` messages expose provider-neutral attachment metadata plus an opaque retrieval handle. Events never inline attachment bytes; use `uxc email attachment get` to download content on demand.
  <!-- mdorigin:index kind=article -->

<!-- INDEX:END -->
