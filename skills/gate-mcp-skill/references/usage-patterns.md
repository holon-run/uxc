# Usage Patterns

This skill defaults to fixed link command `gate-mcp-cli`.

## Setup

```bash
command -v gate-mcp-cli
uxc link gate-mcp-cli https://api.gatemcp.ai/mcp
gate-mcp-cli -h
```

## Help-First Discovery

```bash
gate-mcp-cli -h
gate-mcp-cli cex_spot_get_spot_tickers -h
gate-mcp-cli cex_spot_get_spot_order_book -h
gate-mcp-cli cex_fx_get_fx_tickers -h
gate-mcp-cli cex_fx_get_fx_order_book -h
```

## Spot Reads

```bash
gate-mcp-cli cex_spot_list_currencies
gate-mcp-cli cex_spot_list_currency_pairs
gate-mcp-cli cex_spot_get_spot_tickers currency_pair=BTC_USDT
gate-mcp-cli cex_spot_get_spot_order_book currency_pair=BTC_USDT limit=20
gate-mcp-cli cex_spot_get_spot_trades currency_pair=BTC_USDT limit=20
gate-mcp-cli cex_spot_get_spot_candlesticks currency_pair=BTC_USDT interval=1h limit=20
```

## Futures Reads

```bash
gate-mcp-cli cex_fx_list_fx_contracts
gate-mcp-cli cex_fx_get_fx_contract contract=BTC_USDT
gate-mcp-cli cex_fx_get_fx_tickers contract=BTC_USDT
gate-mcp-cli cex_fx_get_fx_order_book contract=BTC_USDT limit=20
gate-mcp-cli cex_fx_get_fx_funding_rate contract=BTC_USDT
gate-mcp-cli cex_fx_get_fx_trades contract=BTC_USDT limit=20
```

## Fallback Equivalence

- `gate-mcp-cli <operation> ...` is equivalent to
  `uxc https://api.gatemcp.ai/mcp <operation> ...`.
