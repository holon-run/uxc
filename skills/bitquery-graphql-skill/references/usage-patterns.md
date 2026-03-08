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

## Subscription Caution

GraphQL subscriptions appear in schema discovery:

```bash
bitquery-graphql-cli subscription/EVM -h
```

Use them only after validating runtime behavior in the current environment. For stable automation, prefer `query/*`.

## Output Parsing

Rely on envelope fields:

- Success: `ok == true`, consume `data`
- Failure: `ok == false`, inspect `error.code` and `error.message`

## Fallback Equivalence

- `bitquery-graphql-cli <operation> ...` is equivalent to `uxc https://streaming.bitquery.io/graphql <operation> ...`.
- If link setup is temporarily unavailable, use the direct `uxc "https://streaming.bitquery.io/graphql" ...` form as fallback.
