---
name: kucoin-openapi-skill
description: Operate KuCoin public exchange market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.
---

# KuCoin Unified API Skill

Use this skill to run KuCoin public market-data operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.kucoin.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/kucoin-openapi-skill/references/kucoin-public.openapi.json`

## Scope

This skill covers a curated KuCoin public market surface for:

- symbol discovery
- all tickers
- order book snapshots
- candlestick reads

This skill does **not** cover:

- private account or order endpoints in v1
- websocket, margin, or broader platform products

## Authentication

Public market endpoints in this skill do not require credentials.

KuCoin private REST auth uses provider-specific headers and signing rules including passphrase handling. Keep this v1 skill public-data-only until a reusable KuCoin signer flow exists in `uxc`.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v kucoin-openapi-cli`
   - If missing, create it:
     `uxc link kucoin-openapi-cli https://api.kucoin.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/kucoin-openapi-skill/references/kucoin-public.openapi.json`
   - `kucoin-openapi-cli -h`

2. Inspect operation help before execution:
   - `kucoin-openapi-cli get:/api/v1/symbols -h`
   - `kucoin-openapi-cli get:/api/v1/market/allTickers -h`

3. Prefer narrow symbol reads first:
   - `kucoin-openapi-cli get:/api/v1/market/orderbook/level2_20 symbol=BTC-USDT`
   - `kucoin-openapi-cli get:/api/v1/market/candles symbol=BTC-USDT type=1hour`

## Operations

- `get:/api/v1/symbols`
- `get:/api/v1/market/allTickers`
- `get:/api/v1/market/orderbook/level2_20`
- `get:/api/v1/market/candles`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- KuCoin symbols use dash-separated names such as `BTC-USDT`.
- `kucoin-openapi-cli <operation> ...` is equivalent to `uxc https://api.kucoin.com --schema-url <kucoin_public_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/kucoin-public.openapi.json`
- Official KuCoin auth docs: https://www.kucoin.com/docs-new/authentication
