---
name: defillama-openapi-skill
description: Operate DefiLlama public analytics APIs through UXC with a curated OpenAPI schema and read-first guardrails.
---

# DefiLlama Public API Skill

Use this skill to run DefiLlama public API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.llama.fi`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-openapi-skill/references/defillama-public.openapi.json`

## Scope

This skill covers a public read-only analytics surface on `api.llama.fi`:

- protocol TVL list
- per-protocol detail
- chain overview reads

This skill does **not** cover:

- write operations or account management
- DefiLlama Pro key-in-path auth
- split-host public services such as `coins.llama.fi` and `yields.llama.fi`
- the full DefiLlama public API surface

## Authentication

This public skill does not require authentication.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v defillama-openapi-cli`
   - If missing, create it:
     `uxc link defillama-openapi-cli https://api.llama.fi --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-openapi-skill/references/defillama-public.openapi.json`
   - `defillama-openapi-cli -h`

2. Inspect operation schema first:
   - `defillama-openapi-cli get:/protocols -h`
   - `defillama-openapi-cli get:/protocol/{protocol} -h`
   - `defillama-openapi-cli get:/v2/chains -h`

3. Prefer narrow read validation before broader reads:
   - `defillama-openapi-cli get:/v2/chains`
   - `defillama-openapi-cli get:/protocols`
   - `defillama-openapi-cli get:/protocol/{protocol} protocol=aave`

4. Execute with key/value parameters:
   - `defillama-openapi-cli get:/protocol/{protocol} protocol=aave`
   - `defillama-openapi-cli get:/v2/chains`

## Operations

- `get:/protocols`
- `get:/protocol/{protocol}`
- `get:/v2/chains`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply wallet, trading, or admin support.
- Public DefiLlama data is split across multiple hosts. This skill intentionally stays on `api.llama.fi`; use the separate Pro skill when you need the unified Pro host.
- `defillama-openapi-cli <operation> ...` is equivalent to `uxc https://api.llama.fi --schema-url <defillama_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/defillama-public.openapi.json`
- DefiLlama API docs: https://defillama.com/docs/api
