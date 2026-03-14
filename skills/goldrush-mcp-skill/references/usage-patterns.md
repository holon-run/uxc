# Usage Patterns

This skill defaults to fixed link command `goldrush-mcp-cli`.

## Setup

```bash
uxc auth credential set goldrush-mcp --auth-type bearer --secret-env GOLDRUSH_API_KEY
command -v goldrush-mcp-cli
uxc link goldrush-mcp-cli "npx -y @covalenthq/goldrush-mcp-server@latest" --credential goldrush-mcp --inject-env GOLDRUSH_API_KEY={{secret}}
goldrush-mcp-cli -h
```

Optional secret manager source:

```bash
uxc auth credential set goldrush-mcp --auth-type bearer --secret-op op://Engineering/goldrush/api-key
```

If the first `-h` run times out during initialization, rerun it once `npx` finishes downloading `@covalenthq/goldrush-mcp-server`.

## Help-First Discovery

```bash
goldrush-mcp-cli getAllChains -h
goldrush-mcp-cli multichain_balances -h
goldrush-mcp-cli transactions_for_address -h
goldrush-mcp-cli historical_portfolio_value -h
```

## Multichain Wallet Overview

Inspect supported chains:

```bash
goldrush-mcp-cli getAllChains
```

Inspect cross-chain balances:

```bash
goldrush-mcp-cli multichain_balances -h
```

Inspect wallet activity:

```bash
goldrush-mcp-cli multichain_address_activity -h
goldrush-mcp-cli multichain_transactions -h
```

## Portfolio And Transfers

Inspect historical portfolio value:

```bash
goldrush-mcp-cli historical_portfolio_value -h
```

Inspect ERC-20 transfers and wallet history:

```bash
goldrush-mcp-cli erc20_token_transfers -h
goldrush-mcp-cli transactions_for_address -h
goldrush-mcp-cli transaction_summary -h
```

## NFT, Security, And Utility

Inspect NFT ownership tools:

```bash
goldrush-mcp-cli nft_for_address -h
goldrush-mcp-cli nft_check_ownership -h
```

Inspect token approval and gas tools:

```bash
goldrush-mcp-cli token_approvals -h
goldrush-mcp-cli gas_prices -h
```

## Practical Rules

- Start with `getAllChains` or a wallet-specific balance read before running transaction history tools.
- Keep scopes narrow: one wallet, one chain, one date range when possible.
- Expect stdio cold-start delay on the first run because the MCP server is started through `npx`.
- The GoldRush key is injected into the subprocess as `GOLDRUSH_API_KEY`; it is not sent as an HTTP auth header by `uxc`.
- Use positional JSON if a tool expects arrays or nested filters.

## Fallback Equivalence

- `goldrush-mcp-cli <operation> ...` is equivalent to `uxc --auth goldrush-mcp --inject-env GOLDRUSH_API_KEY={{secret}} "npx -y @covalenthq/goldrush-mcp-server@latest" <operation> ...` when the link is created as documented above.
