# DefiLlama Pro API Skill - Usage Patterns

## Link Setup

```bash
command -v defillama-pro-openapi-cli
uxc link defillama-pro-openapi-cli https://pro-api.llama.fi \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-pro-openapi-skill/references/defillama-pro.openapi.json
defillama-pro-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set defillama-pro \
  --auth-type api_key \
  --secret-env DEFILLAMA_PRO_API_KEY \
  --path-prefix-template "/{{secret}}"

uxc auth binding add \
  --id defillama-pro \
  --host pro-api.llama.fi \
  --scheme https \
  --credential defillama-pro \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://pro-api.llama.fi
```

## Read Examples

```bash
# List tracked protocols and their top-level TVL metrics
defillama-pro-openapi-cli get:/api/protocols

# Read one protocol in detail
defillama-pro-openapi-cli get:/api/protocol/{protocol} protocol=aave

# Read chain overview metrics
defillama-pro-openapi-cli get:/api/v2/chains

# Read current prices for one or more chain-prefixed assets
defillama-pro-openapi-cli get:/coins/prices/current/{coins} \
  coins=ethereum:0x0000000000000000000000000000000000000000,coingecko:bitcoin \
  searchWidth=4h

# Discover yield pools
defillama-pro-openapi-cli get:/yields/pools

# Read one pool's yield chart history
defillama-pro-openapi-cli get:/yields/chart/{pool} pool=747c1d2a-c668-4682-b9f9-296708a3dd90

# Read stablecoin dominance for one chain
defillama-pro-openapi-cli get:/stablecoins/stablecoindominance/{chain} chain=ethereum
```

## Fallback Equivalence

- `defillama-pro-openapi-cli <operation> ...` is equivalent to
  `uxc https://pro-api.llama.fi --schema-url <defillama_pro_openapi_schema> <operation> ...`.
