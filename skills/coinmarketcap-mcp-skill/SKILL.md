---
name: coinmarketcap-mcp-skill
description: Use CoinMarketCap MCP through UXC for crypto market quotes, technical analysis, on-chain metrics, global market overview, narratives, macro events, news, and semantic search with help-first schema inspection and API-key auth.
---

# CoinMarketCap MCP Skill

Use this skill to run CoinMarketCap MCP operations through `uxc`.

Reuse the `uxc` skill for shared protocol discovery, output parsing, and generic auth/binding flows.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://mcp.coinmarketcap.com/mcp`.
- A CoinMarketCap MCP API key is available.

## Core Workflow

1. Confirm endpoint and protocol with help-first probing:
   - `uxc https://mcp.coinmarketcap.com/mcp -h`
   - expected unauthenticated behavior today: `401 Unauthorized` with `Token not found`
2. Configure credential/binding for repeatable auth:
   - `uxc auth credential set coinmarketcap-mcp --auth-type api_key --header "X-CMC-MCP-API-KEY={{secret}}" --secret-env COINMARKETCAP_MCP_API_KEY`
   - `uxc auth credential set coinmarketcap-mcp --auth-type api_key --header "X-CMC-MCP-API-KEY={{secret}}" --secret-op op://Engineering/coinmarketcap/mcp-api-key`
   - `uxc auth binding add --id coinmarketcap-mcp --host mcp.coinmarketcap.com --path-prefix /mcp --scheme https --credential coinmarketcap-mcp --priority 100`
3. Use fixed link command by default:
   - `command -v coinmarketcap-mcp-cli`
   - If missing, create it: `uxc link coinmarketcap-mcp-cli https://mcp.coinmarketcap.com/mcp`
   - `coinmarketcap-mcp-cli -h`
4. Inspect operation schema before execution:
   - `coinmarketcap-mcp-cli get_crypto_quotes_latest -h`
   - `coinmarketcap-mcp-cli get_global_metrics_latest -h`
   - `coinmarketcap-mcp-cli trending_crypto_narratives -h`
   - `coinmarketcap-mcp-cli get_crypto_latest_news -h`
5. Prefer read-only discovery first, then expand into higher-cost or plan-gated endpoints.

## Capability Map

- Market data and discovery:
  - `get_crypto_quotes_latest`
  - `search_cryptos`
  - `get_crypto_info`
- Technical analysis:
  - `get_crypto_technical_analysis`
  - `get_crypto_marketcap_technical_analysis`
- On-chain and macro metrics:
  - `get_crypto_metrics`
  - `get_global_metrics_latest`
  - `get_global_crypto_derivatives_metrics`
  - `get_upcoming_macro_events`
- Themes, news, and search:
  - `trending_crypto_narratives`
  - `get_crypto_latest_news`
  - `search_crypto_info`

Inspect `coinmarketcap-mcp-cli -h` after auth setup for the current full tool list. CoinMarketCap can revise or expand its MCP tool surface independently of this wrapper skill.

## Recommended Usage Pattern

1. Start with one focused read goal:
   - current quote and market cap for a coin
   - trend or narrative scan for a sector
   - global market and derivatives snapshot
   - latest news or semantic lookup on a project/topic
2. Run `-h` on the specific tool before the first real call.
3. Prefer id-, slug-, or keyword-scoped reads before broad market sweeps.
4. Parse the JSON envelope first, then inspect `data`.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `coinmarketcap-mcp-cli` as default command path.
- `coinmarketcap-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.coinmarketcap.com/mcp <operation> ...`.
- If unauthenticated probe or runtime call returns `401 Unauthorized` or `Token not found`:
  - confirm auth binding matches endpoint with `uxc auth binding match https://mcp.coinmarketcap.com/mcp`
  - confirm credential shape with `uxc auth credential info coinmarketcap-mcp`
  - reset credential as API-key header if needed: `uxc auth credential set coinmarketcap-mcp --auth-type api_key --header "X-CMC-MCP-API-KEY={{secret}}" --secret-env COINMARKETCAP_MCP_API_KEY`
- CoinMarketCap documents an x402 pay-per-call path, but `uxc` does not implement x402 payment handling today. This skill documents only the API-key route.
- Higher CoinMarketCap plans unlock additional datasets or request rates. If a tool returns a plan or quota error, verify the current account tier before retrying.
- CoinMarketCap MCP is read-only. Do not present it as a trading or order-routing integration.
- Use `key=value` only for simple scalar inputs.
- Prefer positional JSON when an operation accepts nested objects, arrays, or optional filters that may evolve.
- Do not assume tool argument names from memory; inspect `<operation> -h` first because CoinMarketCap may revise MCP schemas independently of this skill.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
