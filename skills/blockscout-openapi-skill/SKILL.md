---
name: blockscout-openapi-skill
description: "Look up addresses, tokens, transactions, and blocks on any Blockscout explorer instance through UXC with a curated OpenAPI schema. Use when the task involves blockchain explorer reads across Ethereum, Optimism, or other Blockscout-powered chains."
user-invocable: true
triggers:
  - blockscout
  - blockchain explorer
  - address lookup
  - token holders
  - block explorer
---

# Blockscout Explorer API Skill

Use this skill to run Blockscout explorer operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to a Blockscout deployment that exposes `/api/v2`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json`
- A target Blockscout instance. Examples use `https://eth.blockscout.com/api/v2`.

## Scope

Read-first explorer surface: address summary, token balances, transaction history, token metadata, token holders, transaction detail, and block detail lookups.

Does **not** cover Blockscout GraphQL, raw JSON-RPC proxying, write operations, or admin flows.

## Authentication

Public Blockscout instances need no auth. For gateway-protected instances, use standard `uxc auth` bindings for that host.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v blockscout-openapi-cli`
   - If missing, create it:
     `uxc link blockscout-openapi-cli https://eth.blockscout.com/api/v2 --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json`
   - `blockscout-openapi-cli -h`

2. Inspect operation schema first:
   - `blockscout-openapi-cli get:/addresses/{address_hash} -h`
   - `blockscout-openapi-cli get:/tokens/{address_hash} -h`
   - `blockscout-openapi-cli get:/transactions/{hash} -h`

3. Prefer narrow lookup validation before larger history reads:
   - `blockscout-openapi-cli get:/blocks/{block_number_or_hash} block_number_or_hash=latest`
   - `blockscout-openapi-cli get:/addresses/{address_hash} address_hash=0xd8da6bf26964af9d7eed9e03e53415d37aa96045`
   - `blockscout-openapi-cli get:/tokens/{address_hash} address_hash=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48`

4. Execute with key/value parameters:
   - `blockscout-openapi-cli get:/addresses/{address_hash}/transactions address_hash=0xd8da6bf26964af9d7eed9e03e53415d37aa96045`
   - `blockscout-openapi-cli get:/tokens/{address_hash}/holders address_hash=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48`

## Multi-Instance Use

To target a different Blockscout deployment, relink to another host that serves `/api/v2`:

```bash
uxc link blockscout-openapi-cli https://optimism.blockscout.com/api/v2 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json
```

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply transaction broadcast or contract write support.
- If host help fails, check the deployment path (`/api/v2`) before assuming a protocol mismatch.
- Pagination and filter options vary across deployments — inspect with operation help before building large crawls.
- `blockscout-openapi-cli <operation> ...` is equivalent to `uxc <blockscout_api_v2_host> --schema-url <blockscout_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/blockscout-v2.openapi.json`
- Blockscout API docs: https://docs.blockscout.com/devs/apis-redirect
