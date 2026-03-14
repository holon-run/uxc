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

Private account and order endpoints require a Coinbase Advanced Trade bearer JWT. `uxc` now supports Coinbase's request-scoped JWT flow directly through `jwt_bearer_v1`, so you can store the API key id and private key in a credential and let `uxc` mint the short-lived bearer token per request.

Recommended v1 setup:

1. Download or copy the Coinbase API key material:
   - `key_id`: `organizations/{org_id}/apiKeys/{key_id}`
   - `private_key`: Coinbase exports either `-----BEGIN EC PRIVATE KEY-----` or `-----BEGIN PRIVATE KEY-----`; `uxc` accepts both PEM forms for ES256.
2. Store those values in a local credential.
3. Bind the credential to `api.coinbase.com` with a `jwt_bearer_v1` signer.

```bash
uxc auth credential set coinbase-advanced-trade \
  --auth-type api_key \
  --field key_id=env:COINBASE_KEY_ID \
  --field private_key=env:COINBASE_PRIVATE_KEY

uxc auth binding add \
  --id coinbase-advanced-trade \
  --host api.coinbase.com \
  --path-prefix /api/v3/brokerage \
  --scheme https \
  --credential coinbase-advanced-trade \
  --signer-json '{"kind":"jwt_bearer_v1","algorithm":"es256","private_key_field":"private_key","header_typ":"JWT","header_kid_field":"key_id","expires_in_seconds":120,"claims":{"static":{"iss":"cdp"},"from_fields":{"sub":"key_id"},"time":{"nbf":"now","exp":"now_plus_ttl"}},"request_claim":{"name":"uri","format":"string","value_template":"{{request.method}} {{request.host}}{{request.path}}"}}' \
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
- `uxc` mints a fresh short-lived Coinbase JWT on each private request; do not try to bind a stale pre-generated bearer token when `jwt_bearer_v1` is available.
- Coinbase exports ES256 private keys in more than one PEM form; this skill expects the raw downloaded PEM and does not require a manual PKCS#8 conversion step.
- Treat `post:/api/v3/brokerage/orders` and `post:/api/v3/brokerage/orders/batch_cancel` as high-risk writes.
- Keep initial product/account pulls narrow with small `limit` values.
- `coinbase-openapi-cli <operation> ...` is equivalent to `uxc https://api.coinbase.com --schema-url <coinbase_advanced_trade_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/coinbase-advanced-trade.openapi.json`
- Coinbase Advanced Trade overview: https://docs.cdp.coinbase.com/coinbase-app/advanced-trade-apis/overview
