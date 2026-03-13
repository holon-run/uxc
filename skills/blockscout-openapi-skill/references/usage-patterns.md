# Blockscout Explorer API Skill - Usage Patterns

## Link Setup

```bash
command -v blockscout-openapi-cli
uxc link blockscout-openapi-cli https://eth.blockscout.com/api/v2 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json
blockscout-openapi-cli -h
```

## Read Examples

```bash
# Read one block
blockscout-openapi-cli get:/blocks/{block_number_or_hash} block_number_or_hash=latest

# Read one address summary
blockscout-openapi-cli get:/addresses/{address_hash} address_hash=0xd8da6bf26964af9d7eed9e03e53415d37aa96045

# Read one address token balances
blockscout-openapi-cli get:/addresses/{address_hash}/token-balances \
  address_hash=0xd8da6bf26964af9d7eed9e03e53415d37aa96045

# Read one address transaction history
blockscout-openapi-cli get:/addresses/{address_hash}/transactions \
  address_hash=0xd8da6bf26964af9d7eed9e03e53415d37aa96045

# Read token metadata
blockscout-openapi-cli get:/tokens/{address_hash} \
  address_hash=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48

# Read token holders
blockscout-openapi-cli get:/tokens/{address_hash}/holders \
  address_hash=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48

# Read transaction detail
blockscout-openapi-cli get:/transactions/{hash} \
  hash=0x4e3f3bc239f496f59c3e4d4a4d5f10f7f0d6d9f4cd790beeb520d05f6f7d98ae
```

## Relink For Another Deployment

```bash
uxc link blockscout-openapi-cli https://optimism.blockscout.com/api/v2 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blockscout-openapi-skill/references/blockscout-v2.openapi.json
```

## Fallback Equivalence

- `blockscout-openapi-cli <operation> ...` is equivalent to
  `uxc <blockscout_api_v2_host> --schema-url <blockscout_openapi_schema> <operation> ...`.
