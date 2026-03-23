# Daemon API

`uxc` exposes a stable local daemon control plane over a Unix socket using `Content-Length` framed JSON-RPC 2.0.

Current supported methods:

- `daemon.status`
- `daemon.sessions`
- `runtime.invoke`
- `subscription.start`
- `subscription.list`
- `subscription.status`
- `subscription.stop`
- `subscription.events`

## Transport

- Socket path follows the same daemon rules as the CLI:
  - `$XDG_RUNTIME_DIR/uxc/uxc.sock`
  - fallback: `$HOME/.uxc/daemon/uxc.sock`
- Each request uses one framed JSON-RPC message and one framed JSON-RPC response.
- This surface is intended for local Unix / macOS / Linux / WSL usage.

Frame format:

```text
Content-Length: <bytes>\r\n
\r\n
<json body>
```

## Subscription Events

`subscription.events` reads event batches for a running or recently-stopped subscription job.

Request:

```json
{
  "job_id": "sub_1",
  "after_seq": 0,
  "limit": 100,
  "wait_ms": 15000
}
```

Response:

```json
{
  "job_id": "sub_1",
  "status": "running",
  "events": [],
  "next_after_seq": 0,
  "has_more": false
}
```

Behavior:

- `after_seq` is exclusive.
- `limit` defaults to `100` and is capped at `500`.
- `wait_ms` defaults to `0` and is capped at `15000`.
- Events use the same envelope shape as daemon NDJSON subscription sinks.
- `memory:` sink is supported for SDK-driven subscriptions that do not want to manage files directly.
- Stopped jobs remain readable through `subscription.status` and `subscription.events` for a short terminal retention window.

## TypeScript Package

The first-party Node package is `@holon-run/uxc-daemon-client`.

Typical usage:

```ts
import { UxcDaemonClient } from "@holon-run/uxc-daemon-client";

const client = new UxcDaemonClient();

const result = await client.call({
  endpoint: "https://petstore3.swagger.io/api/v3",
  operation: "get:/store/inventory",
});

const sub = await client.subscribeStart({
  endpoint: "ws://127.0.0.1:9000",
  sink: "memory:",
  transportHint: "websocket",
});

for await (const event of client.subscribeEvents(sub.job_id)) {
  console.log(event);
}
```
