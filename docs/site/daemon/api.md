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

## TypeScript Client

The first-party Node package is `@holon-run/uxc-daemon-client`.

See also:

- [Generated TypeScript Clients](../ecosystem/typescript-client.md)
