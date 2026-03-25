# Protocols

UXC presents multiple protocol families through one top-level CLI contract.

## Supported Protocols

- OpenAPI / Swagger
- MCP over HTTP and stdio
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

<!-- INDEX:END -->
