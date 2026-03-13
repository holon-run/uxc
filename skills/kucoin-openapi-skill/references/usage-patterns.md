# KuCoin Unified API Skill - Usage Patterns

## Link Setup

```bash
command -v kucoin-openapi-cli
uxc link kucoin-openapi-cli https://api.kucoin.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/kucoin-openapi-skill/references/kucoin-public.openapi.json
kucoin-openapi-cli -h
```

## Read Examples

```bash
# Read symbols
kucoin-openapi-cli get:/api/v1/symbols

# Read all tickers
kucoin-openapi-cli get:/api/v1/market/allTickers

# Read order book snapshot
kucoin-openapi-cli get:/api/v1/market/orderbook/level2_20 symbol=BTC-USDT

# Read candles
kucoin-openapi-cli get:/api/v1/market/candles symbol=BTC-USDT type=1hour
```

## Guardrail Note

- Keep this v1 skill on public reads only because KuCoin private REST auth uses provider-specific headers and signing rules not yet packaged into a reusable `uxc` signer flow.

## Fallback Equivalence

- `kucoin-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.kucoin.com --schema-url <kucoin_public_openapi_schema> <operation> ...`.
