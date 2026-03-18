---
name: mempool-space-openapi-skill
description: Operate mempool.space public Bitcoin and Lightning explorer APIs through UXC with a curated OpenAPI schema, no-auth setup, and read-first guardrails.
---

# mempool.space API Skill

Use this skill to run mempool.space public Bitcoin and Lightning explorer operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://mempool.space/api`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/mempool-space-openapi-skill/references/mempool-space-public.openapi.json`

## Scope

This skill covers a read-first mempool.space surface for:

- Bitcoin fee estimation and mempool state reads
- block tip height checks
- address and transaction status reads
- Lightning network search, statistics, node rankings, node detail, and channel reads

This skill does **not** cover:

- transaction broadcast or package submission
- websocket subscriptions
- internal or admin routes
- every Esplora-compatible endpoint exposed by mempool.space

## Authentication

mempool.space public reads in this skill do not require authentication.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v mempool-space-openapi-cli`
   - If missing, create it:
     `uxc link mempool-space-openapi-cli https://mempool.space/api --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/mempool-space-openapi-skill/references/mempool-space-public.openapi.json`
   - `mempool-space-openapi-cli -h`

2. Inspect operation schema first:
   - `mempool-space-openapi-cli get:/v1/fees/recommended -h`
   - `mempool-space-openapi-cli get:/mempool -h`
   - `mempool-space-openapi-cli get:/address/{address} -h`
   - `mempool-space-openapi-cli get:/v1/lightning/search -h`
   - `mempool-space-openapi-cli get:/v1/lightning/channels/{short_id} -h`

3. Prefer narrow reads before broader scans:
   - `mempool-space-openapi-cli get:/v1/fees/recommended`
   - `mempool-space-openapi-cli get:/blocks/tip/height`
   - `mempool-space-openapi-cli get:/v1/lightning/statistics/latest`

4. Execute with key/value parameters:
   - `mempool-space-openapi-cli get:/address/{address} address=bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh`
   - `mempool-space-openapi-cli get:/tx/{txid}/status txid=4d5d7f2d5dc69aa68a51887db07dd6d906f31f9141320f9f0b4bab76d735a47f`
   - `mempool-space-openapi-cli get:/v1/lightning/search searchText=bfx`
   - `mempool-space-openapi-cli get:/v1/lightning/channels public_key=033d8656219478701227199cbd6f670335c8d408a92ae88b962c49d4dc0e83e025 status=active`
   - `mempool-space-openapi-cli get:/v1/lightning/channels/{short_id} short_id=835866331763769345`

## Operations

- `get:/v1/fees/recommended`
- `get:/mempool`
- `get:/blocks/tip/height`
- `get:/address/{address}`
- `get:/tx/{txid}/status`
- `get:/v1/lightning/statistics/latest`
- `get:/v1/lightning/search`
- `get:/v1/lightning/nodes/rankings`
- `get:/v1/lightning/nodes/{public_key}`
- `get:/v1/lightning/channels`
- `get:/v1/lightning/channels/{short_id}`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not use `tx/push`, package submission, or internal routes.
- Prefer the curated fee, mempool, and Lightning reads here before dropping to the much larger generic Esplora surface.
- `mempool.space` is a public explorer service, so mempool state and Lightning rankings can move quickly. Re-query instead of assuming cached values remain current.
- For `get:/v1/lightning/channels/{short_id}`, mempool.space currently accepts the channel `id` string even though the route label says `short_id`; prefer values returned by search or node channel listing.
- `mempool-space-openapi-cli <operation> ...` is equivalent to `uxc https://mempool.space/api --schema-url <mempool_space_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/mempool-space-public.openapi.json`
- Official mempool repository: https://github.com/mempool/mempool
