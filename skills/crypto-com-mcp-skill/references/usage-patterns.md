# Crypto.com MCP Skill - Usage Patterns

## Link Setup

```bash
command -v crypto-com-mcp-cli
uxc link crypto-com-mcp-cli https://mcp.crypto.com/market-data/mcp
crypto-com-mcp-cli -h
```

## Help-First Discovery

```bash
crypto-com-mcp-cli -h
crypto-com-mcp-cli get_ticker -h
crypto-com-mcp-cli get_book -h
crypto-com-mcp-cli get_candlestick -h
```

## Read Examples

```bash
# List supported instruments
crypto-com-mcp-cli get_instruments

# Inspect a single instrument
crypto-com-mcp-cli get_instrument instrument_name=BTC_USDT

# Read a single ticker
crypto-com-mcp-cli get_ticker instrument_name=BTC_USDT

# Read an order book snapshot
crypto-com-mcp-cli get_book instrument_name=BTC_USDT depth=20

# Read recent candles
crypto-com-mcp-cli get_candlestick instrument_name=BTC_USDT timeframe=1h

# Read recent trades
crypto-com-mcp-cli get_trades instrument_name=BTC_USDT limit=20
```

## Fallback Equivalence

- `crypto-com-mcp-cli <operation> ...` is equivalent to
  `uxc https://mcp.crypto.com/market-data/mcp <operation> ...`.
