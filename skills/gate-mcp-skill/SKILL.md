---
name: gate-mcp-skill
description: Use Gate MCP through UXC for public spot and futures market data workflows with a fixed streamable-http endpoint and read-first guardrails.
---

# Gate MCP Skill

Use this skill to run Gate public market data workflows through `uxc` + MCP.

Reuse the `uxc` skill for shared MCP execution, output parsing, and generic auth guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.gatemcp.ai/mcp`.

## Scope

This skill covers the official Gate MCP surface for:

- public spot market discovery
- public spot tickers, trades, order books, and candlesticks
- public futures contracts, tickers, trades, order books, funding, and premium data

This skill does **not** cover:

- private account or trading workflows
- Gate REST v4 signed APIs
- wallet, funding, or portfolio actions

## Endpoint

Use Gate's official MCP endpoint:

- `https://api.gatemcp.ai/mcp`

## Core Workflow

1. Use the fixed link command by default:
   - `command -v gate-mcp-cli`
   - If missing, create it:
     `uxc link gate-mcp-cli https://api.gatemcp.ai/mcp`
   - `gate-mcp-cli -h`
2. Inspect tool and argument help before execution:
   - `gate-mcp-cli cex_spot_get_spot_tickers -h`
   - `gate-mcp-cli cex_spot_get_spot_order_book -h`
   - `gate-mcp-cli cex_fx_get_fx_tickers -h`
   - `gate-mcp-cli cex_fx_get_fx_order_book -h`
3. Prefer narrow spot or futures reads with explicit instrument identifiers:
   - `gate-mcp-cli cex_spot_get_spot_tickers currency_pair=BTC_USDT`
   - `gate-mcp-cli cex_spot_get_spot_order_book currency_pair=BTC_USDT limit=20`
   - `gate-mcp-cli cex_fx_get_fx_tickers contract=BTC_USDT`
   - `gate-mcp-cli cex_fx_get_fx_funding_rate contract=BTC_USDT`

## Operations

### Spot

- `cex_spot_list_currencies`
- `cex_spot_list_currency_pairs`
- `cex_spot_get_currency`
- `cex_spot_get_currency_pair`
- `cex_spot_get_spot_tickers`
- `cex_spot_get_spot_order_book`
- `cex_spot_get_spot_trades`
- `cex_spot_get_spot_candlesticks`

### Futures

- `cex_fx_list_fx_contracts`
- `cex_fx_get_fx_contract`
- `cex_fx_get_fx_tickers`
- `cex_fx_get_fx_order_book`
- `cex_fx_get_fx_trades`
- `cex_fx_get_fx_candlesticks`
- `cex_fx_get_fx_funding_rate`
- `cex_fx_get_fx_premium_index`
- `cex_fx_list_fx_liq_orders`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Prefer `currency_pair` values like `BTC_USDT` for spot and `contract` values like `BTC_USDT` for futures unless help output indicates a different contract name.
- `gate-mcp-cli <operation> ...` is equivalent to `uxc https://api.gatemcp.ai/mcp <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Official Gate for AI / MCP docs: https://www.gate.com/gate-mcp-skills
- Gate MCP setup article: https://www.gate.com/ru/help/gateforai/gateforaibasics/50102/gate-for-ai-one-click-integration-with-major-ai-agents-no-api-keys-required-zero-barriers
