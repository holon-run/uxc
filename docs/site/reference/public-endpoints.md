# Public Endpoints

These endpoints are useful for protocol availability checks without API keys.

## OpenAPI

- Endpoint: `https://petstore3.swagger.io/api/v3`

```bash
uxc petstore3.swagger.io/api/v3 -h
```

## GraphQL

- Endpoint: `https://countries.trevorblades.com/`

```bash
uxc countries.trevorblades.com -h
```

## gRPC

- Plaintext: `grpcb.in:9000`
- TLS: `grpcb.in:9001`

```bash
uxc grpcb.in:9000 -h
```

## MCP HTTP

- Endpoint: `https://mcp.deepwiki.com/mcp`

```bash
uxc mcp.deepwiki.com/mcp -h
```

## MCP stdio

- Command: `npx -y @modelcontextprotocol/server-filesystem /tmp`

```bash
uxc "npx -y @modelcontextprotocol/server-filesystem /tmp" -h
```

## JSON-RPC

- Endpoint: `https://fullnode.mainnet.sui.io`

```bash
uxc fullnode.mainnet.sui.io -h
```

## Notes

Public endpoints may change behavior or rate limits over time.
For CI stability, prefer local mock or test servers when possible.
