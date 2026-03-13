# Upbit Open API Skill - Usage Patterns

## Link Setup

Choose the right regional host first. Example for Singapore:

```bash
command -v upbit-openapi-cli
uxc link upbit-openapi-cli https://sg-api.upbit.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/upbit-openapi-skill/references/upbit-public.openapi.json
upbit-openapi-cli -h
```

## Read Examples

```bash
# List markets
upbit-openapi-cli get:/v1/market/all isDetails=false

# Read ticker
upbit-openapi-cli get:/v1/ticker markets=BTC-SGD

# Read minute candles
upbit-openapi-cli get:/v1/candles/minutes/{unit} unit=60 market=BTC-SGD count=24

# Read order book
upbit-openapi-cli get:/v1/orderbook markets=BTC-SGD
```

## Guardrail Note

- Keep this v1 skill on public reads only because Upbit private APIs use provider-specific JWT generation with request-specific claims not yet packaged into a reusable `uxc` signer flow.

## Fallback Equivalence

- `upbit-openapi-cli <operation> ...` is equivalent to
  `uxc <upbit_region_host> --schema-url <upbit_public_openapi_schema> <operation> ...`.
