# Bybit V5 Skill - Usage Patterns

## Link Setup

```bash
command -v bybit-openapi-cli
uxc link bybit-openapi-cli https://api.bybit.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/bybit-openapi-skill/references/bybit-v5.openapi.json
bybit-openapi-cli -h
```

## Read Examples

```bash
# Read Bybit server time
bybit-openapi-cli get:/v5/market/time

# Read spot instrument metadata
bybit-openapi-cli get:/v5/market/instruments-info category=spot symbol=BTCUSDT

# Read spot ticker
bybit-openapi-cli get:/v5/market/tickers category=spot symbol=BTCUSDT

# Read spot order book snapshot
bybit-openapi-cli get:/v5/market/orderbook category=spot symbol=BTCUSDT limit=20

# Read spot kline data
bybit-openapi-cli get:/v5/market/kline category=spot symbol=BTCUSDT interval=60 limit=24
```

## Guardrail Note

- Keep this v1 skill on public reads only because Bybit private REST auth uses provider-specific header signing not yet packaged into a reusable `uxc` signer flow.

## Fallback Equivalence

- `bybit-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.bybit.com --schema-url <bybit_v5_openapi_schema> <operation> ...`.
