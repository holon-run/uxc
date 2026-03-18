# Nodit Web3 Data API Skill - Usage Patterns

## Link Setup

```bash
command -v nodit-openapi-cli
uxc link nodit-openapi-cli https://web3.nodit.io \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/nodit-openapi-skill/references/nodit-web3.openapi.json
nodit-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set nodit \
  --auth-type api_key \
  --api-key-header X-API-KEY \
  --secret-env NODIT_API_KEY

uxc auth binding add \
  --id nodit \
  --host web3.nodit.io \
  --scheme https \
  --credential nodit \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://web3.nodit.io
```

## Read Examples

```bash
# Identify whether an input string is an account or transaction entity
nodit-openapi-cli post:/v1/multichain/lookupEntities \
  input=near

# If this returns HTTP 429 TOO_MANY_REQUESTS, do not treat it as auth failure.
# Back off and continue with chain-specific reads when the target chain is already known.

# Read native balance for an EVM account
nodit-openapi-cli post:/v1/{chain}/{network}/native/getNativeBalanceByAccount \
  chain=ethereum \
  network=mainnet \
  accountAddress=0xd8da6bf26964af9d7eed9e03e53415d37aa96045

# Read recent transactions for an account
nodit-openapi-cli post:/v1/{chain}/{network}/blockchain/getTransactionsByAccount \
  chain=ethereum \
  network=mainnet \
  accountAddress=0xd8da6bf26964af9d7eed9e03e53415d37aa96045 \
  limit=20

# Read token contract metadata
nodit-openapi-cli post:/v1/{chain}/{network}/token/getTokenContractMetadataByContracts \
  chain=ethereum \
  network=mainnet \
  contractAddresses:='["0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"]'

# Read token prices by contracts
nodit-openapi-cli post:/v1/{chain}/{network}/token/getTokenPricesByContracts \
  chain=ethereum \
  network=mainnet \
  contractAddresses:='["0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"]'
```

## Fallback Equivalence

- `nodit-openapi-cli <operation> ...` is equivalent to
  `uxc https://web3.nodit.io --schema-url <nodit_openapi_schema> <operation> ...`.
