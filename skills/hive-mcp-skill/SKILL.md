---
name: hive-mcp-skill
description: Use Hive Intelligence MCP through UXC for broad crypto market, onchain, portfolio, and risk workflows with help-first discovery and convenience-layer guardrails.
---

# Hive Intelligence MCP Skill

Use this skill to run Hive Intelligence MCP operations through `uxc`.

Reuse the `uxc` skill for shared protocol discovery, output parsing, and generic auth handling.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://hiveintelligence.xyz/mcp`.
- Access to the official Hive MCP documentation:
  - `https://hiveintelligence.xyz/crypto-market-data-mcp`

## Scope

This skill covers the official remote Hive MCP surface for:

- category-level endpoint discovery
- crypto market and onchain data discovery
- wallet and portfolio lookup workflows
- security and risk endpoint discovery
- schema inspection for underlying API endpoints
- endpoint invocation through Hive's MCP tool layer

This skill does **not** cover:

- direct replacement of first-party provider integrations
- assuming one Hive endpoint has the same data quality as the underlying provider skill
- write operations without separate explicit review

## Positioning

Hive is an official remote MCP convenience layer.

Use it when a user wants one broad crypto MCP entrypoint quickly.

Do not treat Hive as a substitute for direct provider integrations like `DexScreener`, `Helius`, `Blocknative`, or other first-party skills where long-term provider coverage quality matters.

## Endpoint

Use the official Hive MCP endpoint:

- `https://hiveintelligence.xyz/mcp`

## Authentication

The public help-first flow for this skill works without a local auth bootstrap.

If Hive later introduces auth for higher-tier access, verify the runtime error and bind credentials before extending this skill.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v hive-mcp-cli`
   - If missing, create it:
     `uxc link hive-mcp-cli https://hiveintelligence.xyz/mcp`
   - `hive-mcp-cli -h`

2. Discover categories first:
   - `hive-mcp-cli get_market_and_price_endpoints`
   - `hive-mcp-cli get_onchain_dex_pool_endpoints`
   - `hive-mcp-cli get_portfolio_wallet_endpoints`
   - `hive-mcp-cli get_security_risk_endpoints`

3. Inspect endpoint schema before invoking:
   - `hive-mcp-cli get_api_endpoint_schema endpoint=get_token_price`
   - `hive-mcp-cli get_api_endpoint_schema endpoint=get_wallet_portfolio`

4. Invoke only after confirming the schema:
   - `hive-mcp-cli invoke_api_endpoint endpoint_name=get_token_price args:='{"symbol":"BTC"}'`
   - `hive-mcp-cli invoke_api_endpoint endpoint_name=get_wallet_portfolio args:='{"address":"0xd8da6bf26964af9d7eed9e03e53415d37aa96045"}'`

## Recommended Discovery Operations

- `get_market_and_price_endpoints`
- `get_onchain_dex_pool_endpoints`
- `get_portfolio_wallet_endpoints`
- `get_token_contract_endpoints`
- `get_security_risk_endpoints`
- `get_network_infrastructure_endpoints`

## Guardrails

- Keep automation on the JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Treat Hive as a convenience aggregation layer, not as the canonical source when a direct provider skill already exists.
- Always inspect a target endpoint schema with `get_api_endpoint_schema` before calling `invoke_api_endpoint`.
- Keep `invoke_api_endpoint` payloads narrow and explicit; do not guess large nested arg objects.
- If a workflow becomes repetitive on one underlying provider surface, prefer a dedicated first-party skill over routing everything through Hive indefinitely.
- `hive-mcp-cli <operation> ...` is equivalent to `uxc https://hiveintelligence.xyz/mcp <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Hive MCP docs: https://hiveintelligence.xyz/crypto-market-data-mcp
