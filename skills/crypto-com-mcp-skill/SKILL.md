---
name: crypto-com-mcp-skill
description: Use Crypto.com MCP through UXC for exchange market data workflows with help-first discovery and read-only guardrails.
---

# Crypto.com MCP Skill

Use this skill to run Crypto.com exchange market data workflows through `uxc` + MCP.

Reuse the `uxc` skill for shared MCP execution, output parsing, and auth handling.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://mcp.crypto.com/market-data/mcp`.
- Access to the official Crypto.com MCP docs:
  - `https://mcp.crypto.com/docs`

## Scope

This skill covers the official Crypto.com MCP surface for:

- exchange market discovery
- ticker and order book style reads
- read-only market data workflows

This skill does **not** cover:

- exchange trading or account writes
- private account workflows
- non-MCP REST or WebSocket product families

## Endpoint

Use the official Crypto.com MCP endpoint:

- `https://mcp.crypto.com/market-data/mcp`

## Core Workflow

1. Use the fixed link command by default:
   - `command -v crypto-com-mcp-cli`
   - If missing, create it:
     `uxc link crypto-com-mcp-cli https://mcp.crypto.com/market-data/mcp`
2. Inspect tool and argument help before execution:
   - `crypto-com-mcp-cli -h`
   - `crypto-com-mcp-cli get_ticker -h`
   - `crypto-com-mcp-cli get_book -h`
   - `crypto-com-mcp-cli get_candlestick -h`
3. Prefer read-only market queries and keep instruments and limits narrow:
   - `crypto-com-mcp-cli get_ticker instrument_name=BTC_USDT`
   - `crypto-com-mcp-cli get_book instrument_name=BTC_USDT depth=20`
   - `crypto-com-mcp-cli get_candlestick instrument_name=BTC_USDT timeframe=1h`

## Operations

- `get_instruments`
- `get_instrument`
- `get_ticker`
- `get_tickers`
- `get_book`
- `get_index_price`
- `get_mark_price`
- `get_candlestick`
- `get_trades`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Treat this as market-data only. Do not imply trading, balances, or private account access.
- `crypto-com-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.crypto.com/market-data/mcp <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Official Crypto.com MCP docs: https://mcp.crypto.com/docs
