---
name: blockscout-openapi-skill
description: Operate Blockscout explorer reads through UXC with a curated OpenAPI schema, instance-specific host selection, and read-first guardrails.
---

# Blockscout Explorer API Skill

Use this skill to run Blockscout explorer operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to a Blockscout deployment that exposes `/api/v2`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json`
- A target Blockscout instance. Examples in this skill use `https://eth.blockscout.com/api/v2`.

## Scope

This skill covers a read-first explorer surface:

- address summary lookup
- address token balances
- address transaction history
- token metadata
- token holder reads
- transaction detail lookup
- block detail lookup

This skill does **not** cover:

- Blockscout GraphQL
- raw JSON-RPC proxying
- write operations or admin/configuration flows
- custom authenticated gateways beyond what the caller explicitly binds

## Authentication

Public Blockscout instances usually allow explorer reads without auth.

If you are targeting a self-hosted or gateway-protected instance, configure auth separately with standard `uxc auth` bindings for that host. This skill does not assume any default credential.

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

## Operation Groups

### Address Reads

- `get:/addresses/{address_hash}`
- `get:/addresses/{address_hash}/token-balances`
- `get:/addresses/{address_hash}/transactions`

### Token, Transaction, And Block Reads

- `get:/tokens/{address_hash}`
- `get:/tokens/{address_hash}/holders`
- `get:/transactions/{hash}`
- `get:/blocks/{block_number_or_hash}`

## Multi-Instance Use

To target a different Blockscout deployment, keep the same schema and relink the command to another host that serves `/api/v2`:

```bash
uxc link blockscout-openapi-cli https://optimism.blockscout.com/api/v2 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json
```

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply transaction broadcast or contract write support.
- This schema is designed for Blockscout deployments that expose the explorer REST surface at `/api/v2`. If host help fails, check the deployment path before assuming a protocol mismatch.
- Pagination and filter options vary across deployments. Start with host help and operation help on the target instance before building large crawls.
- `blockscout-openapi-cli <operation> ...` is equivalent to `uxc <blockscout_api_v2_host> --schema-url <blockscout_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/blockscout-v2.openapi.json`
- Blockscout API docs: https://docs.blockscout.com/devs/apis-redirect
