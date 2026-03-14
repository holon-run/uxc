# Usage Patterns

This skill defaults to fixed link command `coinmarketcap-mcp-cli`.

## Setup

```bash
command -v coinmarketcap-mcp-cli
uxc link coinmarketcap-mcp-cli https://mcp.coinmarketcap.com/mcp
coinmarketcap-mcp-cli -h
```

Auth setup:

```bash
uxc auth credential set coinmarketcap-mcp --auth-type api_key --header "X-CMC-MCP-API-KEY={{secret}}" --secret-env COINMARKETCAP_MCP_API_KEY
uxc auth binding add --id coinmarketcap-mcp --host mcp.coinmarketcap.com --path-prefix /mcp --scheme https --credential coinmarketcap-mcp --priority 100
```

Optional secret manager source:

```bash
uxc auth credential set coinmarketcap-mcp --auth-type api_key --header "X-CMC-MCP-API-KEY={{secret}}" --secret-op op://Engineering/coinmarketcap/mcp-api-key
```

## Help-First Discovery

```bash
coinmarketcap-mcp-cli get_crypto_quotes_latest -h
coinmarketcap-mcp-cli get_crypto_info -h
coinmarketcap-mcp-cli get_global_metrics_latest -h
coinmarketcap-mcp-cli trending_crypto_narratives -h
coinmarketcap-mcp-cli get_crypto_latest_news -h
```

## Quotes And Metadata

Get the latest quote for BTC:

```bash
coinmarketcap-mcp-cli get_crypto_quotes_latest id=1
```

Search for projects by keyword:

```bash
coinmarketcap-mcp-cli search_cryptos query='liquid staking' limit=5
```

Fetch coin metadata with positional JSON:

```bash
coinmarketcap-mcp-cli get_crypto_info '{"symbol":"ETH","aux":["urls","logo","description","tags"]}'
```

## Technical Analysis And Metrics

Inspect technical analysis options first:

```bash
coinmarketcap-mcp-cli get_crypto_technical_analysis -h
coinmarketcap-mcp-cli get_crypto_marketcap_technical_analysis -h
```

Inspect crypto metrics:

```bash
coinmarketcap-mcp-cli get_crypto_metrics -h
```

## Global Market And Narratives

Fetch global market metrics:

```bash
coinmarketcap-mcp-cli get_global_metrics_latest
```

Inspect derivatives and narrative tools:

```bash
coinmarketcap-mcp-cli get_global_crypto_derivatives_metrics -h
coinmarketcap-mcp-cli trending_crypto_narratives -h
coinmarketcap-mcp-cli get_upcoming_macro_events -h
```

## News And Semantic Search

Inspect latest news tool:

```bash
coinmarketcap-mcp-cli get_crypto_latest_news -h
```

Search crypto concepts or documentation:

```bash
coinmarketcap-mcp-cli search_crypto_info query='restaking market map' limit=5
```

## Practical Rules

- Start with quotes, metadata, or global metrics before using broader search tools.
- Keep keyword searches short and specific.
- Use positional JSON when the tool accepts arrays or nested filters.
- If a tool fails with an auth, quota, or plan error, verify the stored key and current CoinMarketCap tier.
- CoinMarketCap MCP is read-only and does not support trading.
- CoinMarketCap documents an x402 pay-per-call option, but this skill does not use it because `uxc` does not implement x402 payment handling today.

## Fallback Equivalence

- `coinmarketcap-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.coinmarketcap.com/mcp <operation> ...`.
