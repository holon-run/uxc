---
name: dingtalk-openapi-skill
description: Operate DingTalk messaging APIs through UXC with a curated OpenAPI schema, app-token bearer auth, and robot/service-group guardrails.
---

# DingTalk Messaging Skill

Use this skill to run DingTalk messaging operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.dingtalk.com/v1.0`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/dingtalk-openapi-skill/references/dingtalk-messaging.openapi.json`
- A DingTalk internal app or app suite with bot messaging permissions enabled.
- A DingTalk app `appKey` + `appSecret`, or a current `accessToken` if you are using the manual fallback path.
- Robot code, conversation identifiers, and user identifiers for the target send flows.

## Scope

This skill covers a narrow IM-focused request/response surface:

- one-to-one bot sends
- group bot sends
- service group sends
- minimal user lookup by `unionId`

This skill does **not** cover:

- approval, attendance, admin, or broader enterprise app workflows
- old `oapi.dingtalk.com` endpoints
- custom robot webhook token/signature flows
- inbound callback or webhook receiver runtime
## Subscribe / Stream Mode Status

DingTalk has event-delivery flows such as Stream Mode, but those flows are outside the current scope of this skill.

Current `uxc subscribe` status:

- this skill is validated only for request/response messaging operations
- DingTalk inbound event/message intake is **not** currently validated through `uxc subscribe`

Treat DingTalk as a possible future subscribe target via Stream Mode rather than a currently supported IM subscribe provider.

## API Surface Choice

This skill is intentionally pinned to the newer DingTalk Open Platform host:

- `https://api.dingtalk.com/v1.0`

The older `oapi.dingtalk.com` surface is intentionally excluded from v1 to keep auth and schema shape consistent.

## Authentication

DingTalk v1 APIs use app `accessToken` credentials.

Preferred setup is to store `appKey` + `appSecret` as credential fields and let `uxc auth bootstrap` fetch and refresh the short-lived access token automatically.

Bootstrap-managed setup:

```bash
uxc auth credential set dingtalk-app \
  --auth-type bearer \
  --field app_key=env:DINGTALK_APP_KEY \
  --field app_secret=env:DINGTALK_APP_SECRET

uxc auth bootstrap set dingtalk-app \
  --token-endpoint https://api.dingtalk.com/v1.0/oauth2/accessToken \
  --header 'Content-Type=application/json' \
  --request-json '{"appKey":"{{field:app_key}}","appSecret":"{{field:app_secret}}"}' \
  --access-token-pointer /accessToken \
  --expires-in-pointer /expireIn

uxc auth binding add \
  --id dingtalk-app \
  --host api.dingtalk.com \
  --path-prefix /v1.0 \
  --scheme https \
  --credential dingtalk-app \
  --priority 100
```

Manual fallback if you already have an app access token:

```bash
curl -sS https://api.dingtalk.com/v1.0/oauth2/accessToken \
  -H 'Content-Type: application/json' \
  -d '{"appKey":"dingxxxx","appSecret":"xxxx"}'
```

Configure one bearer credential and bind it to the DingTalk API host:

```bash
uxc auth credential set dingtalk-app \
  --auth-type bearer \
  --secret-env DINGTALK_ACCESS_TOKEN

uxc auth binding add \
  --id dingtalk-app \
  --host api.dingtalk.com \
  --path-prefix /v1.0 \
  --scheme https \
  --credential dingtalk-app \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://api.dingtalk.com/v1.0
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v dingtalk-openapi-cli`
   - If missing, create it:
     `uxc link dingtalk-openapi-cli https://api.dingtalk.com/v1.0 --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/dingtalk-openapi-skill/references/dingtalk-messaging.openapi.json`
   - `dingtalk-openapi-cli -h`

2. Inspect operation schema first:
   - `dingtalk-openapi-cli get:/contact/users/{unionId} -h`
   - `dingtalk-openapi-cli post:/robot/oToMessages/batchSend -h`
   - `dingtalk-openapi-cli post:/robot/groupMessages/send -h`

3. Prefer read/setup validation before writes:
   - `dingtalk-openapi-cli get:/contact/users/{unionId} unionId=$DINGTALK_UNION_ID`
   - `dingtalk-openapi-cli post:/robot/oToMessages/batchSend -h`
   - `dingtalk-openapi-cli post:/serviceGroup/messages/send -h`

4. Execute with key/value or positional JSON:
   - key/value:
     `dingtalk-openapi-cli get:/contact/users/{unionId} unionId=$DINGTALK_UNION_ID language=zh_CN`
   - positional JSON:
     `dingtalk-openapi-cli post:/robot/groupMessages/send '{"openConversationId":"cidxxxx","robotCode":"dingxxxx","msgKey":"sampleText","msgParam":"{\"content\":\"Hello from UXC\"}"}'`

## Operation Groups

### User Lookup

- `get:/contact/users/{unionId}`

### Messaging

- `post:/robot/oToMessages/batchSend`
- `post:/robot/groupMessages/send`
- `post:/serviceGroup/messages/send`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Prefer `uxc auth bootstrap` over manual token management. Manual `accessToken` setup is still supported as a fallback.
- All three send operations are high-risk writes. Require explicit user confirmation before execution.
- DingTalk `msgParam` is a JSON-encoded string payload, not a nested JSON object. Build and validate that string carefully before sending.
- `robotCode`, `openConversationId`, `coolAppCode`, and target identifiers are all provider-specific routing fields. Missing any of them generally means the send will fail even if auth is valid.
- This v1 wrapper does not include custom robot webhook token/sign flows; use app-based APIs only.
- `dingtalk-openapi-cli <operation> ...` is equivalent to `uxc https://api.dingtalk.com/v1.0 --schema-url <dingtalk_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/dingtalk-messaging.openapi.json`
- DingTalk developer docs: https://open.dingtalk.com/document/
