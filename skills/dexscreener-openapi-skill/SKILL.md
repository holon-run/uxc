---
name: dexscreener-openapi-skill
description: Operate DexScreener public market data APIs through UXC with a curated OpenAPI schema, no-auth setup, and read-first guardrails.
---

# DexScreener API Skill

Use this skill to run DexScreener public market data operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.dexscreener.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/dexscreener-openapi-skill/references/dexscreener-public.openapi.json`

## Scope

This skill covers a read-first DexScreener surface for:

- token profile discovery
- latest and top token boosts
- pair search by free-text query
- pair lookup by chain and pair address
- token market lookup by chain and token address list

This skill does **not** cover:

- write operations
- private or authenticated workflows
- every DexScreener endpoint
- trading or wallet execution

## Authentication

DexScreener public reads in this skill do not require authentication.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v dexscreener-openapi-cli`
   - If missing, create it:
     `uxc link dexscreener-openapi-cli https://api.dexscreener.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/dexscreener-openapi-skill/references/dexscreener-public.openapi.json`
   - `dexscreener-openapi-cli -h`

2. Inspect operation schema first:
   - `dexscreener-openapi-cli get:/token-profiles/latest/v1 -h`
   - `dexscreener-openapi-cli get:/latest/dex/search -h`
   - `dexscreener-openapi-cli get:/latest/dex/pairs/{chainId}/{pairId} -h`
   - `dexscreener-openapi-cli get:/tokens/v1/{chainId}/{tokenAddresses} -h`

3. Prefer narrow reads before broader scans:
   - `dexscreener-openapi-cli get:/token-profiles/latest/v1`
   - `dexscreener-openapi-cli get:/token-boosts/latest/v1`
   - `dexscreener-openapi-cli get:/latest/dex/search q=solana`

4. Execute with key/value parameters:
   - `dexscreener-openapi-cli get:/latest/dex/pairs/{chainId}/{pairId} chainId=solana pairId=GgzbfpKtozV6Hyiahkh2yNVZBZsJa4pcetCmjNtgEXiM`
   - `dexscreener-openapi-cli get:/tokens/v1/{chainId}/{tokenAddresses} chainId=solana tokenAddresses=So11111111111111111111111111111111111111112`

## Operations

- `get:/token-profiles/latest/v1`
- `get:/token-boosts/latest/v1`
- `get:/token-boosts/top/v1`
- `get:/latest/dex/search`
- `get:/latest/dex/pairs/{chainId}/{pairId}`
- `get:/tokens/v1/{chainId}/{tokenAddresses}`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply order entry, swaps, or wallet operations.
- Keep `q` focused to a token, pair, chain, or symbol rather than broad crawler-style searches.
- For `tokenAddresses`, start with a single address or a short comma-separated list before scaling up.
- DexScreener enforces endpoint-specific rate limits. Profile and boost endpoints are typically lower-throughput than pair and token lookup endpoints, so cache aggressively when polling discovery feeds.
- DexScreener data is market-observation oriented and may be noisier on long-tail tokens than curated exchange-only feeds.
- `dexscreener-openapi-cli <operation> ...` is equivalent to `uxc https://api.dexscreener.com --schema-url <dexscreener_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/dexscreener-public.openapi.json`
- DexScreener API reference: https://docs.dexscreener.com/api/reference
