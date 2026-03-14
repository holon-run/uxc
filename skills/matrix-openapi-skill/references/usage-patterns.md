# Matrix Client-Server API Skill - Usage Patterns

## Link Setup

```bash
command -v matrix-openapi-cli
uxc link matrix-openapi-cli https://matrix.org/_matrix/client/v3 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/matrix-openapi-skill/references/matrix-client-server.openapi.json
matrix-openapi-cli -h
```

Replace `matrix.org` with your own homeserver when needed.

## Auth Setup

OAuth-aware homeserver:

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

Use a loopback callback on an uncommon high port to avoid collisions with local services on common ports.

Existing access token:

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

## Read Examples

```bash
# Confirm token owner
matrix-openapi-cli get:/account/whoami

# List joined rooms
matrix-openapi-cli get:/joined_rooms

# Read all current state events for a room
matrix-openapi-cli get:/rooms/{roomId}/state roomId=!abc123:example.org

# Read one named state event
matrix-openapi-cli get:/rooms/{roomId}/state/{eventType}/{stateKey} \
  roomId=!abc123:example.org \
  eventType=m.room.topic \
  stateKey=

# Poll /sync once
matrix-openapi-cli get:/sync timeout=30000

# Run room-scoped /sync as a background polling subscription
uxc subscribe start https://matrix.org/_matrix/client/v3 get:/sync \
  --auth matrix-oauth \
  --mode poll \
  --poll-config '{"interval_secs":2,"extract_items_pointer":"/rooms/join/!abc123:example.org/timeline/events","missing_extract_items_pointer_as_empty":true,"request_cursor_arg":"since","response_cursor_pointer":"/next_batch","checkpoint_strategy":{"type":"cursor_only"}}' \
  --sink file:$HOME/.uxc/subscriptions/matrix-sync.ndjson \
  timeout=1000 \
  'filter={"room":{"rooms":["!abc123:example.org"],"timeline":{"limit":5}}}'

# Inspect a user profile
matrix-openapi-cli get:/profile/{userId} userId=@alice:example.org
```

## Write Example (Confirm Intent First)

```bash
# Send a plain text room message with a unique transaction ID
matrix-openapi-cli put:/rooms/{roomId}/send/{eventType}/{txnId} \
  '{"roomId":"!abc123:example.org","eventType":"m.room.message","txnId":"uxc-001","msgtype":"m.text","body":"Hello from UXC"}'
```

## Fallback Equivalence

- `matrix-openapi-cli <operation> ...` is equivalent to
  `uxc https://matrix.org/_matrix/client/v3 --schema-url <matrix_openapi_schema> <operation> ...`.
