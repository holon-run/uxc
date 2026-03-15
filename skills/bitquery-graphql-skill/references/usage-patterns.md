# Usage Patterns

This skill defaults to fixed link command `bitquery-graphql-cli`.

## Authentication Setup

Login with OAuth client credentials:

```bash
uxc auth oauth login bitquery-graphql \
  --endpoint https://streaming.bitquery.io/graphql \
  --flow client_credentials \
  --client-id <client_id> \
  --client-secret <client_secret> \
  --scope api \
  --token-endpoint https://oauth2.bitquery.io/oauth2/token
```

Bind the endpoint:

```bash
uxc auth binding add \
  --id bitquery-graphql \
  --host streaming.bitquery.io \
  --path-prefix /graphql \
  --scheme https \
  --credential bitquery-graphql \
  --priority 100
```

Check auth state:

```bash
uxc auth binding match https://streaming.bitquery.io/graphql
uxc auth oauth info bitquery-graphql
```

## Link Setup

```bash
command -v bitquery-graphql-cli
uxc link bitquery-graphql-cli https://streaming.bitquery.io/graphql
bitquery-graphql-cli -h
```

## Help-First Discovery

```bash
bitquery-graphql-cli query/EVM -h
bitquery-graphql-cli query/Trading -h
```

## Query Examples

Minimal EVM trade query:

```bash
bitquery-graphql-cli query/EVM '{"network":"eth","dataset":"combined","_select":"DEXTrades(limit: {count: 1}) { Transaction { Hash } }"}'
```

Verified Base DEX trade query:

```bash
bitquery-graphql-cli query/EVM '{"network":"base","dataset":"combined","_select":"DEXTrades(limit: {count: 1}) { Block { Time } Transaction { Hash } Trade { Buy { Amount Buyer Currency { Symbol SmartContract } } Sell { Amount Seller Currency { Symbol SmartContract } } } }"}'
```

Mempool example:

```bash
bitquery-graphql-cli query/EVM '{"network":"eth","mempool":true,"_select":"DEXTrades(limit: {count: 5}) { Transaction { Hash } }"}'
```

Trading root example:

```bash
bitquery-graphql-cli query/Trading '{"dataset":"combined","_select":"Pairs(limit: {count: 5}) { Market { BaseCurrency { Symbol } QuoteCurrency { Symbol } } }"}'
```

## Subscription Usage

GraphQL subscriptions appear in schema discovery:

```bash
bitquery-graphql-cli subscription/EVM -h
```

Bitquery subscriptions are now validated through `uxc subscribe`, but they still need an explicit `_select` and a stream-friendly shape.

Recommended first live subscription:

```bash
uxc subscribe start https://streaming.bitquery.io/graphql \
  subscription/EVM \
  '{"network":"bsc","mempool":true,"_select":"Transfers { Transaction { Hash From To } Transfer { Amount Type Currency { Name } } }"}' \
  --auth bitquery-graphql \
  --sink file:$HOME/.uxc/subscriptions/bitquery-mempool.ndjson
```

Guidance:

- prefer `subscription/EVM` as the first validation target
- always pass `_select`
- do not assume `subscription/Trading` without a selection will stream useful data
- for stable automation, keep `query/*` as the default path unless you explicitly need a live stream

## Output Parsing

Rely on envelope fields:

- Success: `ok == true`, consume `data`
- Failure: `ok == false`, inspect `error.code` and `error.message`

## Fallback Equivalence

- `bitquery-graphql-cli <operation> ...` is equivalent to `uxc https://streaming.bitquery.io/graphql <operation> ...`.
- If link setup is temporarily unavailable, use the direct `uxc "https://streaming.bitquery.io/graphql" ...` form as fallback.
