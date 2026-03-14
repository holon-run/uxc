---
name: bybit-openapi-skill
description: Operate Bybit V5 public market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.
---

# Bybit V5 Skill

Use this skill to run Bybit V5 market-data operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to:
  - `https://api.bybit.com`
  - optionally `https://api-testnet.bybit.com`
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/bybit-openapi-skill/references/bybit-v5.openapi.json`

## Scope

This skill covers a curated Bybit V5 public market surface for:

- server time
- instruments metadata
- tickers
- order book snapshots
- kline reads

This skill does **not** cover:

- private account endpoints in v1
- private order placement or cancellation in v1
- copy trading, earn, broker, or asset management product families

## Authentication

Public market endpoints in this skill do not require credentials.

Bybit private APIs use provider-specific header signing that is not yet packaged as a generic `uxc` signer flow. Keep this v1 skill public-data-only until a reusable Bybit signer path exists.

## Region Guardrail

Bybit's official docs note region and IP restrictions. If requests fail unexpectedly, verify that the current execution environment is permitted for Bybit API access before debugging the schema or parameters.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v bybit-openapi-cli`
   - If missing, create it:
     `uxc link bybit-openapi-cli https://api.bybit.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/bybit-openapi-skill/references/bybit-v5.openapi.json`
   - `bybit-openapi-cli -h`

2. Inspect operation help before execution:
   - `bybit-openapi-cli get:/v5/market/time -h`
   - `bybit-openapi-cli get:/v5/market/instruments-info -h`
   - `bybit-openapi-cli get:/v5/market/tickers -h`

3. Prefer narrow spot reads first:
   - `bybit-openapi-cli get:/v5/market/tickers category=spot symbol=BTCUSDT`
   - `bybit-openapi-cli get:/v5/market/orderbook category=spot symbol=BTCUSDT limit=20`

## Operations

- `get:/v5/market/time`
- `get:/v5/market/instruments-info`
- `get:/v5/market/tickers`
- `get:/v5/market/orderbook`
- `get:/v5/market/kline`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Use `category=spot` unless the user explicitly needs another market family and has checked the symbol format.
- `bybit-openapi-cli <operation> ...` is equivalent to `uxc https://api.bybit.com --schema-url <bybit_v5_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/bybit-v5.openapi.json`
- Official Bybit V5 docs: https://bybit-exchange.github.io/docs/v5/guide
