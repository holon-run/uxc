---
name: kraken-openapi-skill
description: Operate Kraken public market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.
---

# Kraken REST Skill

Use this skill to run Kraken public market-data operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.kraken.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/kraken-openapi-skill/references/kraken-public.openapi.json`

## Scope

This skill covers a curated Kraken public surface for:

- server time
- asset pair metadata
- ticker reads
- OHLC candles
- order book snapshots

This skill does **not** cover:

- private account or trade endpoints in v1
- Kraken FIX
- broader non-core platform products

## Authentication

Public market endpoints in this skill do not require credentials.

Kraken private REST endpoints use provider-specific header signing and nonce handling. Keep this v1 skill public-data-only until a reusable Kraken signer flow exists in `uxc`.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v kraken-openapi-cli`
   - If missing, create it:
     `uxc link kraken-openapi-cli https://api.kraken.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/kraken-openapi-skill/references/kraken-public.openapi.json`
   - `kraken-openapi-cli -h`

2. Inspect operation help before execution:
   - `kraken-openapi-cli get:/0/public/Time -h`
   - `kraken-openapi-cli get:/0/public/Ticker -h`

3. Prefer narrow pair reads first:
   - `kraken-openapi-cli get:/0/public/Ticker pair=XBTUSD`
   - `kraken-openapi-cli get:/0/public/Depth pair=XBTUSD count=20`

## Operations

- `get:/0/public/Time`
- `get:/0/public/AssetPairs`
- `get:/0/public/Ticker`
- `get:/0/public/OHLC`
- `get:/0/public/Depth`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Kraken pair naming can differ from other venues. Check `AssetPairs` before assuming symbol strings.
- `kraken-openapi-cli <operation> ...` is equivalent to `uxc https://api.kraken.com --schema-url <kraken_public_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/kraken-public.openapi.json`
- Official Kraken API intro: https://docs.kraken.com/api/docs/guides/global-intro
