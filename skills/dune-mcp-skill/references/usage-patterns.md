# Usage Patterns

This skill defaults to fixed link command `dune-mcp-cli`.

## Setup

```bash
command -v dune-mcp-cli
uxc link dune-mcp-cli https://api.dune.com/mcp/v1
dune-mcp-cli -h
```

Auth setup:

```bash
uxc auth credential set dune-mcp --auth-type api_key --header "x-dune-api-key={{secret}}" --secret-env DUNE_API_KEY
uxc auth binding add --id dune-mcp --host api.dune.com --path-prefix /mcp/v1 --scheme https --credential dune-mcp --priority 100
```

Optional secret manager source:

```bash
uxc auth credential set dune-mcp --auth-type api_key --header "x-dune-api-key={{secret}}" --secret-op op://Engineering/dune/api-key
```

## Help-First Discovery

```bash
dune-mcp-cli searchTables -h
dune-mcp-cli searchTablesByContractAddress -h
dune-mcp-cli createDuneQuery -h
dune-mcp-cli executeQueryById -h
dune-mcp-cli getExecutionResults -h
```

## Table Discovery

Search by topic:

```bash
dune-mcp-cli searchTables query='uniswap swaps' blockchains=base limit=10
```

Search with schema included:

```bash
dune-mcp-cli searchTables '{"query":"uniswap swaps","blockchains":["base"],"limit":5,"includeSchema":true}'
```

Search decoded tables by contract:

```bash
dune-mcp-cli searchTablesByContractAddress contractAddress=0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```

If `listBlockchains` fails with a facet/schema error, skip it and continue with `searchTables`.

## Query Lifecycle

Create a temporary query:

```bash
dune-mcp-cli createDuneQuery \
  name='tmp uxc test: base uniswapx daily volume' \
  query="SELECT block_date, ROUND(SUM(amount_usd), 2) AS daily_volume_usd, COUNT(*) AS trades FROM uniswap.uniswapx_trades WHERE blockchain = 'base' AND block_date >= date_add('day', -7, CURRENT_DATE) GROUP BY 1 ORDER BY 1 DESC LIMIT 7"
```

Fetch query details:

```bash
dune-mcp-cli getDuneQuery query_id=6794106
```

Execute query:

```bash
dune-mcp-cli executeQueryById query_id=6794106
```

Fetch results by execution ID:

```bash
dune-mcp-cli getExecutionResults '{"executionId":"01KK31GEFYA576GN1PC3ZZJNS8","timeout":90,"limit":20}'
```

Update query content:

```bash
dune-mcp-cli updateDuneQuery queryId=6794106 query="SELECT 1"
```

## Practical Rules For SQL

- Prefer `spell` tables before raw decoded tables when they already expose the metric you need.
- Always add a time filter on partition columns such as `block_date` or `evt_block_date`.
- Keep initial result sets small with `LIMIT`.
- For IDs typed as numbers in MCP schema, `key=value` is fine because `uxc` now auto-converts numeric argument types.
- Use positional JSON when you need nested objects or tighter control over mixed types.
- When passing SQL as `key=value`, quote the whole SQL string with double quotes.

## Example: Base UniswapX Daily Volume

This real flow worked through `uxc`:

```bash
dune-mcp-cli searchTables '{"query":"UniswapX trades daily volume","blockchains":["base"],"categories":["spell"],"limit":3,"includeSchema":true}'

dune-mcp-cli createDuneQuery \
  name='tmp uxc test: base uniswapx daily volume final 2026-03-07' \
  query="SELECT block_date, ROUND(SUM(amount_usd), 2) AS daily_volume_usd, COUNT(*) AS trades FROM uniswap.uniswapx_trades WHERE blockchain = 'base' AND block_date >= date_add('day', -7, CURRENT_DATE) GROUP BY 1 ORDER BY 1 DESC LIMIT 7"

dune-mcp-cli executeQueryById '{"query_id":6794110}'
dune-mcp-cli getExecutionResults '{"executionId":"01KK31GEFYA576GN1PC3ZZJNS8","timeout":90,"limit":20}'
```

Representative result rows:

- `2026-03-06` -> `2148623.85` USD, `380` trades
- `2026-03-04` -> `5014661.68` USD, `736` trades

## Visualization And Usage

Check usage before heavy experimentation:

```bash
dune-mcp-cli getUsage
```

Generate a chart only after confirming the query results and column names:

```bash
dune-mcp-cli generateVisualization -h
```

Run `generateVisualization` only with explicit user approval because it creates saved artifacts.

## Fallback Equivalence

- `dune-mcp-cli <operation> ...` is equivalent to `uxc https://api.dune.com/mcp/v1 <operation> ...`.
