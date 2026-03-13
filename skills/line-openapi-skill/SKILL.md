---
name: line-openapi-skill
description: Operate LINE Messaging API through UXC with a curated OpenAPI schema, bearer-token auth, and messaging-core guardrails.
---

# LINE Messaging API Skill

Use this skill to run LINE Messaging API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.line.me`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/line-openapi-skill/references/line-messaging.openapi.json`
- A LINE Messaging API channel access token.

## Scope

This skill covers a Messaging Core surface:

- bot identity lookup
- user profile lookup
- push and reply message sends
- quota and quota consumption reads
- webhook endpoint get/set/test operations

This skill does **not** cover:

- inbound webhook receiver runtime
- media/content download flows on `api-data.line.me`
- audience, narrowcast, rich menu, or account-management surfaces
- the full LINE Messaging API

## Authentication

LINE Messaging API uses `Authorization: Bearer <channel access token>`.

Configure one bearer credential and bind it to `api.line.me`:

```bash
uxc auth credential set line-channel \
  --auth-type bearer \
  --secret-env LINE_CHANNEL_ACCESS_TOKEN

uxc auth binding add \
  --id line-channel \
  --host api.line.me \
  --scheme https \
  --credential line-channel \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://api.line.me
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v line-openapi-cli`
   - If missing, create it:
     `uxc link line-openapi-cli https://api.line.me --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/line-openapi-skill/references/line-messaging.openapi.json`
   - `line-openapi-cli -h`

2. Inspect operation schema first:
   - `line-openapi-cli get:/v2/bot/info -h`
   - `line-openapi-cli get:/v2/bot/profile/{userId} -h`
   - `line-openapi-cli post:/v2/bot/message/push -h`

3. Prefer read/setup validation before writes:
   - `line-openapi-cli get:/v2/bot/info`
   - `line-openapi-cli get:/v2/bot/message/quota`
   - `line-openapi-cli get:/v2/bot/channel/webhook/endpoint`

4. Execute with key/value or positional JSON:
   - key/value:
     `line-openapi-cli get:/v2/bot/profile/{userId} userId=U1234567890abcdef`
   - positional JSON:
     `line-openapi-cli post:/v2/bot/message/push '{"to":"U1234567890abcdef","messages":[{"type":"text","text":"Hello from UXC"}]}'`

## Operation Groups

### Read / Lookup

- `get:/v2/bot/info`
- `get:/v2/bot/profile/{userId}`
- `get:/v2/bot/message/quota`
- `get:/v2/bot/message/quota/consumption`
- `get:/v2/bot/channel/webhook/endpoint`

### Messaging

- `post:/v2/bot/message/push`
- `post:/v2/bot/message/reply`

### Webhook Endpoint Management

- `put:/v2/bot/channel/webhook/endpoint`
- `post:/v2/bot/channel/webhook/test`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use a channel access token with the scopes required by the target bot/channel configuration.
- `post:/v2/bot/message/push` and `post:/v2/bot/message/reply` are write/high-risk operations; require explicit user confirmation before execution.
- `replyToken` values are short-lived and webhook-derived. Use `post:/v2/bot/message/reply` only when the caller already has a valid token from a recent event.
- Webhook endpoint get/set/test calls configure delivery only; they do not provide a receiver runtime in `uxc`.
- This v1 skill stays on `https://api.line.me`; content retrieval endpoints on `https://api-data.line.me` are intentionally out of scope.
- `line-openapi-cli <operation> ...` is equivalent to `uxc https://api.line.me --schema-url <line_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/line-messaging.openapi.json`
- LINE Messaging API reference: https://developers.line.biz/en/reference/messaging-api/
