# Bitget Exchange Skill - Usage Patterns

## Link Setup

```bash
command -v bitget-openapi-cli
uxc link bitget-openapi-cli https://api.bitget.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/bitget-openapi-skill/references/bitget-v2.openapi.json
bitget-openapi-cli -h
```

## Read Examples

```bash
# Read spot symbol metadata
bitget-openapi-cli get:/api/v2/spot/public/symbols symbol=BTCUSDT

# Read spot ticker
bitget-openapi-cli get:/api/v2/spot/market/tickers symbol=BTCUSDT

# Read spot candles
bitget-openapi-cli get:/api/v2/spot/market/candles symbol=BTCUSDT granularity=1h limit=24

# Read spot order book
bitget-openapi-cli get:/api/v2/spot/market/orderbook symbol=BTCUSDT limit=20
```

## Guardrail Note

- Keep this v1 skill on public reads only because Bitget private REST auth uses provider-specific header signing not yet packaged into a reusable `uxc` signer flow.

## Fallback Equivalence

- `bitget-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.bitget.com --schema-url <bitget_v2_openapi_schema> <operation> ...`.
