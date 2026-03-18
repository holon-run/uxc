# mempool.space API Skill - Usage Patterns

## Link Setup

```bash
command -v mempool-space-openapi-cli
uxc link mempool-space-openapi-cli https://mempool.space/api \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/mempool-space-openapi-skill/references/mempool-space-public.openapi.json
mempool-space-openapi-cli -h
```

## Read Examples

```bash
# Read current public fee recommendations
mempool-space-openapi-cli get:/v1/fees/recommended

# Read aggregate mempool state
mempool-space-openapi-cli get:/mempool

# Read the current block tip height
mempool-space-openapi-cli get:/blocks/tip/height

# Read summary stats for a Bitcoin address
mempool-space-openapi-cli get:/address/{address} \
  address=bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh

# Read whether a transaction is confirmed
mempool-space-openapi-cli get:/tx/{txid}/status \
  txid=4d5d7f2d5dc69aa68a51887db07dd6d906f31f9141320f9f0b4bab76d735a47f

# Read latest Lightning network statistics
mempool-space-openapi-cli get:/v1/lightning/statistics/latest

# Search Lightning nodes and channels by alias or id fragment
mempool-space-openapi-cli get:/v1/lightning/search \
  searchText=bfx

# Read top Lightning node rankings
mempool-space-openapi-cli get:/v1/lightning/nodes/rankings

# Read one Lightning node
mempool-space-openapi-cli get:/v1/lightning/nodes/{public_key} \
  public_key=033d8656219478701227199cbd6f670335c8d408a92ae88b962c49d4dc0e83e025

# Read active channels for a Lightning node
mempool-space-openapi-cli get:/v1/lightning/channels \
  public_key=033d8656219478701227199cbd6f670335c8d408a92ae88b962c49d4dc0e83e025 \
  status=active

# Read one Lightning channel by id
mempool-space-openapi-cli get:/v1/lightning/channels/{short_id} \
  short_id=835866331763769345
```

## Fallback Equivalence

- `mempool-space-openapi-cli <operation> ...` is equivalent to
  `uxc https://mempool.space/api --schema-url <mempool_space_openapi_schema> <operation> ...`.
