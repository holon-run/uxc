# Protocols

UXC presents multiple protocol families through one top-level CLI contract.

## Supported Protocols

- OpenAPI / Swagger
- [MCP over HTTP and stdio](./mcp.md)
- GraphQL introspection and execution
- gRPC reflection-based discovery and unary invocation
- JSON-RPC with OpenRPC-style discovery

## Operation Naming

- OpenAPI: `method:/path`
- gRPC: `Service/Method`
- GraphQL: `query/name` or `mutation/name`
- MCP: tool name
- JSON-RPC: method name

## Runtime Extensions

Related runtime support also includes:

- daemon-backed subscription lifecycle
- WebSocket-based subscription flows
- polling-based subscriptions
- provider-aware event intake for systems such as Slack, Discord, and Feishu

## Notes

UXC does not try to erase protocol differences completely.
It tries to make discovery, inspection, auth attachment, invocation, and output
feel consistent across them.

<!-- INDEX:START -->

- [Embedded Rust MCP client](./embedded-mcp.md)
  Rust applications can depend on `uxc` and use `uxc::mcp` in process on a Tokio runtime. No UXC CLI executable or UXC daemon is needed. `uxc::mcp` is the supported embedding surface; existing adapter, CLI and daemon exports are not part of that compatibility promise.
  <!-- mdorigin:index kind=article -->

- [MCP](./mcp.md)
  For an in-process Rust integration, see the [embedded MCP client](./embedded-mcp.md).
  <!-- mdorigin:index kind=article -->

<!-- INDEX:END -->
