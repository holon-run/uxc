---
name: feishu-openapi-skill
description: Operate Feishu or Lark IM APIs through UXC with a curated OpenAPI schema, tenant-token bearer auth, and chat/message guardrails.
---

# Feishu / Lark IM Skill

Use this skill to run Feishu or Lark IM operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://open.feishu.cn/open-apis` or `https://open.larksuite.com/open-apis`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/feishu-openapi-skill/references/feishu-im.openapi.json`
- A Feishu or Lark app with bot capability enabled.
- A Feishu or Lark app `app_id` + `app_secret`, or a current `tenant_access_token` if you are using the manual fallback path.

## Scope

This skill covers an IM-focused request/response surface:

- chat lookup
- chat member lookup
- image and file upload for IM sends
- message send and reply
- selected message history reads
- basic user lookup through contact APIs

This skill does **not** cover:

- docs, bitable, approval, or non-IM product families
- the full Feishu or Lark Open Platform surface

## Subscribe Status

Feishu and Lark expose event-delivery models beyond plain request/response APIs, including long-connection event delivery in the platform ecosystem.

Current `uxc subscribe` status:

- request/response IM operations are validated
- inbound message intake is validated through the built-in `feishu-long-connection` transport
- live validation confirmed real `im.message.receive_v1` events delivered into the subscribe sink for a `p2p` bot chat

Important runtime notes:

- `feishu-long-connection` is a provider-aware transport inside `uxc subscribe`; it is not a plain raw WebSocket stream
- the runtime opens a temporary WebSocket URL from `/callback/ws/endpoint`
- frames are protobuf binary messages, not text JSON
- the runtime sends required event acknowledgements and ping control frames automatically

## Endpoint Choice

This schema works against either Feishu or Lark Open Platform base URLs:

- China / Feishu default: `https://open.feishu.cn/open-apis`
- International / Lark alternative: `https://open.larksuite.com/open-apis`

The fixed link example below uses Feishu. For Lark, use the same schema URL against the Lark base host.

## Authentication

Feishu and Lark service-side APIs use `Authorization: Bearer <tenant_access_token>` for these operations.

Preferred setup is to store `app_id` + `app_secret` as credential fields and let `uxc auth bootstrap` fetch and refresh the short-lived tenant token automatically.

Feishu bootstrap-managed setup:

```bash
uxc auth credential set feishu-tenant \
  --auth-type bearer \
  --field app_id=env:FEISHU_APP_ID \
  --field app_secret=env:FEISHU_APP_SECRET

uxc auth bootstrap set feishu-tenant \
  --token-endpoint https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal \
  --header 'Content-Type=application/json; charset=utf-8' \
  --request-json '{"app_id":"{{field:app_id}}","app_secret":"{{field:app_secret}}"}' \
  --access-token-pointer /tenant_access_token \
  --expires-in-pointer /expire \
  --success-code-pointer /code \
  --success-code-value 0

uxc auth binding add \
  --id feishu-tenant \
  --host open.feishu.cn \
  --path-prefix /open-apis \
  --scheme https \
  --credential feishu-tenant \
  --priority 100
```

For Lark, use the same bootstrap shape against the Lark host and bind the credential to `open.larksuite.com`.

To use long-connection subscribe, the credential still needs `app_id` and `app_secret` fields because the transport opens its own temporary event URL outside the normal bearer-token request path.

Manual fallback if you already have a tenant token:

```bash
curl -sS https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal \
  -H 'Content-Type: application/json; charset=utf-8' \
  -d '{"app_id":"cli_xxx","app_secret":"xxxx"}'
```

Lark uses the same path shape on the Lark host:

```bash
curl -sS https://open.larksuite.com/open-apis/auth/v3/tenant_access_token/internal \
  -H 'Content-Type: application/json; charset=utf-8' \
  -d '{"app_id":"cli_xxx","app_secret":"xxxx"}'
```

Configure one bearer credential and bind it to the Feishu API host:

```bash
uxc auth credential set feishu-tenant \
  --auth-type bearer \
  --secret-env FEISHU_TENANT_ACCESS_TOKEN

uxc auth binding add \
  --id feishu-tenant \
  --host open.feishu.cn \
  --path-prefix /open-apis \
  --scheme https \
  --credential feishu-tenant \
  --priority 100
```

