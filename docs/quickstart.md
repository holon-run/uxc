# Quickstart

This guide expands the README quickstart with practical command patterns across supported protocols.

## Prerequisites

- `uxc` installed and available in `PATH`
- Network access to target endpoints
- For gRPC unary runtime calls: `grpcurl` installed in `PATH`

Install options are listed in the top-level README:
[`README.md#install`](../README.md#install).

## 1. Discover Operations

Start from a host with help-first discovery:

```bash
uxc <host> -h
```

Examples:

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc countries.trevorblades.com -h
uxc mcp.deepwiki.com/mcp -h
```

## 2. Inspect Operation Schemas

Inspect a specific operation before calling it:

```bash
uxc <host> <operation_id> -h
```

Examples:

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
uxc countries.trevorblades.com query/country -h
uxc mcp.deepwiki.com/mcp ask_question -h
```

## 3. Execute Operations

### Preferred (simple): key/value arguments

```bash
uxc <host> <operation_id> key=value
```

### Preferred (structured): positional JSON object

```bash
uxc <host> <operation_id> '{"key":"value"}'
```

Do not pass raw JSON through `--args`.

## 4. Protocol Recipes

## OpenAPI

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

Schema-separated service (runtime endpoint differs from schema URL):

```bash
uxc api.github.com -h \
  --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json
```

See [`docs/schema-mapping.md`](schema-mapping.md) for mapping-based defaults.

## gRPC

```bash
uxc grpcb.in:9000 -h
uxc grpcb.in:9000 addsvc.Add/Sum a=1 b=2
```

## GraphQL

```bash
uxc countries.trevorblades.com -h
uxc countries.trevorblades.com query/country code=US
```

## MCP HTTP

```bash
uxc mcp.deepwiki.com/mcp -h
uxc mcp.deepwiki.com/mcp ask_question '{"repoName":"holon-run/uxc","question":"What does this project do?"}'
```

## MCP stdio

```bash
uxc "npx -y @modelcontextprotocol/server-filesystem /tmp" -h
uxc "npx -y @modelcontextprotocol/server-filesystem /tmp" list_directory path=/tmp
```

## JSON-RPC

```bash
uxc fullnode.mainnet.sui.io -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber
```

JSON-RPC discovery is OpenRPC-driven (`rpc.discover` or `openrpc.json` style sources).

## 5. Work with JSON-First Output

By default, UXC prints machine-friendly JSON envelopes.

```bash
uxc <host> -h
uxc <host> <operation_id> -h
uxc <host> <operation_id> key=value
uxc <host> <operation_id> '{...}'
```

Switch to CLI-readable text output when needed:

```bash
uxc --text -h
uxc --text <host> -h
```

## 6. Configure Auth

Auth model uses:

- Credentials: auth material
- Bindings: endpoint match rules that attach credentials

Example bearer setup:

```bash
uxc auth credential set deepwiki --auth-type bearer --secret-env DEEPWIKI_TOKEN
uxc auth binding add --id deepwiki-mcp --host mcp.deepwiki.com --path-prefix /mcp --scheme https --credential deepwiki --priority 100
```

For OAuth (MCP HTTP), see [`docs/oauth-mcp-http.md`](oauth-mcp-http.md).

## 7. Optional: Create Host Shortcut

Create a local shortcut command for a frequently used host:

```bash
uxc link petcli petstore3.swagger.io/api/v3
petcli -h
petcli get:/pet/{petId} -h
```

## 8. Next Docs

- Public endpoints without API keys: [`public-endpoints.md`](public-endpoints.md)
- Logging and troubleshooting: [`logging.md`](logging.md)
- Skills: [`skills.md`](skills.md)
- Release flow: [`release.md`](release.md)
