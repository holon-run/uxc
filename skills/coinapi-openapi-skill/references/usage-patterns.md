# CoinAPI REST Skill - Usage Patterns

## Link Setup

```bash
command -v coinapi-openapi-cli
uxc link coinapi-openapi-cli https://rest.coinapi.io \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/coinapi-openapi-skill/references/coinapi-market.openapi.json
coinapi-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set coinapi \
  --auth-type api_key \
  --api-key-header X-CoinAPI-Key \
  --secret-env COINAPI_KEY

uxc auth binding add \
  --id coinapi \
  --host rest.coinapi.io \
  --scheme https \
  --credential coinapi \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://rest.coinapi.io
```

## Read Examples

```bash
# Read current exchange rate
coinapi-openapi-cli get:/v1/exchangerate/{asset_id_base}/{asset_id_quote} \
  asset_id_base=BTC \
  asset_id_quote=USD

# Read current quote snapshots
coinapi-openapi-cli get:/v1/quotes/current filter_symbol_id=BINANCE_SPOT_BTC_USDT

# Read latest OHLCV candles
coinapi-openapi-cli get:/v1/ohlcv/{symbol_id}/latest \
  symbol_id=BINANCE_SPOT_BTC_USDT \
  period_id=1DAY \
  limit=10

# Read latest trades
coinapi-openapi-cli get:/v1/trades/{symbol_id}/latest \
  symbol_id=BINANCE_SPOT_BTC_USDT \
  limit=20

# Read latest order book snapshot
coinapi-openapi-cli get:/v1/orderbooks/{symbol_id}/latest \
  symbol_id=BINANCE_SPOT_BTC_USDT \
  limit_levels=20
```

## Fallback Equivalence

- `coinapi-openapi-cli <operation> ...` is equivalent to
  `uxc https://rest.coinapi.io --schema-url <coinapi_openapi_schema> <operation> ...`.
