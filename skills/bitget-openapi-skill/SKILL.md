---
name: bitget-openapi-skill
description: Operate Bitget public exchange market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.
---

# Bitget Exchange Skill

Use this skill to run Bitget public market-data operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.bitget.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/bitget-openapi-skill/references/bitget-v2.openapi.json`

## Scope

This skill covers a curated Bitget public market surface for:

- spot symbols and metadata
- ticker reads
- candlestick reads
- order book snapshots

This skill does **not** cover:

- private account endpoints in v1
- private order placement or cancellation in v1
- copy trading or P2P workflows

## Authentication

Public market endpoints in this skill do not require credentials.

Bitget private APIs use provider-specific header signing and timestamp headers. Keep this v1 skill public-data-only until a reusable Bitget signer flow exists in `uxc`.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v bitget-openapi-cli`
   - If missing, create it:
     `uxc link bitget-openapi-cli https://api.bitget.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/bitget-openapi-skill/references/bitget-v2.openapi.json`
   - `bitget-openapi-cli -h`

2. Inspect operation help before execution:
   - `bitget-openapi-cli get:/api/v2/spot/public/symbols -h`
   - `bitget-openapi-cli get:/api/v2/spot/market/tickers -h`

3. Prefer narrow spot reads first:
   - `bitget-openapi-cli get:/api/v2/spot/market/tickers symbol=BTCUSDT`
   - `bitget-openapi-cli get:/api/v2/spot/market/orderbook symbol=BTCUSDT limit=20`

## Operations

- `get:/api/v2/spot/public/symbols`
- `get:/api/v2/spot/market/tickers`
- `get:/api/v2/spot/market/candles`
- `get:/api/v2/spot/market/orderbook`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Keep symbol and candle ranges narrow unless the user explicitly wants a broader pull.
- `bitget-openapi-cli <operation> ...` is equivalent to `uxc https://api.bitget.com --schema-url <bitget_v2_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/bitget-v2.openapi.json`
- Official Bitget API intro: https://www.bitget.com/api-doc/common/intro
