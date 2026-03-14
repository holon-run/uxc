---
name: defillama-yields-openapi-skill
description: Operate DefiLlama public yield APIs through UXC with a curated OpenAPI schema and read-first guardrails.
---

# DefiLlama Yields API Skill

Use this skill to run DefiLlama public yield API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://yields.llama.fi`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-yields-openapi-skill/references/defillama-yields.openapi.json`

## Scope

This skill covers a public read-only yield surface on `yields.llama.fi`:

- yield pool discovery
- per-pool chart history

This skill does **not** cover:

- write operations
- protocol or chain overview endpoints from `api.llama.fi`
- price endpoints from `coins.llama.fi`
- DefiLlama Pro APIs

## Authentication

This public skill does not require authentication.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v defillama-yields-openapi-cli`
   - If missing, create it:
     `uxc link defillama-yields-openapi-cli https://yields.llama.fi --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-yields-openapi-skill/references/defillama-yields.openapi.json`
   - `defillama-yields-openapi-cli -h`

2. Inspect operation schema first:
   - `defillama-yields-openapi-cli get:/pools -h`
   - `defillama-yields-openapi-cli get:/chart/{pool} -h`

3. Prefer narrow read validation before broader reads:
   - `defillama-yields-openapi-cli get:/pools`
   - `defillama-yields-openapi-cli get:/chart/{pool} pool=747c1d2a-c668-4682-b9f9-296708a3dd90`

## Operations

- `get:/pools`
- `get:/chart/{pool}`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Public DefiLlama data is split across multiple hosts; this skill intentionally wraps only `yields.llama.fi`.
- `defillama-yields-openapi-cli <operation> ...` is equivalent to `uxc https://yields.llama.fi --schema-url <defillama_yields_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/defillama-yields.openapi.json`
- DefiLlama API docs: https://defillama.com/docs/api
