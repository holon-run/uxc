---
name: coinbase-openapi-skill
description: Operate Coinbase Advanced Trade REST APIs through UXC with a curated OpenAPI schema, products-first discovery, and explicit JWT bearer auth guidance.
---

# Coinbase Advanced Trade Skill

Use this skill to run Coinbase Advanced Trade REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.coinbase.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/coinbase-openapi-skill/references/coinbase-advanced-trade.openapi.json`

## Scope

This skill covers a curated Coinbase Advanced Trade surface for:

- product and best-bid-ask market reads
- account summary reads
- order create, cancel, and lookup workflows

This skill does **not** cover:

- Coinbase Exchange APIs
- Coinbase Prime APIs
- Coinbase Derivatives APIs
- wallet or retail app product families outside Advanced Trade

## Authentication

Public product endpoints can be read without credentials.

Private account and order endpoints require a Coinbase Advanced Trade bearer JWT. In practice, Coinbase expects short-lived JWTs generated from a Coinbase API key + private key pair outside `uxc`.

Recommended v1 setup:

1. Generate the Advanced Trade JWT externally using Coinbase's documented signing flow.
2. Export the current token into an environment variable.
3. Bind it as a bearer credential to `api.coinbase.com`.

```bash
uxc auth credential set coinbase-advanced-trade \
  --auth-type bearer \
  --secret-env COINBASE_ADVANCED_TRADE_JWT

uxc auth binding add \
  --id coinbase-advanced-trade \
  --host api.coinbase.com \
  --path-prefix /api/v3/brokerage \
  --scheme https \
  --credential coinbase-advanced-trade \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://api.coinbase.com/api/v3/brokerage/accounts
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v coinbase-openapi-cli`
   - If missing, create it:
     `uxc link coinbase-openapi-cli https://api.coinbase.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/coinbase-openapi-skill/references/coinbase-advanced-trade.openapi.json`
   - `coinbase-openapi-cli -h`

2. Inspect operation help before execution:
   - `coinbase-openapi-cli get:/api/v3/brokerage/products -h`
   - `coinbase-openapi-cli get:/api/v3/brokerage/accounts -h`
   - `coinbase-openapi-cli post:/api/v3/brokerage/orders -h`

3. Prefer product reads before private account or order workflows:
   - `coinbase-openapi-cli get:/api/v3/brokerage/products product_type=SPOT limit=20`
   - `coinbase-openapi-cli get:/api/v3/brokerage/best_bid_ask product_ids=BTC-USD,ETH-USD`

4. Treat all order placement and cancellation as high-risk writes.

## Operations

- `get:/api/v3/brokerage/products`
- `get:/api/v3/brokerage/products/{product_id}`
- `get:/api/v3/brokerage/best_bid_ask`
- `get:/api/v3/brokerage/accounts`
- `get:/api/v3/brokerage/accounts/{account_uuid}`
- `post:/api/v3/brokerage/orders`
- `post:/api/v3/brokerage/orders/batch_cancel`
- `get:/api/v3/brokerage/orders/historical/{order_id}`
- `get:/api/v3/brokerage/orders/historical/batch`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Refresh `COINBASE_ADVANCED_TRADE_JWT` before private calls because Coinbase JWTs are short-lived.
- Treat `post:/api/v3/brokerage/orders` and `post:/api/v3/brokerage/orders/batch_cancel` as high-risk writes.
- Keep initial product/account pulls narrow with small `limit` values.
- `coinbase-openapi-cli <operation> ...` is equivalent to `uxc https://api.coinbase.com --schema-url <coinbase_advanced_trade_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/coinbase-advanced-trade.openapi.json`
- Coinbase Advanced Trade overview: https://docs.cdp.coinbase.com/coinbase-app/advanced-trade-apis/overview
