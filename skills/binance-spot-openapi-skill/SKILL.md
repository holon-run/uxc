---
name: binance-spot-openapi-skill
description: Operate Binance Spot market, account, and order APIs through UXC with a curated OpenAPI schema, Binance query signing, and separate mainnet/testnet link flows.
---

# Binance Spot API Skill

Use this skill to run Binance Spot REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to:
  - `https://api.binance.com`
  - `https://testnet.binance.vision`
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-spot-openapi-skill/references/binance-spot.openapi.json`

## Scope

This skill covers curated Binance Spot REST endpoints for:

- public market reads
- signed account reads
- signed order queries
- test orders
- order placement and cancellation

This skill does **not** cover:

- OCO, OTO, OTOCO, OPO, OPOCO, or other `orderList/*` endpoints
- `order/cancelReplace`
- `order/amend/*`
- `historicalTrades`
- `uiKlines`
- `ticker/tradingDay`
- SOR endpoints
- Wallet, Margin, Earn, or `/sapi/*` endpoints
- RSA request signing
- `https://demo-api.binance.com`

## Authentication

Public market endpoints do not require credentials.

Signed Spot endpoints require:

- `api_key` field for `X-MBX-APIKEY`
- `private_key` field for Ed25519 PKCS#8 PEM signing, or `secret_key` for deprecated HMAC SHA256 signing

### Testnet API Key Setup

Binance Spot testnet uses a separate host and separate API key records from mainnet:

- base URL: `https://testnet.binance.vision`
- testnet API keys do not work on mainnet
- mainnet API keys do not work on testnet

There are two practical testnet flows:

1. `Ed25519` (recommended by Binance)
   - Generate an Ed25519 keypair locally.
   - Register the public key in the Spot testnet API management UI.
   - After registration, Binance shows a distinct `API key` for that Ed25519 key record.
   - Use that displayed `API key` in `X-MBX-APIKEY`, and use the matching private key PEM for signing.

2. `HMAC` (legacy compatibility)
   - Create an HMAC key in the Spot testnet API management UI.
   - Binance shows both `API key` and `Secret key`.
   - Use `API key` in `X-MBX-APIKEY`, and use `Secret key` for HMAC SHA256 signing.

Important:

- The Ed25519 public key you upload is **not** the `API key`.
- Each Binance key record has its own `API key`.
- Do not mix an HMAC `API key` with an Ed25519 private key, or an Ed25519 `API key` with an HMAC secret.
- If you do, Binance returns `-1022 Signature for this request is not valid.`

### Recommended Credential Setup

Binance recommends `Ed25519`. Store the private key PEM text in an environment variable, or source it from 1Password.

```bash
export BINANCE_TESTNET_ED25519_PRIVATE_KEY="$(cat /absolute/path/to/binance_testnet_ed25519_private.pem)"
export BINANCE_MAINNET_ED25519_PRIVATE_KEY="$(cat /absolute/path/to/binance_mainnet_ed25519_private.pem)"
```

Create one credential per environment so mainnet and testnet keys are never mixed:

```bash
uxc auth credential set binance-spot-mainnet \
  --auth-type api_key \
  --field api_key=env:BINANCE_MAINNET_API_KEY \
  --field private_key=env:BINANCE_MAINNET_ED25519_PRIVATE_KEY

uxc auth credential set binance-spot-testnet \
  --auth-type api_key \
  --field api_key=env:BINANCE_TESTNET_API_KEY \
  --field private_key=env:BINANCE_TESTNET_ED25519_PRIVATE_KEY
```

Add one signer binding per environment:

```bash
uxc auth binding add \
  --id binance-spot-mainnet \
  --host api.binance.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-spot-mainnet \
  --signer-json '{"kind":"ed25519_query_v1","algorithm":"ed25519","signing_field":"private_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"base64","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100

uxc auth binding add \
  --id binance-spot-testnet \
  --host testnet.binance.vision \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-spot-testnet \
  --signer-json '{"kind":"ed25519_query_v1","algorithm":"ed25519","signing_field":"private_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"base64","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100
```

### HMAC Fallback

If you already have legacy HMAC keys, `uxc` still supports them:

```bash
uxc auth credential set binance-spot-mainnet-hmac \
  --auth-type api_key \
  --field api_key=env:BINANCE_MAINNET_API_KEY \
  --field secret_key=env:BINANCE_MAINNET_SECRET_KEY

uxc auth credential set binance-spot-testnet-hmac \
  --auth-type api_key \
  --field api_key=env:BINANCE_TESTNET_API_KEY \
  --field secret_key=env:BINANCE_TESTNET_SECRET_KEY

uxc auth binding add \
  --id binance-spot-mainnet-hmac \
  --host api.binance.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-spot-mainnet-hmac \
  --signer-json '{"kind":"hmac_query_v1","algorithm":"hmac_sha256","signing_field":"secret_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"hex","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100

uxc auth binding add \
  --id binance-spot-testnet-hmac \
  --host testnet.binance.vision \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-spot-testnet-hmac \
  --signer-json '{"kind":"hmac_query_v1","algorithm":"hmac_sha256","signing_field":"secret_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"hex","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100
```

## Core Workflow

1. Use fixed link commands by default:
   - `command -v binance-spot-mainnet-openapi-cli`
   - If missing, create it:
     `uxc link binance-spot-mainnet-openapi-cli https://api.binance.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-spot-openapi-skill/references/binance-spot.openapi.json`
   - `command -v binance-spot-testnet-openapi-cli`
   - If missing, create it:
     `uxc link binance-spot-testnet-openapi-cli https://testnet.binance.vision --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-spot-openapi-skill/references/binance-spot.openapi.json`

2. Discover operations with help-first flow:
   - `binance-spot-mainnet-openapi-cli -h`
   - `binance-spot-testnet-openapi-cli -h`
   - `binance-spot-testnet-openapi-cli post:/api/v3/order/test -h`
   - `binance-spot-testnet-openapi-cli get:/api/v3/account -h`

3. Execute reads first:
   - public read:
     `binance-spot-mainnet-openapi-cli get:/api/v3/ticker/price symbol=BTCUSDT`
   - signed read:
     `binance-spot-testnet-openapi-cli get:/api/v3/account omitZeroBalances=true recvWindow=5000`

4. Use `order/test` before real writes:
   - `binance-spot-testnet-openapi-cli post:/api/v3/order/test symbol=BTCUSDT side=BUY type=MARKET quoteOrderQty=100 recvWindow=5000`

## Operation Groups

### Public Market

- `get:/api/v3/ping`
- `get:/api/v3/time`
- `get:/api/v3/exchangeInfo`
- `get:/api/v3/avgPrice`
- `get:/api/v3/depth`
- `get:/api/v3/klines`
- `get:/api/v3/ticker/24hr`
- `get:/api/v3/ticker/price`
- `get:/api/v3/trades`

### Signed Reads

- `get:/api/v3/account`
- `get:/api/v3/openOrders`
- `get:/api/v3/order`
- `get:/api/v3/allOrders`
- `get:/api/v3/myTrades`
- `get:/api/v3/rateLimit/order`

### Signed Writes

- `post:/api/v3/order/test`
- `post:/api/v3/order`
- `delete:/api/v3/order`
- `delete:/api/v3/openOrders`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat `order/test` as the default validation path before any real order placement.
- Treat all mainnet write operations as high-risk and require explicit user confirmation before execution.
- Prefer testnet for all signed examples unless the user explicitly asks for mainnet.
- Before placing orders, query `exchangeInfo` for symbol filters and `ticker/price` or `depth` for current market context.
- If Binance returns `-1021`, call `get:/api/v3/time`, then retry with a fresh timestamp and, if needed, a larger `recvWindow`.
- If Binance returns `-1022`, first verify you are using the `API key` from the same Binance key record as the signing material:
  - Ed25519: displayed `API key` + matching private key PEM
  - HMAC: displayed `API key` + matching `Secret key`
- Use `--path-prefix /api/v3` on auth bindings. `uxc` now resolves OpenAPI auth against the final operation URL, so this narrower binding works for signed Spot requests and avoids over-broad host-level matching.
- `timestamp` and `signature` are injected by the signer binding; users normally provide business parameters plus optional `recvWindow`.
- `binance-spot-*-openapi-cli <operation> ...` is equivalent to `uxc <host> --schema-url <binance_spot_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/binance-spot.openapi.json`
- Binance Spot skill source material: https://github.com/binance/binance-skills-hub/tree/main/skills/binance/spot
- Official Binance Spot API docs: https://github.com/binance/binance-spot-api-docs
