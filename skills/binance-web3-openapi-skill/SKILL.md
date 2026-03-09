---
name: binance-web3-openapi-skill
description: Operate Binance Web3 public market and research APIs through UXC with a curated OpenAPI schema. Use when tasks need token search, token metadata/market snapshots, address holdings, rankings, token audit, or smart money signals on Binance Web3.
---

# Binance Web3 API Skill

Use this skill to run Binance Web3 public read operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://web3.binance.com`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-web3-openapi-skill/references/binance-web3.openapi.json`

## Scope

This skill covers the public `web3.binance.com` endpoints for:

- token search
- token metadata
- token market snapshots
- address holdings
- token security audit
- social hype leaderboard
- unified token ranks
- meme rush ranks
- smart money signals

This skill does **not** cover:

- Binance Spot / account trading APIs
- Binance Square posting
- K-line candles hosted on `https://dquery.sintral.io`

## Authentication

Most operations are public and do not require API credentials.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v binance-web3-openapi-cli`
   - If missing, create it:
     `uxc link binance-web3-openapi-cli https://web3.binance.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-web3-openapi-skill/references/binance-web3.openapi.json`
   - `binance-web3-openapi-cli -h`

2. Inspect operation schema first:
   - `binance-web3-openapi-cli get:/bapi/defi/v5/public/wallet-direct/buw/wallet/market/token/search -h`
   - `binance-web3-openapi-cli post:/bapi/defi/v1/public/wallet-direct/security/token/audit -h`

3. Execute operation:
   - key/value:
     `binance-web3-openapi-cli get:/bapi/defi/v5/public/wallet-direct/buw/wallet/market/token/search keyword=bnb chainIds=56 orderBy=volume24h`
   - positional JSON:
     `binance-web3-openapi-cli post:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/unified/rank/list '{"rankType":10,"chainId":"56","period":50,"page":1,"size":20}'`

## Operation Groups

### Token Discovery And Market Snapshot

- Search tokens:
  - `get:/bapi/defi/v5/public/wallet-direct/buw/wallet/market/token/search`
- Fetch token metadata:
  - `get:/bapi/defi/v1/public/wallet-direct/buw/wallet/dex/market/token/meta/info`
- Fetch token market data:
  - `get:/bapi/defi/v4/public/wallet-direct/buw/wallet/market/token/dynamic/info`

### Rankings And Signals

- Social hype leaderboard:
  - `get:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/social/hype/rank/leaderboard`
- Unified token rank:
  - `post:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/unified/rank/list`
- Meme rush rank:
  - `post:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/rank/list`
- Smart money signals:
  - `post:/bapi/defi/v1/public/wallet-direct/buw/wallet/web/signal/smart-money`

### Research

- Address holdings:
  - `get:/bapi/defi/v3/public/wallet-direct/buw/wallet/address/pnl/active-position-list`
- Token audit:
  - `post:/bapi/defi/v1/public/wallet-direct/security/token/audit`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable envelope fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Binance Web3 responses usually wrap payloads as `code`, `message`, `success`, `data`; treat `code == "000000"` as success.
- `audit` requires a UUID v4 `requestId`; generate one for every request instead of reusing old IDs.
- Address holdings requires operation-level headers `clienttype=web` and `clientversion=1.2.0`; keep them scoped to that operation instead of injecting them host-wide.
- For non-string objects, prefer positional JSON instead of flattening complex filters into many `key=value` args.
- `binance-web3-openapi-cli <operation> ...` is equivalent to `uxc https://web3.binance.com --schema-url <binance_web3_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/binance-web3.openapi.json`
- Binance skills hub source material: https://github.com/binance/binance-skills-hub/tree/main/skills/binance-web3