For Lark, create the same binding against `open.larksuite.com`:

```bash
uxc auth binding add \
  --id lark-tenant \
  --host open.larksuite.com \
  --path-prefix /open-apis \
  --scheme https \
  --credential feishu-tenant \
  --priority 100
```

Inspect or pre-warm bootstrap state when auth looks wrong:

```bash
uxc auth bootstrap info feishu-tenant
uxc auth bootstrap refresh feishu-tenant
```

Validate the active binding when auth looks wrong:

```bash
uxc auth binding match https://open.feishu.cn/open-apis
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v feishu-openapi-cli`
   - If missing, create it:
     `uxc link feishu-openapi-cli https://open.feishu.cn/open-apis --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/feishu-openapi-skill/references/feishu-im.openapi.json`
   - `feishu-openapi-cli -h`

2. Inspect operation schema first:
   - `feishu-openapi-cli get:/im/v1/chats -h`
   - `feishu-openapi-cli post:/im/v1/images -h`
   - `feishu-openapi-cli post:/im/v1/files -h`
   - `feishu-openapi-cli post:/im/v1/messages -h`
   - `feishu-openapi-cli get:/im/v1/messages -h`

3. Prefer read/setup validation before writes:
   - `feishu-openapi-cli get:/im/v1/chats page_size=20`
   - `feishu-openapi-cli get:/im/v1/chats/{chat_id} chat_id=oc_xxx`
   - `feishu-openapi-cli get:/contact/v3/users/{user_id} user_id=ou_xxx user_id_type=open_id`

4. Execute with key/value or positional JSON:
   - key/value:
     `feishu-openapi-cli get:/im/v1/messages container_id_type=chat container_id=oc_xxx page_size=20`
   - multipart upload:
     `feishu-openapi-cli post:/im/v1/images image_type=message image=/tmp/example.png`
   - positional JSON:
     `feishu-openapi-cli post:/im/v1/messages receive_id_type=chat_id '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"Hello from UXC\"}"}'`

5. For inbound message intake, use `uxc subscribe` directly:
   - `uxc subscribe start https://open.feishu.cn/open-apis --transport feishu-long-connection --auth feishu-tenant --sink file:$HOME/.uxc/subscriptions/feishu.ndjson`
   - send a bot-visible message, then inspect the sink for `header.event_type = "im.message.receive_v1"`

## Operation Groups

### Chat Reads

- `get:/im/v1/chats`
- `get:/im/v1/chats/{chat_id}`
- `get:/im/v1/chats/{chat_id}/members`

### Message Reads / Writes

- `get:/im/v1/messages`
- `get:/im/v1/messages/{message_id}`
- `post:/im/v1/messages`
- `post:/im/v1/messages/{message_id}/reply`

### Uploads

- `post:/im/v1/images`
- `post:/im/v1/files`

### User Lookup

- `get:/contact/v3/users/{user_id}`
- `post:/contact/v3/users/batch_get_id`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Prefer `uxc auth bootstrap` over manual token management. Manual `tenant_access_token` setup is still supported as a fallback.
- `feishu-long-connection` requires the app credential fields `app_id` and `app_secret`; a plain bearer-only credential is not enough for event intake.
- `post:/im/v1/images` and `post:/im/v1/files` use `multipart/form-data`. File fields must be local path strings; help output marks them as multipart file fields.
- `post:/im/v1/messages` requires the `receive_id_type` query parameter and the body `content` field is a JSON-encoded string, not a nested JSON object.
- Upload first, then send by returned key:
  - image sends use `msg_type=image` with `content='{\"image_key\":\"img_xxx\"}'`
  - file sends use `msg_type=file` with `content='{\"file_key\":\"file_xxx\"}'`
- `post:/im/v1/messages/{message_id}/reply` is for explicit replies to an existing message. Treat it as a high-risk write.
- History reads only return chats and messages visible to the bot/app configuration. Auth success does not imply access to every chat.
- Long-connection message intake is validated for Feishu bot chats; webhook-style callbacks and non-IM products are still out of scope.
- `feishu-openapi-cli <operation> ...` is equivalent to `uxc https://open.feishu.cn/open-apis --schema-url <feishu_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/feishu-im.openapi.json`
- Feishu Open Platform docs: https://open.feishu.cn/document/
- Lark Open Platform docs: https://open.larksuite.com/document/
