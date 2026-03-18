# Helius Wallet API Skill - Usage Patterns

## Link Setup

```bash
command -v helius-openapi-cli
uxc link helius-openapi-cli https://api.helius.xyz \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/helius-openapi-skill/references/helius-wallet.openapi.json
helius-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set helius \
  --auth-type api_key \
  --api-key-header X-Api-Key \
  --secret-env HELIUS_API_KEY

uxc auth binding add \
  --id helius \
  --host api.helius.xyz \
  --scheme https \
  --credential helius \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://api.helius.xyz
```

## Read Examples

```bash
# Identify a wallet with a known label
helius-openapi-cli get:/v1/wallet/{wallet}/identity \
  wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664

# Batch identity lookup for one or more addresses
helius-openapi-cli post:/v1/wallet/batch-identity \
  addresses:='["HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664"]'

# If your shell does not preserve :='[...]' quoting well, pass the same body as bare JSON
helius-openapi-cli post:/v1/wallet/batch-identity \
  '{"addresses":["HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664"]}'

# Read balances with a small page size first
helius-openapi-cli get:/v1/wallet/{wallet}/balances \
  wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664 \
  page=1 \
  limit=20 \
  showNative=true

# Read recent wallet history
helius-openapi-cli get:/v1/wallet/{wallet}/history \
  wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664 \
  limit=20

# Read recent transfers
helius-openapi-cli get:/v1/wallet/{wallet}/transfers \
  wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664 \
  limit=20

# Discover the wallet's original funding source
helius-openapi-cli get:/v1/wallet/{wallet}/funded-by \
  wallet=HXsKP7wrBWaQ8T2Vtjry3Nj3oUgwYcqq9vrHDM12G664
```

## Fallback Equivalence

- `helius-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.helius.xyz --schema-url <helius_openapi_schema> <operation> ...`.
