# Usage Patterns

All commands in this skill use the native MCP HTTP endpoint:
`https://token-api.mcp.thegraph.com/`

This skill defaults to fixed link command `thegraph-token-mcp-cli`.
Create it when missing:

```bash
command -v thegraph-token-mcp-cli
uxc auth credential set thegraph --secret-env THEGRAPH_API_KEY
uxc auth binding add --id thegraph-token-mcp --host token-api.mcp.thegraph.com --scheme https --credential thegraph --priority 100
uxc link thegraph-token-mcp-cli https://token-api.mcp.thegraph.com/
thegraph-token-mcp-cli -h
```

Notes:

- Auth is handled through standard `uxc auth credential` + `binding`.
- Reuse the existing `thegraph` credential because the same The Graph API key works for Subgraph MCP and Token API MCP.
- Check the active binding with `uxc auth binding match https://token-api.mcp.thegraph.com/`.

## Discover And Inspect

```bash
thegraph-token-mcp-cli -h
thegraph-token-mcp-cli getV1Networks -h
thegraph-token-mcp-cli getV1EvmTokens -h
thegraph-token-mcp-cli getV1EvmBalances -h
```

## Network Discovery

```bash
thegraph-token-mcp-cli getV1Networks
```

Use this first to confirm supported network identifiers before querying balances, tokens, or pools.

## Token Metadata

```bash
thegraph-token-mcp-cli getV1EvmTokens network=base contract=0x4200000000000000000000000000000000000006
```

Native token metadata:

```bash
thegraph-token-mcp-cli getV1EvmTokensNative network=base
```

## Wallet Balances

```bash
thegraph-token-mcp-cli getV1EvmBalances network=base address=0xYourAddress
```

Use operation help before querying transfers, holders, or market surfaces because exact operation names may evolve.

## Practical Rules

- Start with one network and one address/contract.
- Prefer operation-level help before relying on less common endpoints.
- Use positional JSON for nested inputs or arrays.
- Keep initial queries narrow and inspect response shape before widening scope.

## Fallback Equivalence

- `thegraph-token-mcp-cli <operation> ...` is equivalent to `uxc https://token-api.mcp.thegraph.com/ <operation> ...` when the same auth binding is configured.
- If link setup is temporarily unavailable, use the direct `uxc "<endpoint>" ...` form as fallback.
