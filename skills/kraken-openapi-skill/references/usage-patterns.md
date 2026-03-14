# Kraken REST Skill - Usage Patterns

## Link Setup

```bash
command -v kraken-openapi-cli
uxc link kraken-openapi-cli https://api.kraken.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/kraken-openapi-skill/references/kraken-public.openapi.json
kraken-openapi-cli -h
```

## Read Examples

```bash
# Read server time
kraken-openapi-cli get:/0/public/Time

# Read asset pair metadata
kraken-openapi-cli get:/0/public/AssetPairs pair=XBTUSD

# Read ticker
kraken-openapi-cli get:/0/public/Ticker pair=XBTUSD

# Read OHLC candles
kraken-openapi-cli get:/0/public/OHLC pair=XBTUSD interval=60

# Read order book snapshot
kraken-openapi-cli get:/0/public/Depth pair=XBTUSD count=20
```

## Guardrail Note

- Keep this v1 skill on public reads only because Kraken private REST auth uses provider-specific header signing and nonce handling not yet packaged into a reusable `uxc` signer flow.

## Fallback Equivalence

- `kraken-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.kraken.com --schema-url <kraken_public_openapi_schema> <operation> ...`.
