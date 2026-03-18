---
name: nodit-openapi-skill
description: Operate Nodit Web3 Data API reads through UXC with a curated OpenAPI schema, API-key auth, and overlap-aware guardrails.
---

# Nodit Web3 Data API Skill

Use this skill to run Nodit Web3 Data API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://web3.nodit.io`.
- Network access to `https://raw.githubusercontent.com` when using the hosted schema URL directly.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/nodit-openapi-skill/references/nodit-web3.openapi.json`
- A Nodit API key.

## Scope

This skill covers a read-first Nodit Web3 Data API surface:

- multichain entity lookup
- native balance lookup
- account transaction history
- token contract metadata lookup
- token price lookup by contracts

This skill does **not** cover:

- transaction submission
- full JSON-RPC node compatibility
- every Nodit product surface
- broad coverage of chain-specific APIs beyond the selected v1 reads

## Authentication

Nodit uses `X-API-KEY` header auth.

Configure one API-key credential and bind it to `web3.nodit.io`:

```bash
uxc auth credential set nodit \
  --auth-type api_key \
  --api-key-header X-API-KEY \
  --secret-env NODIT_API_KEY

uxc auth binding add \
  --id nodit \
  --host web3.nodit.io \
  --scheme https \
  --credential nodit \
  --priority 100
```

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://web3.nodit.io
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v nodit-openapi-cli`
   - If missing, create it:
     `uxc link nodit-openapi-cli https://web3.nodit.io --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/nodit-openapi-skill/references/nodit-web3.openapi.json`
   - `nodit-openapi-cli -h`

2. Inspect operation schema first:
   - `nodit-openapi-cli post:/v1/multichain/lookupEntities -h`
   - `nodit-openapi-cli post:/v1/{chain}/{network}/native/getNativeBalanceByAccount -h`
   - `nodit-openapi-cli post:/v1/{chain}/{network}/token/getTokenPricesByContracts -h`

3. Prefer narrow reads before broader crawls:
   - `nodit-openapi-cli post:/v1/multichain/lookupEntities input=near`
   - `nodit-openapi-cli post:/v1/{chain}/{network}/native/getNativeBalanceByAccount chain=ethereum network=mainnet accountAddress=0xd8da6bf26964af9d7eed9e03e53415d37aa96045`
   - `nodit-openapi-cli post:/v1/{chain}/{network}/token/getTokenContractMetadataByContracts chain=ethereum network=mainnet contractAddresses:='[\"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48\"]'`

If `lookupEntities` returns `HTTP 429 TOO_MANY_REQUESTS`, treat it as a plan/tier rate-limit signal rather than an auth failure. Back off and continue with chain-specific reads when you already know the target network.

4. Execute with key/value parameters:
   - `nodit-openapi-cli post:/v1/{chain}/{network}/token/getTokenPricesByContracts chain=ethereum network=mainnet contractAddresses:='[\"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48\"]'`
   - `nodit-openapi-cli post:/v1/{chain}/{network}/blockchain/getTransactionsByAccount chain=ethereum network=mainnet accountAddress=0xd8da6bf26964af9d7eed9e03e53415d37aa96045 limit=20`

## Operation Groups

### Discovery

- `post:/v1/multichain/lookupEntities`

### Account And Token Reads

- `post:/v1/{chain}/{network}/native/getNativeBalanceByAccount`
- `post:/v1/{chain}/{network}/blockchain/getTransactionsByAccount`
- `post:/v1/{chain}/{network}/token/getTokenContractMetadataByContracts`
- `post:/v1/{chain}/{network}/token/getTokenPricesByContracts`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat this v1 skill as read-only.
- Nodit overlaps with `Chainbase`, `Alchemy`, and `Moralis` in some account and token workflows. Prefer Nodit when its multi-chain ergonomics or endpoint shape is a better fit for the task, not by default for every wallet query.
- A concrete good fit is `lookupEntities`, where Nodit can quickly normalize an input string before you decide which chain-specific follow-up read to call.
- `lookupEntities` may hit tight plan limits before other reads do. If you get `HTTP 429 TOO_MANY_REQUESTS`, back off, avoid hot-loop retries, and skip straight to chain-specific reads when the chain is already known.
- Keep `contractAddresses` lists short in v1 and stay well under the documented per-call maximums.
- For long transaction histories, start with small `limit` values and paginate deliberately.
- `nodit-openapi-cli <operation> ...` is equivalent to `uxc https://web3.nodit.io --schema-url <nodit_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/nodit-web3.openapi.json`
- Nodit introduction: https://developer.nodit.io/en/guides/overview/introduction
- Nodit entity lookup docs: https://developer.nodit.io/reference/multichain_lookupentities
- Nodit Web3 Data docs: https://developer.nodit.io/reference/gettransactionsbyaccount
