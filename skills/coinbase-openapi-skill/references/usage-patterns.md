# Coinbase Advanced Trade Skill - Usage Patterns

## Link Setup

```bash
command -v coinbase-openapi-cli
uxc link coinbase-openapi-cli https://api.coinbase.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/coinbase-openapi-skill/references/coinbase-advanced-trade.openapi.json
coinbase-openapi-cli -h
```

## Auth Setup

Store the Coinbase API key id and private key, then let `uxc` mint the short-lived Advanced Trade JWT per request:

```bash
uxc auth credential set coinbase-advanced-trade \
  --auth-type api_key \
  --field key_id=env:COINBASE_KEY_ID \
  --field private_key=env:COINBASE_PRIVATE_KEY

uxc auth binding add \
  --id coinbase-advanced-trade \
  --host api.coinbase.com \
  --path-prefix /api/v3/brokerage \
  --scheme https \
  --credential coinbase-advanced-trade \
  --signer-json '{"kind":"jwt_bearer_v1","algorithm":"es256","private_key_field":"private_key","header_typ":"JWT","header_kid_field":"key_id","expires_in_seconds":120,"claims":{"static":{"iss":"cdp"},"from_fields":{"sub":"key_id"},"time":{"nbf":"now","exp":"now_plus_ttl"}},"request_claim":{"name":"uri","format":"string","value_template":"{{request.method}} {{request.host}}{{request.path}}"}}' \
  --priority 100
```

`COINBASE_PRIVATE_KEY` may be either `-----BEGIN EC PRIVATE KEY-----` or `-----BEGIN PRIVATE KEY-----`.

Validate the binding:

```bash
uxc auth binding match https://api.coinbase.com/api/v3/brokerage/accounts
```

## Read Examples

```bash
# List spot products
coinbase-openapi-cli get:/api/v3/brokerage/products product_type=SPOT limit=20

# Read one product
coinbase-openapi-cli get:/api/v3/brokerage/products/{product_id} product_id=BTC-USD

# Read best bid/ask for a narrow product set
coinbase-openapi-cli get:/api/v3/brokerage/best_bid_ask product_ids=BTC-USD,ETH-USD

# List accounts
coinbase-openapi-cli get:/api/v3/brokerage/accounts limit=20

# Read one account
coinbase-openapi-cli get:/api/v3/brokerage/accounts/{account_uuid} account_uuid=<account_uuid>

# List historical orders
coinbase-openapi-cli get:/api/v3/brokerage/orders/historical/batch product_id=BTC-USD order_status=OPEN limit=20

# Read one order
coinbase-openapi-cli get:/api/v3/brokerage/orders/historical/{order_id} order_id=<order_id>
```

## Write Examples

```bash
# Create a live limit order
coinbase-openapi-cli post:/api/v3/brokerage/orders \
  '{"client_order_id":"uxc-demo-order-1","product_id":"BTC-USD","side":"BUY","order_configuration":{"limit_limit_gtc":{"base_size":"0.001","limit_price":"25000.00","post_only":true}}}'

# Cancel orders in batch
coinbase-openapi-cli post:/api/v3/brokerage/orders/batch_cancel \
  '{"order_ids":["<order_id_1>","<order_id_2>"]}'
```

## Fallback Equivalence

- `coinbase-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.coinbase.com --schema-url <coinbase_advanced_trade_openapi_schema> <operation> ...`.
