---
name: helius-openapi-skill
description: Operate Helius Wallet API reads through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.
---

# Helius Wallet API Skill

Use this skill to run Helius Wallet API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.helius.xyz`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/helius-openapi-skill/references/helius-wallet.openapi.json`
- A Helius API key.

## Scope

This skill covers a read-first Helius Wallet API surface:

- wallet identity lookup
- batch identity lookup
- wallet balances
- wallet history
- wallet transfers
- wallet funding source lookup

This skill does **not** cover:

- Helius RPC, DAS, or WebSocket surfaces
- transaction submission
- webhook setup
- the broader Helius platform beyond the selected wallet intelligence endpoints

## Authentication

Helius accepts API keys by query parameter or header. This skill standardizes on `X-Api-Key` header auth.

Configure one API-key credential and bind it to `api.helius.xyz`:

```bash
uxc auth credential set helius \
  --auth-type api_key \
  --api-key-header X-Api-Key \
  --secret-env HELIUS_API_KEY

uxc auth binding add \
  --id helius \
  --host api.helius.xyz \
  --scheme https \
  --credential helius \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://api.helius.xyz
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v helius-openapi-cli`
   - If missing, create it:
     ```bash
     uxc link helius-openapi-cli https://api.helius.xyz \
       --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/helius-openapi-skill/references/helius-wallet.openapi.json
     ```
   - `helius-openapi-cli -h`

2. Inspect operation schema first:
   - `helius-openapi-cli get:/v1/wallet/{wallet}/identity -h`
   - `helius-openapi-cli post:/v1/wallet/batch-identity -h`
   - `helius-openapi-cli get:/v1/wallet/{wallet}/balances -h`

3. Prefer narrow validation before broader reads:
   - `helius-openapi-cli get:/v1/wallet/{wallet}/identity wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664`
   - `helius-openapi-cli get:/v1/wallet/{wallet}/funded-by wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664`
   - `helius-openapi-cli get:/v1/wallet/{wallet}/balances wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664 page=1 limit=20 showNative=true`

4. Execute with key/value parameters:
   - `helius-openapi-cli post:/v1/wallet/batch-identity addresses:='["HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664"]'`
   - `helius-openapi-cli get:/v1/wallet/{wallet}/history wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664 limit=20`
   - `helius-openapi-cli get:/v1/wallet/{wallet}/transfers wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664 limit=20`
   - For `addresses:='[...]'`, keep shell quoting in mind. If your shell mangles that form, pass the JSON array as a bare positional payload instead.

## Operation Groups

### Wallet Identity

- `get:/v1/wallet/{wallet}/identity`
- `post:/v1/wallet/batch-identity`
- `get:/v1/wallet/{wallet}/funded-by`

### Wallet Activity

- `get:/v1/wallet/{wallet}/balances`
- `get:/v1/wallet/{wallet}/history`
- `get:/v1/wallet/{wallet}/transfers`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply RPC write methods, transaction sending, or webhooks.
- Helius labels the Wallet API as beta. Expect response shape drift and keep parsers focused on stable fields.
- Start with small `limit` values before paginating large histories or transfer crawls.
- Identity and funded-by lookups can return 404-style misses for unknown wallets; treat that as a data absence case before assuming auth failure.
- `helius-openapi-cli <operation> ...` is equivalent to `uxc https://api.helius.xyz --schema-url <helius_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/helius-wallet.openapi.json`
- Helius authentication docs: https://www.helius.dev/docs/api-reference/authentication
- Helius Wallet API docs: https://www.helius.dev/docs/api-reference/wallet-api
