---
name: okx-mcp-skill
description: Use OKX OnchainOS MCP through UXC for token discovery, market data, wallet balance, and swap execution planning. Use when tasks need OKX MCP tools such as token search/ranking/holder, price/trades/candlesticks/index, balance queries, and DEX quote/swap flows with help-first schema inspection and safe auth handling.
---

# OKX MCP Skill

Use this skill to run OKX MCP operations through `uxc`.

Reuse the `uxc` skill for shared protocol discovery, output parsing, and generic auth/binding flows.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://web3.okx.com/api/v1/onchainos-mcp`.
- OKX Onchain API key is available for real calls.

## Quick Trial Key

For quick read-only trial, OKX docs currently expose a shared demo key:

- `d573a84c-8e79-4a35-b0c6-427e9ad2478d`

Example:

- `uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret d573a84c-8e79-4a35-b0c6-427e9ad2478d`

Use your own key for regular usage, stability, and production workflows.

## Core Workflow

1. Confirm endpoint and protocol with help-first probing:
   - `uxc https://web3.okx.com/api/v1/onchainos-mcp -h`
2. Configure credential/binding for repeatable auth:
   - `uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-env OKX_ACCESS_KEY`
   - `uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-op op://Engineering/okx/OK-ACCESS-KEY`
   - `uxc auth binding add --id okx-mcp --host web3.okx.com --path-prefix /api/v1/onchainos-mcp --scheme https --credential okx-mcp --priority 100`
3. Use fixed link command by default:
   - `command -v okx-mcp-cli`
   - If missing, create it: `uxc link okx-mcp-cli https://web3.okx.com/api/v1/onchainos-mcp`
   - `okx-mcp-cli -h`
4. Inspect operation schema before execution:
   - `okx-mcp-cli dex-okx-market-price -h`
   - `okx-mcp-cli dex-okx-market-token-search -h`
   - `okx-mcp-cli dex-okx-balance-total-value -h`
   - `okx-mcp-cli dex-okx-dex-quote -h`
5. Prefer read operations first, then high-impact operations.

## Capability Map

- Market:
  - `dex-okx-market-price`
  - `dex-okx-market-trades`
  - `dex-okx-market-candlesticks`
  - `dex-okx-market-candlesticks-history`
  - `dex-okx-index-current-price`
  - `dex-okx-index-historical-price`
- Token discovery/enrichment:
  - `dex-okx-market-token-search`
  - `dex-okx-market-token-ranking`
  - `dex-okx-market-token-holder`
  - `dex-okx-market-token-basic-info`
  - `dex-okx-market-token-price-info`
- Wallet:
  - `dex-okx-balance-chains`
  - `dex-okx-balance-total-value`
  - `dex-okx-balance-total-token-balances`
  - `dex-okx-balance-specific-token-balance`
- Swap:
  - `dex-okx-dex-aggregator-supported-chains`
  - `dex-okx-dex-liquidity`
  - `dex-okx-dex-quote`
  - `dex-okx-dex-approve-transaction`
  - `dex-okx-dex-swap`
  - `dex-okx-dex-solana-swap-instruction`

## ChainIndex Quick Reference

- Ethereum: `1`
- BSC: `56`
- XLayer: `196`
- Solana: `501`
- Base: `8453`
- Arbitrum: `42161`

Always prefer runtime discovery (`dex-okx-market-price-chains` / `dex-okx-dex-aggregator-supported-chains` / `dex-okx-balance-chains`) when possible.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `okx-mcp-cli` as default command path.
- `okx-mcp-cli <operation> ...` is equivalent to `uxc https://web3.okx.com/api/v1/onchainos-mcp <operation> ...`.
- If call result contains `Request header OK-ACCESS-KEY can not be empty`:
  - confirm auth binding matches endpoint with `uxc auth binding match https://web3.okx.com/api/v1/onchainos-mcp`
  - update credential with explicit header: `uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-env OKX_ACCESS_KEY`
  - confirm credential has a secret source (`--secret-env`, `--secret-op`, or literal `--secret`)
- For high-impact operations require explicit user confirmation:
  - `dex-okx-dex-approve-transaction`
  - `dex-okx-dex-swap`
  - `dex-okx-dex-solana-swap-instruction`
- Solana caveat from OKX docs:
  - for market candles/trades, use wSOL token address `So11111111111111111111111111111111111111112`
  - for some price contexts, SOL system address `11111111111111111111111111111111` may also appear in docs
  - when data is empty on Solana, verify operation-level address expectation first.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
