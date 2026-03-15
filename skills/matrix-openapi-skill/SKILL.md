---
name: matrix-openapi-skill
description: Operate Matrix Client-Server API through UXC with a curated OpenAPI schema, bearer-token auth, and homeserver-aware messaging guardrails.
---

# Matrix Client-Server API Skill

Use this skill to run Matrix Client-Server operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to your Matrix homeserver's client-server base URL, usually `https://<homeserver>/_matrix/client/v3`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/matrix-openapi-skill/references/matrix-client-server.openapi.json`
- A Matrix access token for the target homeserver.

## Scope

This skill covers a practical request/response Matrix surface:

- token owner lookup
- joined room discovery
- room state lookup
- `/sync` polling reads, including daemon-backed poll subscribe
- user profile and presence lookup
- room message sends

This skill does **not** cover:

- login, SSO, device registration, or generic token acquisition flows
- appservice, federation, or bot framework abstractions
- webhook or long-running event receiver runtime
- full Matrix spec coverage

## Homeserver Base URL

Matrix is homeserver-specific. The endpoint you link must include the Matrix client-server base path:

- typical form: `https://<homeserver>/_matrix/client/v3`
- example form: `https://matrix.org/_matrix/client/v3`

Do not link only the homeserver origin without `/_matrix/client/v3`.

## Authentication

Matrix Client-Server API uses `Authorization: Bearer <access_token>`.

Preferred path for OAuth-aware homeservers:

```bash
uxc auth oauth start matrix-oauth \
  --endpoint https://matrix.org/_matrix/client/v3 \
  --redirect-uri http://127.0.0.1:8788/callback \
  --client-id <client_id>

uxc auth oauth complete matrix-oauth \
  --session-id <session_id> \
  --authorization-response 'http://127.0.0.1:8788/callback?code=...'

uxc auth binding add \
  --id matrix-oauth \
  --host matrix.org \
  --path-prefix /_matrix/client/v3 \
  --scheme https \
  --credential matrix-oauth \
  --priority 100
```

Fallback path when you already have an access token:

```bash
uxc auth credential set matrix-access \
  --auth-type bearer \
  --secret-env MATRIX_ACCESS_TOKEN

uxc auth binding add \
  --id matrix-access \
  --host matrix.org \
  --path-prefix /_matrix/client/v3 \
  --scheme https \
  --credential matrix-access \
  --priority 100
```

If your homeserver is not `matrix.org`, replace the host while keeping the same path prefix. Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://matrix.org/_matrix/client/v3
```

Notes:
- `uxc auth oauth` works only for homeservers that expose Matrix OAuth metadata.
- Prefer a loopback redirect URI on an uncommon high port, such as `http://127.0.0.1:8788/callback`, to avoid conflicts with local services on common ports.
- Legacy Matrix login and SSO fallback flows are not covered by this skill yet.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v matrix-openapi-cli`
   - If missing, create it:
     `uxc link matrix-openapi-cli https://matrix.org/_matrix/client/v3 --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/matrix-openapi-skill/references/matrix-client-server.openapi.json`
   - `matrix-openapi-cli -h`

2. Inspect operation schema first:
   - `matrix-openapi-cli get:/account/whoami -h`
   - `matrix-openapi-cli get:/sync -h`
   - `matrix-openapi-cli put:/rooms/{roomId}/send/{eventType}/{txnId} -h`

3. Prefer read validation before writes:
   - `matrix-openapi-cli get:/account/whoami`
   - `matrix-openapi-cli get:/joined_rooms`
   - `matrix-openapi-cli get:/rooms/{roomId}/state roomId=!abc123:example.org`

4. Execute with key/value or positional JSON:
   - key/value:
     `matrix-openapi-cli get:/sync timeout=30000 filter={"room":{"timeline":{"limit":10}}}`
   - positional JSON:
     `matrix-openapi-cli put:/rooms/{roomId}/send/{eventType}/{txnId} '{"roomId":"!abc123:example.org","eventType":"m.room.message","txnId":"uxc-001","msgtype":"m.text","body":"Hello from UXC"}'`

5. For background `/sync` polling, call `uxc subscribe start` directly against the homeserver base URL:
   - `uxc subscribe start https://matrix.org/_matrix/client/v3 get:/sync --auth matrix-oauth --mode poll --poll-config '{"interval_secs":2,"extract_items_pointer":"/rooms/join/!abc123:example.org/timeline/events","missing_extract_items_pointer_as_empty":true,"request_cursor_arg":"since","response_cursor_pointer":"/next_batch","checkpoint_strategy":{"type":"cursor_only"}}' --sink file:$HOME/.uxc/subscriptions/matrix-sync.ndjson timeout=1000 'filter={"room":{"rooms":["!abc123:example.org"],"timeline":{"limit":5}}}'`

## Operation Groups

### Session / Discovery

- `get:/account/whoami`
- `get:/joined_rooms`
- `get:/sync`

### Room Reads

- `get:/rooms/{roomId}/state`
- `get:/rooms/{roomId}/state/{eventType}/{stateKey}`

### User Reads

- `get:/profile/{userId}`
- `get:/presence/{userId}/status`

### Messaging

- `put:/rooms/{roomId}/send/{eventType}/{txnId}`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `get:/sync` works both as a normal polling/read call and as a validated daemon-backed poll subscription when invoked through `uxc subscribe start`.
- For room-scoped `/sync` subscriptions, set `missing_extract_items_pointer_as_empty=true` so sparse responses without new room timeline events are treated as an empty batch instead of a fatal error.
- `put:/rooms/{roomId}/send/{eventType}/{txnId}` is high-risk and should default to `m.room.message` text sends unless the user explicitly asks for another event type.
- Reuse a unique `txnId` per send attempt to avoid accidental duplicates.
- Many homeservers restrict presence visibility and room state/event access based on membership and server policy; auth success does not imply every room or profile read will succeed.
- `matrix-openapi-cli <operation> ...` is equivalent to `uxc <homeserver_client_base> --schema-url <matrix_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/matrix-client-server.openapi.json`
- Matrix Client-Server API: https://spec.matrix.org/latest/client-server-api/
- Matrix spec source: https://github.com/matrix-org/matrix-spec/tree/main/data/api/client-server
