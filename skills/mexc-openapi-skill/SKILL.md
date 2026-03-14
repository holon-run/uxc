---
name: mexc-openapi-skill
description: Operate MEXC Spot REST APIs through UXC with a curated OpenAPI schema, HMAC query signing, and separate public/signed workflow guardrails.
---

# MEXC Spot API Skill

Use this skill to run MEXC Spot REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.mexc.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/mexc-openapi-skill/references/mexc-spot.openapi.json`

## Scope

This skill covers a curated MEXC Spot REST surface for:

- public market reads
- signed account reads
- signed order create, cancel, and lookup flows

This skill does **not** cover:

- futures APIs
- broader platform product families

## Authentication

Public market endpoints do not require credentials.

Signed Spot endpoints require:

- `api_key` field for `X-MEXC-APIKEY`
- `secret_key` field for HMAC SHA256 query signing

Create one credential:

```bash
uxc auth credential set mexc-spot \
  --auth-type api_key \
  --field api_key=env:MEXC_API_KEY \
  --field secret_key=env:MEXC_SECRET_KEY
```

Add one signer binding:

```bash
uxc auth binding add \
  --id mexc-spot \
  --host api.mexc.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential mexc-spot \
  --signer-json '{"kind":"hmac_query_v1","algorithm":"hmac_sha256","signing_field":"secret_key","key_field":"api_key","key_placement":"header","key_name":"X-MEXC-APIKEY","signature_param":"signature","signature_encoding":"hex","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://api.mexc.com/api/v3/account
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v mexc-openapi-cli`
   - If missing, create it:
     `uxc link mexc-openapi-cli https://api.mexc.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/mexc-openapi-skill/references/mexc-spot.openapi.json`
   - `mexc-openapi-cli -h`

2. Inspect operation help before execution:
   - `mexc-openapi-cli get:/api/v3/ticker/price -h`
   - `mexc-openapi-cli get:/api/v3/account -h`
   - `mexc-openapi-cli post:/api/v3/order -h`

3. Prefer public reads first:
   - `mexc-openapi-cli get:/api/v3/ticker/price symbol=BTCUSDT`
   - `mexc-openapi-cli get:/api/v3/depth symbol=BTCUSDT limit=20`

4. Use signed reads before writes:
   - `mexc-openapi-cli get:/api/v3/account recvWindow=5000`
   - `mexc-openapi-cli get:/api/v3/openOrders symbol=BTCUSDT recvWindow=5000`

## Operation Groups

### Public Market

- `get:/api/v3/ping`
- `get:/api/v3/exchangeInfo`
- `get:/api/v3/ticker/price`
- `get:/api/v3/ticker/24hr`
- `get:/api/v3/depth`

### Signed Reads

- `get:/api/v3/account`
- `get:/api/v3/openOrders`
- `get:/api/v3/order`

### Signed Writes

- `post:/api/v3/order`
- `delete:/api/v3/order`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat signed write operations as high-risk and require explicit confirmation before execution.
- `timestamp` and `signature` are injected by the signer binding; users normally provide business parameters plus optional `recvWindow`.
- Query `exchangeInfo` before placing orders so symbol filters and lot sizes are known.
- `mexc-openapi-cli <operation> ...` is equivalent to `uxc https://api.mexc.com --schema-url <mexc_spot_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/mexc-spot.openapi.json`
- Official MEXC Spot v3 docs: https://mexcdevelop.github.io/apidocs/spot_v3_en/
