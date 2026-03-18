---
name: blocknative-openapi-skill
description: Operate Blocknative gas intelligence APIs through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.
---

# Blocknative Gas Platform Skill

Use this skill to run Blocknative gas intelligence operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.blocknative.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/blocknative-openapi-skill/references/blocknative-gas.openapi.json`
- A Blocknative API key for the full v1 surface.

## Scope

This skill covers a read-first Blocknative gas intelligence surface:

- supported chain discovery
- gas price confidence estimates
- base fee and blob fee prediction
- pending gas distribution analysis

This skill does **not** cover:

- write operations
- transaction submission
- mempool event streaming
- broader Blocknative product areas outside the selected gas platform endpoints

## Authentication

Blocknative uses `Authorization` header auth. Some discovery and gas reads can work without a key, but this skill standardizes on authenticated requests because `basefee-estimates` and `distribution` require a valid API key.

Configure one API-key credential and bind it to `api.blocknative.com`:

```bash
uxc auth credential set blocknative \
  --auth-type api_key \
  --api-key-header Authorization \
  --secret-env BLOCKNATIVE_API_KEY

uxc auth binding add \
  --id blocknative \
  --host api.blocknative.com \
  --scheme https \
  --credential blocknative \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://api.blocknative.com
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v blocknative-openapi-cli`
   - If missing, create it:
     `uxc link blocknative-openapi-cli https://api.blocknative.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blocknative-openapi-skill/references/blocknative-gas.openapi.json`
   - `blocknative-openapi-cli -h`

2. Inspect operation schema first:
   - `blocknative-openapi-cli get:/chains -h`
   - `blocknative-openapi-cli get:/gasprices/blockprices -h`
   - `blocknative-openapi-cli get:/gasprices/basefee-estimates -h`

3. Prefer narrow validation before broader polling:
   - `blocknative-openapi-cli get:/chains`
   - `blocknative-openapi-cli get:/gasprices/blockprices chainid=1`
   - `blocknative-openapi-cli get:/gasprices/basefee-estimates`

4. Execute with key/value parameters:
   - `blocknative-openapi-cli get:/gasprices/blockprices chainid=1 confidenceLevels=70,90,99`
   - `blocknative-openapi-cli get:/gasprices/blockprices system=story network=mainnet`
   - `blocknative-openapi-cli get:/gasprices/distribution chainid=1`

## Operation Groups

### Discovery

- `get:/chains`

### Gas Intelligence

- `get:/gasprices/blockprices`
- `get:/gasprices/basefee-estimates`
- `get:/gasprices/distribution`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply transaction sending or execution support.
- `blockprices` can be polled without auth on some plans, but `basefee-estimates` and `distribution` require a valid key. Standardize on auth so mixed workflows do not fail mid-run.
- These endpoints update at most once per second on paid plans and more slowly on free plans. For automation, start around one poll every 5 to 10 seconds and only tighten that interval when you specifically need fresher paid-plan data.
- `distribution` is Ethereum-mainnet focused in the current docs. Do not assume multi-chain coverage there just because `blockprices` supports many chains.
- Keep `confidenceLevels` narrow and explicit when you do not need the full default ladder.
- `blocknative-openapi-cli <operation> ...` is equivalent to `uxc https://api.blocknative.com --schema-url <blocknative_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/blocknative-gas.openapi.json`
- Blocknative Gas Price API docs: https://docs.blocknative.com/gas-prediction/gas-platform
- Blocknative Base Fee API docs: https://docs.blocknative.com/gas-prediction/prediction-api-base-fee-and-blob-fee
- Blocknative Gas Distribution API docs: https://docs.blocknative.com/gas-prediction/gas-distribution-api
