---
name: goldrush-mcp-skill
description: Use GoldRush MCP through UXC for multichain wallet balances, transfers, portfolio history, NFT ownership, token approvals, prices, and chain metadata via stdio MCP with injected API-key auth.
---

# GoldRush MCP Skill

Use this skill to run GoldRush MCP operations through `uxc` using a fixed stdio endpoint.

Reuse the `uxc` skill for generic protocol discovery, output parsing, and auth/error handling rules.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- `npx` is available in `PATH` (Node.js installed).
- Network access for first-time `@covalenthq/goldrush-mcp-server` package fetch.
- A GoldRush API key is available.

## Core Workflow

Endpoint candidate inputs before finalizing:
- Raw package form from docs: `npx @covalenthq/goldrush-mcp-server@latest`
- Reliable non-interactive form: `npx -y @covalenthq/goldrush-mcp-server@latest`
- This skill defaults to:
  - `npx -y @covalenthq/goldrush-mcp-server@latest`

1. Verify protocol/path from official source and probe:
   - Official source: `https://goldrush.dev/docs/goldrush-mcp-server`
   - probe candidate endpoint with:
     - `uxc --inject-env GOLDRUSH_API_KEY=$GOLDRUSH_API_KEY "npx -y @covalenthq/goldrush-mcp-server@latest" -h`
   - cold start can take longer on first run because `npx` may need to download the package
2. Configure credential for repeatable auth:
   - `uxc auth credential set goldrush-mcp --auth-type bearer --secret-env GOLDRUSH_API_KEY`
   - `uxc auth credential set goldrush-mcp --auth-type bearer --secret-op op://Engineering/goldrush/api-key`
3. Use fixed link command by default:
   - `command -v goldrush-mcp-cli`
   - If missing, create it:
     - `uxc link goldrush-mcp-cli "npx -y @covalenthq/goldrush-mcp-server@latest" --credential goldrush-mcp --inject-env GOLDRUSH_API_KEY={{secret}}`
   - `goldrush-mcp-cli -h`
4. Inspect operation schema before execution:
   - `goldrush-mcp-cli getAllChains -h`
   - `goldrush-mcp-cli multichain_balances -h`
   - `goldrush-mcp-cli transactions_for_address -h`
   - `goldrush-mcp-cli historical_portfolio_value -h`
5. Prefer read-only discovery first, then expand into broader wallet-history or NFT scans.

## Capability Map

- Cross-chain overview:
  - `getAllChains`
  - `multichain_address_activity`
  - `multichain_balances`
  - `multichain_transactions`
- Wallet balances and portfolio:
  - `token_balances`
  - `historical_token_balances`
  - `native_token_balance`
  - `historical_portfolio_value`
  - `historical_token_prices`
- Transfers and transactions:
  - `erc20_token_transfers`
  - `transaction`
  - `transaction_summary`
  - `transactions_for_address`
  - `transactions_for_block`
- NFT and security:
  - `nft_for_address`
  - `nft_check_ownership`
  - `token_approvals`
- Utility:
  - `gas_prices`
  - `log_events_by_address`
  - `log_events_by_topic`
  - `block`
  - `block_heights`

GoldRush also exposes MCP resources such as `config://supported-chains`, `config://quote-currencies`, and `status://all-chains`. Inspect the live server after auth setup for the current full tool and resource list.

## Recommended Usage Pattern

1. Start with one focused read goal:
   - multichain balances for a wallet
   - recent transfers for an address
   - historical portfolio value over time
   - NFT ownership or token approval review
2. Run `-h` on the specific tool before the first real call.
3. Prefer a single wallet and chain first before running wide history scans.
4. Parse the JSON envelope first, then inspect `data`.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `goldrush-mcp-cli` as default command path.
- `goldrush-mcp-cli <operation> ...` is equivalent to `uxc --auth goldrush-mcp --inject-env GOLDRUSH_API_KEY={{secret}} "npx -y @covalenthq/goldrush-mcp-server@latest" <operation> ...` when the link is created as documented above.
- GoldRush uses a stdio MCP server started through `npx`, not a hosted HTTPS MCP endpoint. Expect slower cold starts on the first run.
- If help or the first call times out during initialization:
  - rerun the same command after the package download finishes
  - confirm `npx` is available in `PATH`
  - confirm the key is being injected as `GOLDRUSH_API_KEY`
- The skill docs use bearer credential storage only as a secret container for `--inject-env`; GoldRush auth actually happens through the child environment variable, not an HTTP bearer header.
- Prefer wallet-scoped reads before wide transaction or log scans because some tools can produce large result sets.
- Do not assume tool argument names from memory; inspect `<operation> -h` first because GoldRush may revise MCP schemas independently of this skill.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
