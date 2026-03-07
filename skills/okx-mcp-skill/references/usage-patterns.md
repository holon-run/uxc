# Usage Patterns

This skill defaults to fixed link command `okx-mcp-cli`.

## Setup

```bash
command -v okx-mcp-cli
uxc link okx-mcp-cli https://web3.okx.com/api/v1/onchainos-mcp
okx-mcp-cli -h
```

Quick trial auth (shared doc key):

```bash
uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret d573a84c-8e79-4a35-b0c6-427e9ad2478d
uxc auth binding add --id okx-mcp --host web3.okx.com --path-prefix /api/v1/onchainos-mcp --scheme https --credential okx-mcp --priority 100
```

For long-term usage, replace with your own key and keep explicit header config:

```bash
uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-env OKX_ACCESS_KEY
uxc auth binding add --id okx-mcp --host web3.okx.com --path-prefix /api/v1/onchainos-mcp --scheme https --credential okx-mcp --priority 100
```

## Help-First Discovery

```bash
okx-mcp-cli dex-okx-market-price-chains
okx-mcp-cli dex-okx-dex-aggregator-supported-chains
okx-mcp-cli dex-okx-balance-chains
okx-mcp-cli dex-okx-market-token-search -h
okx-mcp-cli dex-okx-dex-quote -h
```

Common chainIndex quick picks:
- `1` (Ethereum), `56` (BSC), `196` (XLayer), `501` (Solana), `8453` (Base), `42161` (Arbitrum)

## Token Discovery

```bash
okx-mcp-cli dex-okx-market-token-search chains=1 search=USDC
okx-mcp-cli dex-okx-market-token-ranking chains=1 sortBy=priceChange timeFrame=24h
okx-mcp-cli dex-okx-market-token-holder chainIndex=1 tokenContractAddress=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```

## Market Data

```bash
okx-mcp-cli dex-okx-market-price '{"items":[{"chainIndex":"1","tokenContractAddress":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"}]}'
okx-mcp-cli dex-okx-market-trades chainIndex=1 tokenContractAddress=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 limit=50
okx-mcp-cli dex-okx-market-candlesticks-history '{"chainIndex":"1","tokenContractAddress":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","bar":"1H","limit":"100"}'
```

## Wallet

```bash
okx-mcp-cli dex-okx-balance-total-value address=0xYourAddress chains=1,8453
okx-mcp-cli dex-okx-balance-total-token-balances address=0xYourAddress chains=1,8453
okx-mcp-cli dex-okx-balance-specific-token-balance '{"address":"0xYourAddress","tokenContractAddresses":[{"chainIndex":"1","tokenContractAddress":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"}]}'
```

## Swap (Confirm First)

```bash
okx-mcp-cli dex-okx-dex-quote chainIndex=1 fromTokenAddress=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 toTokenAddress=0xdac17f958d2ee523a2206206994597c13d831ec7 amount=1000000 swapMode=exactIn
```

Run `dex-okx-dex-approve-transaction`, `dex-okx-dex-swap`, or `dex-okx-dex-solana-swap-instruction` only with explicit user approval.

## Auth/Header Failure Recovery

When response includes `Request header OK-ACCESS-KEY can not be empty`:

```bash
uxc auth binding match https://web3.okx.com/api/v1/onchainos-mcp
uxc auth credential info okx-mcp
```

If needed, reset credential with explicit header:

```bash
uxc auth credential set okx-mcp --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-env OKX_ACCESS_KEY
```

## Fallback Equivalence

- `okx-mcp-cli <operation> ...` is equivalent to `uxc https://web3.okx.com/api/v1/onchainos-mcp <operation> ...`.
