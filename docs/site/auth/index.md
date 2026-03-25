# Auth

UXC is designed for real provider integrations, not only public demo endpoints.

## Main Model

- credentials store auth material
- bindings match endpoints and select which credential applies

This keeps auth setup reusable instead of embedding secrets and rules into every
individual command.

## Supported Patterns

- bearer tokens
- API keys with configurable header or query placement
- multi-field credentials for signed APIs
- signer-backed request generation
- OAuth for supported MCP HTTP flows
- secret sources from literal values, environment variables, or external secret providers

## Guides

- [Secret Sources](./secret-sources.md)
- [MCP HTTP OAuth](./oauth-mcp-http.md)

<!-- INDEX:START -->

- [MCP HTTP OAuth](./oauth-mcp-http.md)
  This page summarizes OAuth support for MCP HTTP in UXC.

- [Secret Sources](./secret-sources.md)
  UXC supports two layers of non-OAuth auth values:

<!-- INDEX:END -->
