# DefiLlama Prices API Skill - Usage Patterns

## Link Setup

```bash
command -v defillama-prices-openapi-cli
uxc link defillama-prices-openapi-cli https://coins.llama.fi \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-prices-openapi-skill/references/defillama-prices.openapi.json
defillama-prices-openapi-cli -h
```

This public skill does not require auth.

## Read Examples

```bash
# Read a current price for one asset
defillama-prices-openapi-cli get:/prices/current/{coins} \
  coins=coingecko:bitcoin \
  searchWidth=4h

# Read multiple assets at once
defillama-prices-openapi-cli get:/prices/current/{coins} \
  coins=coingecko:bitcoin,ethereum:0x0000000000000000000000000000000000000000 \
  searchWidth=4h
```

## Fallback Equivalence

- `defillama-prices-openapi-cli <operation> ...` is equivalent to
  `uxc https://coins.llama.fi --schema-url <defillama_prices_openapi_schema> <operation> ...`.
