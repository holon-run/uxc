# Public Test Endpoints (No API Key)

These endpoints are useful for protocol availability checks without API keys.

Last verified in project docs on **2026-02-28**.

## OpenAPI

- Endpoint: `https://petstore3.swagger.io/api/v3`
- Quick checks:

```bash
uxc petstore3.swagger.io/api/v3 -h
curl -sS https://petstore3.swagger.io/api/v3/openapi.json | jq -r '.openapi, .info.title'
```

## GraphQL

- Endpoint: `https://countries.trevorblades.com/`
- Quick checks:

```bash
uxc countries.trevorblades.com -h
curl -sS https://countries.trevorblades.com/ \
  -H 'content-type: application/json' \
  --data '{"query":"{ __schema { queryType { name } } }"}' \
  | jq -r '.data.__schema.queryType.name'
```

## gRPC (Server Reflection)

- Endpoint (plaintext): `grpcb.in:9000`
- Endpoint (TLS): `grpcb.in:9001`
- Quick checks:

```bash
uxc grpcb.in:9000 -h
grpcurl -plaintext grpcb.in:9000 list
grpcurl grpcb.in:9001 list
```

## MCP (HTTP)

- Endpoint: `https://mcp.deepwiki.com/mcp`
- Quick checks:

```bash
uxc mcp.deepwiki.com/mcp -h
curl -sS https://mcp.deepwiki.com/mcp \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"uxc-check","version":"0.1"}}}'
```

Note: DeepWiki MCP can return streamable HTTP/SSE payloads.

## MCP (stdio, local no-key baseline)

- Command: `npx -y @modelcontextprotocol/server-filesystem /tmp`
- Quick checks:

```bash
uxc "npx -y @modelcontextprotocol/server-filesystem /tmp" -h
uxc "npx -y @modelcontextprotocol/server-filesystem /tmp" list_directory path=/tmp
```

## JSON-RPC

- Endpoint: `https://fullnode.mainnet.sui.io`
- Quick checks:

```bash
uxc fullnode.mainnet.sui.io -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber
```

## Notes

- Public endpoints may change behavior, schemas, or rate limits over time.
- For CI stability, prefer local mock/test servers where possible.
