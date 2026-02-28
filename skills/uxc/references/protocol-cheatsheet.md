# Protocol Cheatsheet

UXC operation names follow protocol-native conventions.

## OpenAPI

- Operation id format: `method:/path`
- Example:
  - `get:/users/{id}`
  - `post:/pet`

## gRPC

- Operation id format: `Service/Method`
- Example:
  - `addsvc.Add/Sum`

## GraphQL

- Operation id format: `<operation_type>/<field>`
- Example:
  - `query/viewer`
  - `mutation/addStar`
  - `subscription/onEvent`

## MCP

- Operation id format: tool name
- Example:
  - `ask_question`
  - `list_directory`

## JSON-RPC (OpenRPC-driven)

- Operation id format: method name
- Example:
  - `eth_getBalance`
  - `net_version`

## Generic Command Templates

```bash
uxc <host> -h
uxc <host> <operation> -h
uxc <host> <operation> field=value
uxc <host> <operation> '{"field":"value"}'
```
