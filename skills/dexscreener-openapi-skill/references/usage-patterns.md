# DexScreener API Skill - Usage Patterns

## Link Setup

```bash
command -v dexscreener-openapi-cli
uxc link dexscreener-openapi-cli https://api.dexscreener.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/dexscreener-openapi-skill/references/dexscreener-public.openapi.json
dexscreener-openapi-cli -h
```

## Read Examples

```bash
# Read the latest token profile feed
dexscreener-openapi-cli get:/token-profiles/latest/v1

# Read latest boosted tokens
dexscreener-openapi-cli get:/token-boosts/latest/v1

# Read top boosted tokens
dexscreener-openapi-cli get:/token-boosts/top/v1

# Search pairs by query
dexscreener-openapi-cli get:/latest/dex/search q=solana

# Read a specific pair on one chain
dexscreener-openapi-cli get:/latest/dex/pairs/{chainId}/{pairId} \
  chainId=solana \
  pairId=GgzbfpKtozV6Hyiahkh2yNVZBZsJa4pcetCmjNtgEXiM

# Read token market rows for one token address on one chain
dexscreener-openapi-cli get:/tokens/v1/{chainId}/{tokenAddresses} \
  chainId=solana \
  tokenAddresses=So11111111111111111111111111111111111111112
```

## Fallback Equivalence

- `dexscreener-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.dexscreener.com --schema-url <dexscreener_openapi_schema> <operation> ...`.
