---
name: coinapi-openapi-skill
description: Operate CoinAPI market data reads through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.
---

# CoinAPI REST Skill

Use this skill to run CoinAPI REST market data operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://rest.coinapi.io`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/coinapi-openapi-skill/references/coinapi-market.openapi.json`
- A CoinAPI key.

## Scope

This skill covers a read-first market data surface:

- current exchange rates
- current quote snapshots
- latest OHLCV candles
- latest trades
- latest order books

This skill does **not** cover:

- FIX, WebSocket, or MCP transport surfaces
- write operations
- the broader CoinAPI catalog

## Authentication

CoinAPI uses `X-CoinAPI-Key` header auth.

Configure one API-key credential and bind it to `rest.coinapi.io`:

```bash
uxc auth credential set coinapi \
  --auth-type api_key \
  --api-key-header X-CoinAPI-Key \
  --secret-env COINAPI_KEY

uxc auth binding add \
  --id coinapi \
  --host rest.coinapi.io \
  --scheme https \
  --credential coinapi \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://rest.coinapi.io
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v coinapi-openapi-cli`
   - If missing, create it:
     `uxc link coinapi-openapi-cli https://rest.coinapi.io --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/coinapi-openapi-skill/references/coinapi-market.openapi.json`
   - `coinapi-openapi-cli -h`

2. Inspect operation schema first:
   - `coinapi-openapi-cli get:/v1/exchangerate/{asset_id_base}/{asset_id_quote} -h`
   - `coinapi-openapi-cli get:/v1/ohlcv/{symbol_id}/latest -h`
   - `coinapi-openapi-cli get:/v1/trades/{symbol_id}/latest -h`

3. Prefer narrow spot and latest reads before broader crawls:
   - `coinapi-openapi-cli get:/v1/exchangerate/{asset_id_base}/{asset_id_quote} asset_id_base=BTC asset_id_quote=USD`
   - `coinapi-openapi-cli get:/v1/quotes/current filter_symbol_id=BINANCE_SPOT_BTC_USDT`

4. Execute with key/value parameters:
   - `coinapi-openapi-cli get:/v1/ohlcv/{symbol_id}/latest symbol_id=BINANCE_SPOT_BTC_USDT period_id=1DAY limit=10`
   - `coinapi-openapi-cli get:/v1/orderbooks/{symbol_id}/latest symbol_id=BINANCE_SPOT_BTC_USDT limit_levels=20`

## Operations

- `get:/v1/exchangerate/{asset_id_base}/{asset_id_quote}`
- `get:/v1/quotes/current`
- `get:/v1/ohlcv/{symbol_id}/latest`
- `get:/v1/trades/{symbol_id}/latest`
- `get:/v1/orderbooks/{symbol_id}/latest`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply order entry or market connectivity support.
- Keep `filter_symbol_id`, `period_id`, `limit`, and `limit_levels` narrow unless the user explicitly wants larger pulls.
- `coinapi-openapi-cli <operation> ...` is equivalent to `uxc https://rest.coinapi.io --schema-url <coinapi_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/coinapi-market.openapi.json`
- CoinAPI REST docs: https://docs.coinapi.io/market-data/rest-api
