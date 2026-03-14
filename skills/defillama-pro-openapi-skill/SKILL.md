---
name: defillama-pro-openapi-skill
description: Operate DefiLlama Pro analytics APIs through UXC with a curated OpenAPI schema, path-templated API-key auth, and read-first guardrails.
---

# DefiLlama Pro API Skill

Use this skill to run DefiLlama Pro API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://pro-api.llama.fi`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-pro-openapi-skill/references/defillama-pro.openapi.json`
- A DefiLlama Pro API key.

## Scope

This skill covers a read-first analytics surface:

- protocol TVL list and per-protocol detail
- chain overview reads
- current token price lookups
- yield pool discovery
- yield chart history
- stablecoin dominance reads

This skill does **not** cover:

- write operations or account management
- the public unauthenticated host variants
- the full DefiLlama Pro endpoint surface

## Authentication

DefiLlama Pro places the API key in the request path, between the host and the endpoint path.

Configure one API-key credential with a request path prefix template:

```bash
uxc auth credential set defillama-pro \
  --auth-type api_key \
  --secret-env DEFILLAMA_PRO_API_KEY \
  --path-prefix-template "/{{secret}}"

uxc auth binding add \
  --id defillama-pro \
  --host pro-api.llama.fi \
  --scheme https \
  --credential defillama-pro \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://pro-api.llama.fi
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v defillama-pro-openapi-cli`
   - If missing, create it:
     `uxc link defillama-pro-openapi-cli https://pro-api.llama.fi --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-pro-openapi-skill/references/defillama-pro.openapi.json`
   - `defillama-pro-openapi-cli -h`

2. Inspect operation schema first:
   - `defillama-pro-openapi-cli get:/api/protocols -h`
   - `defillama-pro-openapi-cli get:/coins/prices/current/{coins} -h`
   - `defillama-pro-openapi-cli get:/yields/chart/{pool} -h`

3. Prefer narrow read validation before broader reads:
   - `defillama-pro-openapi-cli get:/api/v2/chains`
   - `defillama-pro-openapi-cli get:/api/protocol/{protocol} protocol=aave`
   - `defillama-pro-openapi-cli get:/yields/pools`

4. Execute with key/value parameters:
   - `defillama-pro-openapi-cli get:/api/protocol/{protocol} protocol=aave`
   - `defillama-pro-openapi-cli get:/coins/prices/current/{coins} coins=ethereum:0x0000000000000000000000000000000000000000 searchWidth=4h`
   - `defillama-pro-openapi-cli get:/stablecoins/stablecoindominance/{chain} chain=ethereum`

## Operation Groups

### Protocol And Chain Analytics

- `get:/api/protocols`
- `get:/api/protocol/{protocol}`
- `get:/api/v2/chains`

### Prices, Yields, And Stablecoins

- `get:/coins/prices/current/{coins}`
- `get:/yields/pools`
- `get:/yields/chart/{pool}`
- `get:/stablecoins/stablecoindominance/{chain}`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only. Do not imply wallet, trading, or admin support.
- This skill assumes the Pro host and key-in-path auth model. Do not bind the same credential to a different path shape without checking the upstream docs first.
- API keys are sensitive because they appear in the request path. Use `--secret-env` or `--secret-op`, not shell history literals, when possible.
- Avoid sharing raw daemon logs when troubleshooting this integration. The key is part of the request path, so if you inspect `~/.uxc/daemon/daemon.log`, sanitize, rotate, or delete the log after debugging and avoid verbose logging unless necessary.
- `defillama-pro-openapi-cli <operation> ...` is equivalent to `uxc https://pro-api.llama.fi --schema-url <defillama_pro_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/defillama-pro.openapi.json`
- DefiLlama API docs: https://defillama.com/docs/api
- DefiLlama Pro docs: https://defillama.com/pro-api/docs
