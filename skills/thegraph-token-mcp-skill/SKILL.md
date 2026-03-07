---
name: thegraph-token-mcp-skill
description: Use The Graph Token API MCP through UXC for token metadata, wallet balances, transfers, holders, pools, and market data with help-first inspection and standard auth binding.
---

# The Graph Token MCP Skill

Use this skill to run The Graph Token API MCP operations through `uxc`.

Reuse the `uxc` skill for generic protocol discovery, envelope parsing, and error handling rules.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://token-api.mcp.thegraph.com/`.
- The Graph Token API access token is available for authenticated calls.

## Core Workflow

1. Verify endpoint and protocol with help-first probing:
   - `uxc https://token-api.mcp.thegraph.com/ -h`
   - Confirm protocol is MCP (`protocol == "mcp"` in envelope).
2. Configure credential and auth binding:
   - `uxc auth credential set thegraph-token --auth-type api_key --header "Authorization=Bearer {{secret}}" --secret-env THEGRAPH_TOKEN_API_KEY`
   - `uxc auth binding add --id thegraph-token-mcp --host token-api.mcp.thegraph.com --scheme https --credential thegraph-token --priority 100`
3. Use fixed link command by default:
   - `command -v thegraph-token-mcp-cli`
   - If missing, create it:
     - `uxc link thegraph-token-mcp-cli https://token-api.mcp.thegraph.com/`
   - `thegraph-token-mcp-cli -h`
4. Inspect operation schema before execution:
   - `thegraph-token-mcp-cli getV1Networks -h`
   - `thegraph-token-mcp-cli getV1EvmTokens -h`
   - `thegraph-token-mcp-cli getV1EvmBalances -h`
5. Prefer read operations first, then narrower wallet/token/pool queries.

## Capability Map

- Service discovery:
  - `getV1Health`
  - `getV1Version`
  - `getV1Networks`
- Token data:
  - `getV1EvmTokens`
  - `getV1EvmTokensNative`
- Wallet and transfer data:
  - `getV1EvmBalances`
  - transfer/history operations exposed by the endpoint
- Market and DEX data:
  - pool / OHLC / dex operations exposed by the endpoint
- Non-EVM coverage:
  - TVM and other chain families exposed by the endpoint

Always inspect host help and operation help in the current endpoint version before relying on an operation name or argument shape.

## Recommended Usage Pattern

1. Start with network discovery:
   - `thegraph-token-mcp-cli getV1Networks`
2. Confirm the operation and required arguments with `-h`.
3. Query the narrowest surface first:
   - token metadata for one contract
   - balances for one address
   - one pool / one token / one date range
4. Expand to broader scans only when needed.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `thegraph-token-mcp-cli` as default command path.
- `thegraph-token-mcp-cli <operation> ...` is equivalent to `uxc https://token-api.mcp.thegraph.com/ <operation> ...` when the same auth binding is configured.
- Use direct `uxc "<endpoint>" ...` only as temporary fallback when link setup is unavailable.
- Prefer `key=value` for simple arguments and positional JSON for nested objects.
- If auth fails:
  - confirm `uxc auth credential info thegraph-token` succeeds
  - confirm `uxc auth binding match https://token-api.mcp.thegraph.com/` resolves to `thegraph-token`
  - rerun `thegraph-token-mcp-cli -h`

## Tested Real Scenario

The endpoint was verified through `uxc` host discovery and returned a live MCP tool list including:

- `getV1Health`
- `getV1Version`
- `getV1Networks`
- `getV1EvmTokens`
- `getV1EvmTokensNative`
- `getV1EvmBalances`

This confirms the skill target is a real MCP surface rather than a direct OpenAPI host.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
