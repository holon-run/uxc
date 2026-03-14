---
name: lifi-mcp-skill
description: Use the LI.FI MCP server through UXC for cross-chain route discovery, bridge/DEX availability checks, token and chain lookup, gas/balance/allowance checks, quote generation, and transfer status tracking. Use when tasks involve planning or monitoring cross-chain swaps and bridges without signing or broadcasting transactions.
---

# LI.FI MCP Skill

Use this skill to run LI.FI MCP operations through `uxc`.

Reuse the `uxc` skill for generic protocol discovery, auth binding, envelope parsing, and error handling.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://mcp.li.quest/mcp`.
- Optional: a LI.FI API key for higher rate limits.

## Core Workflow

1. Verify endpoint and protocol with help-first probing:
   - `uxc https://mcp.li.quest/mcp -h`
   - Confirm protocol is MCP (`protocol == "mcp"` in the envelope).
2. Optional auth setup for higher rate limits:
   - `uxc auth credential set lifi-mcp --auth-type bearer --secret-env LIFI_API_KEY`
   - `uxc auth binding add --id lifi-mcp --host mcp.li.quest --path-prefix /mcp --scheme https --credential lifi-mcp --priority 100`
   - LI.FI accepts either `Authorization: Bearer <key>` or `X-LiFi-Api-Key`; prefer bearer unless endpoint behavior changes.
3. Use a fixed link command by default:
   - `command -v lifi-mcp-cli`
   - If missing, create it:
     - `uxc link lifi-mcp-cli https://mcp.li.quest/mcp`
   - `lifi-mcp-cli -h`
4. Inspect operation schema before execution:
   - `lifi-mcp-cli get-chains -h`
   - `lifi-mcp-cli get-token -h`
   - `lifi-mcp-cli get-quote -h`
   - `lifi-mcp-cli get-status -h`
5. Prefer discovery first, then route/quote, then execution-precheck queries.

## Capability Map

- Chain and token discovery:
  - `get-chains`
  - `get-chain-by-id`
  - `get-chain-by-name`
  - `get-tokens`
  - `get-token`
- Route discovery and provider availability:
  - `get-connections`
  - `get-tools`
- Quotes and route planning:
  - `get-quote`
  - `get-routes`
  - `get-step-transaction`
  - `get-quote-with-calls`
- Wallet prechecks:
  - `get-native-token-balance`
  - `get-token-balance`
  - `get-allowance`
  - `get-gas-prices`
  - `get-gas-suggestion`
- Monitoring and service checks:
  - `get-status`
  - `test-api-key`
  - `health-check`

Always inspect host help and operation help in the current endpoint version before relying on an operation name or argument shape.

## Recommended Usage Pattern

1. Discover chain IDs dynamically:
   - `lifi-mcp-cli get-chains`
   - `lifi-mcp-cli get-chain-by-name name=base`
2. Resolve token addresses before quoting:
   - `lifi-mcp-cli get-token chain=base token=USDC`
3. Check whether a route exists before asking for a quote:
   - `lifi-mcp-cli get-connections fromChain=8453 toChain=42161`
4. Generate the quote:
   - `lifi-mcp-cli get-quote fromChain=8453 toChain=42161 fromToken=USDC toToken=USDC fromAddress=<wallet> fromAmount=<smallest-unit-amount>`
5. Validate execution preconditions:
   - `lifi-mcp-cli get-allowance ...`
   - `lifi-mcp-cli get-native-token-balance ...`
6. Use `get-status` only after the externally signed transaction has been broadcast.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `lifi-mcp-cli` as the default command path.
- `lifi-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.li.quest/mcp <operation> ...` when the same auth binding is configured.
- Use direct `uxc "<endpoint>" ...` only as a temporary fallback when link setup is unavailable.
- Prefer `key=value` for simple arguments and positional JSON for nested objects.
- This endpoint is read-only from the agent perspective:
  - it does **not** sign transactions
  - it does **not** broadcast transactions
  - it returns unsigned `transactionRequest` objects for external wallet execution
- Do not present `get-quote` or `get-step-transaction` as executed trades; they are execution plans only.
- Before suggesting an ERC20 route as ready to execute, check allowance if the route needs approval.
- Prefer `get-routes` only when the user explicitly wants multiple alternatives; default to `get-quote` for the best route.

## Tested Real Scenario

The endpoint was verified through `uxc` host discovery and returned a live MCP tool list including:

- `get-allowance`
- `get-chain-by-id`
- `get-chain-by-name`
- `get-chains`
- `get-connections`
- `get-gas-prices`
- `get-gas-suggestion`
- `get-quote`
- `get-routes`
- `get-status`
- `get-token`
- `get-tokens`
- `get-tools`
- `health-check`

This confirms the skill target is a real hosted MCP surface accessible via UXC.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
