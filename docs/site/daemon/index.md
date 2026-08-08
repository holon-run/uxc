# Daemon

The UXC daemon exists to support reuse, lifecycle management, and background
runtime features that do not fit cleanly into one-off CLI calls.

## What It Handles

- session reuse for MCP transports
- background subscriptions
- local runtime invocation over a stable socket
- daemon status and session inspection

## Guides

- [Daemon API](./api.md)
- [Run as a Managed Service](./service.md)
- [Logging and Troubleshooting](./logging.md)

## Local App Integration

UXC also ships a TypeScript daemon client for local applications:

```bash
npm install @holon-run/uxc-daemon-client
```

## Notes

The daemon is especially useful when auth resolution, session reuse, or
background subscriptions should stay outside one-off CLI processes.

<!-- INDEX:START -->

- [Daemon API](./api.md)
  UXC exposes a stable local daemon control plane over a Unix socket using `Content-Length` framed JSON-RPC 2.0.
  <!-- mdorigin:index kind=article -->

- [Logging and Troubleshooting](./logging.md)
  UXC uses structured logging through `tracing`. Logs go to `stderr` so JSON output on `stdout` remains machine-parseable.
  <!-- mdorigin:index kind=article -->

- [Run as a Managed Service](./service.md)
  Run `uxc` daemon under a service manager when runtime auth environment must stay stable across requests.
  <!-- mdorigin:index kind=article -->

<!-- INDEX:END -->
