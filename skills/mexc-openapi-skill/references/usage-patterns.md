# MEXC Spot API Skill - Usage Patterns

## Link Setup

```bash
command -v mexc-openapi-cli
uxc link mexc-openapi-cli https://api.mexc.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/mexc-openapi-skill/references/mexc-spot.openapi.json
mexc-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set mexc-spot \
  --auth-type api_key \
  --field api_key=env:MEXC_API_KEY \
  --field secret_key=env:MEXC_SECRET_KEY

SIGNER='{"kind":"hmac_query_v1","algorithm":"hmac_sha256","signing_field":"secret_key","key_field":"api_key","key_placement":"header","key_name":"X-MEXC-APIKEY","signature_param":"signature","signature_encoding":"hex","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}'

uxc auth binding add \
  --id mexc-spot \
  --host api.mexc.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential mexc-spot \
  --signer-json "$SIGNER" \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://api.mexc.com/api/v3/account
```

## Read Examples

```bash
# Read ticker
mexc-openapi-cli get:/api/v3/ticker/price symbol=BTCUSDT

# Read depth
mexc-openapi-cli get:/api/v3/depth symbol=BTCUSDT limit=20

# Read account
mexc-openapi-cli get:/api/v3/account recvWindow=5000

# Read open orders
mexc-openapi-cli get:/api/v3/openOrders symbol=BTCUSDT recvWindow=5000

# Read one order
mexc-openapi-cli get:/api/v3/order symbol=BTCUSDT orderId=123456 recvWindow=5000
```

## Write Examples

```bash
# Place an order
mexc-openapi-cli post:/api/v3/order \
  symbol=BTCUSDT \
  side=BUY \
  type=LIMIT \
  quantity=0.001 \
  price=25000 \
  timeInForce=GTC \
  recvWindow=5000

# Cancel an order
mexc-openapi-cli delete:/api/v3/order symbol=BTCUSDT orderId=123456 recvWindow=5000
```

## Fallback Equivalence

- `mexc-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.mexc.com --schema-url <mexc_spot_openapi_schema> <operation> ...`.
