# Blocknative Gas Platform Skill - Usage Patterns

## Link Setup

```bash
command -v blocknative-openapi-cli
uxc link blocknative-openapi-cli https://api.blocknative.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/blocknative-openapi-skill/references/blocknative-gas.openapi.json
blocknative-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set blocknative \
  --auth-type api_key \
  --api-key-header Authorization \
  --secret-env BLOCKNATIVE_API_KEY

uxc auth binding add \
  --id blocknative \
  --host api.blocknative.com \
  --scheme https \
  --credential blocknative \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://api.blocknative.com
```

## Read Examples

```bash
# Discover supported chains and their feature flags
blocknative-openapi-cli get:/chains

# Read gas price confidence intervals for Ethereum mainnet
blocknative-openapi-cli get:/gasprices/blockprices \
  chainid=1 \
  confidenceLevels=70,90,99

# Read gas prices by system + network instead of chainid
blocknative-openapi-cli get:/gasprices/blockprices \
  system=story \
  network=mainnet

# Read Ethereum base fee and blob fee predictions
blocknative-openapi-cli get:/gasprices/basefee-estimates

# Read current pending gas distribution for Ethereum mainnet
blocknative-openapi-cli get:/gasprices/distribution \
  chainid=1
```

## Error Handling Pattern

```bash
# Keep error inspection at the UXC envelope level before debugging provider-specific fields
blocknative-openapi-cli get:/gasprices/basefee-estimates | jq '{ok, error, data}'

# If auth is missing or invalid, inspect .error first before assuming the endpoint is unavailable
blocknative-openapi-cli get:/gasprices/distribution chainid=1 | jq '.error'
```

## Fallback Equivalence

- `blocknative-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.blocknative.com --schema-url <blocknative_openapi_schema> <operation> ...`.
