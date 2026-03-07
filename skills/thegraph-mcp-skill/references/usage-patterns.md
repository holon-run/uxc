# Usage Patterns

All commands in this skill use the native MCP SSE endpoint:
`https://subgraphs.mcp.thegraph.com/sse`

This skill defaults to fixed link command `thegraph-mcp-cli`.
Create it when missing:

```bash
command -v thegraph-mcp-cli
uxc auth credential set thegraph --secret-env THEGRAPH_API_KEY
uxc auth binding add --id thegraph-sse --host subgraphs.mcp.thegraph.com --path-prefix /sse --scheme https --credential thegraph --priority 100
uxc link thegraph-mcp-cli https://subgraphs.mcp.thegraph.com/sse
thegraph-mcp-cli -h
```

Notes:

- This skill now uses native SSE support in `uxc`.
- Auth is handled through standard `uxc auth credential` + `binding`.
- Check the active binding with `uxc auth binding match https://subgraphs.mcp.thegraph.com/sse`.

## Discover And Inspect

```bash
thegraph-mcp-cli -h
thegraph-mcp-cli <operation> -h
```

Use help-first inspection to identify the exact operation names exposed by the bridge in your current version.

## Read-First Flow

Recommended workflow:

1. Search candidate subgraphs or deployments
2. Inspect schema for the chosen deployment/subgraph
3. Run a minimal GraphQL query with limited fields and rows

## Example Query Payload Pattern

Most query execution operations accept either key/value fields or positional JSON.
Prefer positional JSON for GraphQL requests with embedded query text:

```bash
thegraph-mcp-cli <query-operation> '{"query":"{ _meta { deployment } }"}'
```

When the operation requires a deployment ID, subgraph ID, or IPFS hash, inspect `-h` first and include only the required identifier plus the GraphQL query.

## Example Discovery Pattern

Search by keyword first, then pivot to schema:

```bash
thegraph-mcp-cli <search-operation> keyword=uniswap
thegraph-mcp-cli <schema-operation> deploymentId=Qm...
```

Contract-oriented discovery:

```bash
thegraph-mcp-cli <contract-discovery-operation> contractAddress=0x...
```

## Practical Rules For GraphQL Through MCP

- Start with `_meta` or a tiny entity selection before issuing wide queries.
- Always add `first`, `where`, or other narrowing arguments on the first pass.
- Prefer deployment-based execution for stable repeated workflows.
- Use subgraph-level selection when you intentionally want the latest version.

## Output Parsing

Rely on envelope fields:
- Success: `ok == true`, consume `data`
- Failure: `ok == false`, inspect `error.code` and `error.message`

## Fallback Equivalence

- `thegraph-mcp-cli <operation> ...` is equivalent to `uxc https://subgraphs.mcp.thegraph.com/sse <operation> ...` when the same auth binding is configured.
- If link setup is temporarily unavailable, use the direct `uxc "<endpoint>" ...` form as fallback.
