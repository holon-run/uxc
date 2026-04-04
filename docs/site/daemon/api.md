# Daemon API

UXC exposes a stable local daemon control plane over a Unix socket using
`Content-Length` framed JSON-RPC 2.0.

## Current Methods

- `daemon.status`
- `daemon.sessions`
- `runtime.invoke`
- `subscription.start`
- `subscription.list`
- `subscription.status`
- `subscription.stop`
- `subscription.events`

## Transport

Socket path follows the same daemon rules as the CLI:

- `$XDG_RUNTIME_DIR/uxc/uxc.sock`
- fallback: `$HOME/.uxc/daemon/uxc.sock`

Frame format:

```text
Content-Length: <bytes>\r\n
\r\n
<json body>
```

## Subscription Events

`subscription.events` reads event batches for a running or recently-stopped
subscription job.

Typical request shape:

```json
{
  "job_id": "sub_1",
  "after_seq": 0,
  "limit": 100,
  "wait_ms": 15000
}
```

## TypeScript Client

The first-party Node package is `@holon-run/uxc-daemon-client`.

See also:

- [Generated TypeScript Clients](../ecosystem/typescript-client.md)
