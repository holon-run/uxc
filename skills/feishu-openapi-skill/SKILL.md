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
- A current `tenant_access_token` for the target tenant.

## Scope

This skill covers an IM-focused request/response surface:

- chat lookup
- chat member lookup
- message send and reply
- selected message history reads
- basic user lookup through contact APIs

This skill does **not** cover:

- token bootstrap or automatic token refresh inside `uxc`
- inbound event subscription receiver runtime
- docs, bitable, approval, or non-IM product families
- the full Feishu or Lark Open Platform surface

## Subscribe Status

Feishu and Lark expose event-delivery models beyond plain request/response APIs, including long-connection event delivery in the platform ecosystem.

Current `uxc subscribe` status:

- this skill is validated only for request/response IM operations
- inbound event/message intake is **not** currently validated through `uxc subscribe`

Treat Feishu / Lark as future subscribe targets that will need provider-aware session behavior beyond this v1 OpenAPI wrapper.

## Endpoint Choice

This schema works against either Feishu or Lark Open Platform base URLs:

- China / Feishu default: `https://open.feishu.cn/open-apis`
- International / Lark alternative: `https://open.larksuite.com/open-apis`

The fixed link example below uses Feishu. For Lark, use the same schema URL against the Lark base host.

## Authentication

Feishu and Lark service-side APIs use `Authorization: Bearer <tenant_access_token>` for these operations.

Tenant access tokens are typically fetched from the internal app token endpoint using `app_id` and `app_secret`, and the official auth docs state they are valid for 2 hours. Keep that bootstrap outside this skill, then bind the resulting token into `uxc auth`.

Feishu bootstrap example:

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

Validate the active mapping when auth looks wrong:

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
   - `feishu-openapi-cli post:/im/v1/messages -h`
   - `feishu-openapi-cli get:/im/v1/messages -h`

3. Prefer read/setup validation before writes:
   - `feishu-openapi-cli get:/im/v1/chats page_size=20`
   - `feishu-openapi-cli get:/im/v1/chats/{chat_id} chat_id=oc_xxx`
   - `feishu-openapi-cli get:/contact/v3/users/{user_id} user_id=ou_xxx user_id_type=open_id`

4. Execute with key/value or positional JSON:
   - key/value:
     `feishu-openapi-cli get:/im/v1/messages container_id_type=chat container_id=oc_xxx page_size=20`
   - positional JSON:
     `feishu-openapi-cli post:/im/v1/messages receive_id_type=chat_id '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"Hello from UXC\"}"}'`

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

### User Lookup

- `get:/contact/v3/users/{user_id}`
- `post:/contact/v3/users/batch_get_id`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `tenant_access_token` bootstrap and refresh are outside this skill. If calls start failing after token expiry, fetch a fresh token and update the bound secret.
- `post:/im/v1/messages` requires the `receive_id_type` query parameter and the body `content` field is a JSON-encoded string, not a nested JSON object.
- `post:/im/v1/messages/{message_id}/reply` is for explicit replies to an existing message. Treat it as a high-risk write.
- History reads only return chats and messages visible to the bot/app configuration. Auth success does not imply access to every chat.
- Event subscription and callback verification are intentionally out of scope for this v1 skill.
- `feishu-openapi-cli <operation> ...` is equivalent to `uxc https://open.feishu.cn/open-apis --schema-url <feishu_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/feishu-im.openapi.json`
- Feishu Open Platform docs: https://open.feishu.cn/document/
- Lark Open Platform docs: https://open.larksuite.com/document/
