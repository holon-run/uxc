---
name: defillama-prices-openapi-skill
description: Operate DefiLlama public price APIs through UXC with a curated OpenAPI schema and read-first guardrails.
---

# DefiLlama Prices API Skill

Use this skill to run DefiLlama public price API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://coins.llama.fi`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-prices-openapi-skill/references/defillama-prices.openapi.json`

## Scope

This skill covers a small public read-only price surface on `coins.llama.fi`:

- current price lookups for one or more assets

This skill does **not** cover:

- write operations
- protocol or chain overview endpoints from `api.llama.fi`
- yield endpoints from `yields.llama.fi`
- DefiLlama Pro APIs

## Authentication

This public skill does not require authentication.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v defillama-prices-openapi-cli`
   - If missing, create it:
     `uxc link defillama-prices-openapi-cli https://coins.llama.fi --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-prices-openapi-skill/references/defillama-prices.openapi.json`
   - `defillama-prices-openapi-cli -h`

2. Inspect operation schema first:
   - `defillama-prices-openapi-cli get:/prices/current/{coins} -h`

3. Prefer narrow read validation before broader reads:
   - `defillama-prices-openapi-cli get:/prices/current/{coins} coins=coingecko:bitcoin searchWidth=4h`

## Operations

- `get:/prices/current/{coins}`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- This skill only wraps `coins.llama.fi`; use the separate public analytics or yields skills for other DefiLlama hosts.
- `defillama-prices-openapi-cli <operation> ...` is equivalent to `uxc https://coins.llama.fi --schema-url <defillama_prices_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/defillama-prices.openapi.json`
- DefiLlama API docs: https://defillama.com/docs/api
