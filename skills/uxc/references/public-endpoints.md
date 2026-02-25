# Public Endpoints

The following endpoints are practical no-key baselines for smoke checks. Availability can change over time.

## OpenAPI

- Endpoint: `https://petstore3.swagger.io/api/v3`
- Check:

```bash
uxc https://petstore3.swagger.io/api/v3 list
uxc https://petstore3.swagger.io/api/v3 get:/store/inventory
```

## GraphQL

- Endpoint: `https://countries.trevorblades.com/`
- Check:

```bash
uxc https://countries.trevorblades.com/ list
uxc https://countries.trevorblades.com/ query/country --json '{"code":"US"}'
```

## gRPC

- Endpoint: `grpcb.in:9000` (plaintext), `grpcb.in:9001` (TLS)
- Prerequisite: `grpcurl` installed
- Check:

```bash
uxc grpcb.in:9000 list
uxc grpcb.in:9000 addsvc.Add/Sum --json '{"a":1,"b":2}'
```

## MCP (HTTP)

- Endpoint: `https://mcp.deepwiki.com/mcp`
- Check:

```bash
uxc https://mcp.deepwiki.com/mcp list
uxc https://mcp.deepwiki.com/mcp describe ask_question
```

## JSON-RPC

- Constraint: UXC JSON-RPC discovery requires `openrpc.json`, `/.well-known/openrpc.json`, or `rpc.discover`.
- Current status: no stable, keyless public endpoint is curated in this repository.
- Recommended baseline for reliable tests:
  - use a controlled endpoint that exposes OpenRPC/rpc.discover
  - or run local/self-hosted JSON-RPC with OpenRPC enabled
