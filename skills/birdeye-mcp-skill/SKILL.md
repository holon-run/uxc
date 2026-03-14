---
name: birdeye-mcp-skill
description: Use Birdeye MCP through UXC for token market data, trending and discovery workflows, price monitoring, and DEX-related reads with help-first live tool discovery and API-key auth.
---

# Birdeye MCP Skill

Use this skill to run Birdeye MCP operations through `uxc`.

Reuse the `uxc` skill for shared protocol discovery, output parsing, and generic auth/binding flows.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://mcp.birdeye.so/mcp`.
- A Birdeye API key is available.

## Core Workflow

1. Confirm endpoint and protocol with help-first probing:
   - `uxc https://mcp.birdeye.so/mcp -h`
   - expected unauthenticated behavior today: `401 Unauthorized`
2. Configure credential/binding for repeatable auth:
   - `uxc auth credential set birdeye-mcp --auth-type api_key --header "X-API-KEY={{secret}}" --secret-env BIRDEYE_API_KEY`
   - `uxc auth credential set birdeye-mcp --auth-type api_key --header "X-API-KEY={{secret}}" --secret-op op://Engineering/birdeye/api-key`
   - `uxc auth binding add --id birdeye-mcp --host mcp.birdeye.so --path-prefix /mcp --scheme https --credential birdeye-mcp --priority 100`
3. Use fixed link command by default:
   - `command -v birdeye-mcp-cli`
   - If missing, create it: `uxc link birdeye-mcp-cli https://mcp.birdeye.so/mcp`
   - `birdeye-mcp-cli -h`
4. Inspect the live tool list before execution:
   - `birdeye-mcp-cli -h`
   - then inspect the specific operation you need with `<operation> -h`
5. Prefer read-only discovery first, then broader monitoring queries.

## Capability Focus

Birdeye MCP is a fit for these read-heavy workflows:

- token market data
- trending or discovery views
- price monitoring
- DEX liquidity or trading context
- token or pair lookup

Inspect `birdeye-mcp-cli -h` after auth setup for the current tool list. Birdeye MCP is still in beta and the live tool surface may evolve independently of this wrapper skill.

## Recommended Usage Pattern

1. Start from one focused read goal:
   - current market data for a token
   - trending token or narrative discovery
   - DEX liquidity or price movement checks
2. Run host help first, then operation help.
3. Prefer narrow symbol, address, or chain scopes before broad scans.
4. Parse the JSON envelope first, then inspect `data`.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `birdeye-mcp-cli` as default command path.
- `birdeye-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.birdeye.so/mcp <operation> ...`.
- If unauthenticated probe or runtime call returns `401 Unauthorized`:
  - confirm auth binding matches endpoint with `uxc auth binding match https://mcp.birdeye.so/mcp`
  - confirm credential shape with `uxc auth credential info birdeye-mcp`
  - reset credential as API-key header if needed: `uxc auth credential set birdeye-mcp --auth-type api_key --header "X-API-KEY={{secret}}" --secret-env BIRDEYE_API_KEY`
- Birdeye MCP is beta. Do not hardcode assumptions about the live tool list or argument names; inspect `-h` first.
- Keep initial queries small and specific because market/discovery tools can return wide datasets.
- Treat this skill as read-only market and discovery access, not a trading or execution surface.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
