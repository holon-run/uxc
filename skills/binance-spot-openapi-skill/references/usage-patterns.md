# Binance Spot API Skill - Usage Patterns

## Link Setup

```bash
command -v binance-spot-mainnet-openapi-cli
uxc link binance-spot-mainnet-openapi-cli https://api.binance.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-spot-openapi-skill/references/binance-spot.openapi.json

command -v binance-spot-testnet-openapi-cli
uxc link binance-spot-testnet-openapi-cli https://testnet.binance.vision \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-spot-openapi-skill/references/binance-spot.openapi.json

binance-spot-mainnet-openapi-cli -h
binance-spot-testnet-openapi-cli -h
```

## Auth Setup

### Get Spot Testnet Keys

- Open `https://testnet.binance.vision`
- Create or select a Spot API key record in the testnet UI
- For `Ed25519`, generate the keypair locally first, upload the public key, then copy the `API key` Binance shows for that record
- For `HMAC`, Binance shows both `API key` and `Secret key`
- Keep the `API key` matched with the same record as the signing material

If you want to generate an Ed25519 keypair locally:

```bash
openssl genpkey -algorithm ed25519 -out binance_testnet_ed25519_private.pem
openssl pkey -in binance_testnet_ed25519_private.pem -pubout -out binance_testnet_ed25519_public.pem
```

Then export the private key PEM content and the Binance-displayed API key:

```bash
export BINANCE_TESTNET_API_KEY='<binance testnet api key for this Ed25519 record>'
export BINANCE_TESTNET_ED25519_PRIVATE_KEY="$(cat /absolute/path/to/binance_testnet_ed25519_private.pem)"
```

### Ed25519 Setup

```bash
uxc auth credential set binance-spot-mainnet \
  --auth-type api_key \
  --field api_key=env:BINANCE_MAINNET_API_KEY \
  --field private_key=env:BINANCE_MAINNET_ED25519_PRIVATE_KEY

uxc auth credential set binance-spot-testnet \
  --auth-type api_key \
  --field api_key=env:BINANCE_TESTNET_API_KEY \
  --field private_key=env:BINANCE_TESTNET_ED25519_PRIVATE_KEY
```

```bash
SIGNER='{"kind":"ed25519_query_v1","algorithm":"ed25519","signing_field":"private_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"base64","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}'

uxc auth binding add \
  --id binance-spot-mainnet \
  --host api.binance.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-spot-mainnet \
  --signer-json "$SIGNER" \
  --priority 100

uxc auth binding add \
  --id binance-spot-testnet \
  --host testnet.binance.vision \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-spot-testnet \
  --signer-json "$SIGNER" \
  --priority 100
```

## Legacy HMAC Setup

```bash
uxc auth credential set binance-spot-mainnet-hmac \
  --auth-type api_key \
  --field api_key=env:BINANCE_MAINNET_API_KEY \
  --field secret_key=env:BINANCE_MAINNET_SECRET_KEY

uxc auth credential set binance-spot-testnet-hmac \
  --auth-type api_key \
  --field api_key=env:BINANCE_TESTNET_API_KEY \
  --field secret_key=env:BINANCE_TESTNET_SECRET_KEY

HMAC_SIGNER='{"kind":"hmac_query_v1","algorithm":"hmac_sha256","signing_field":"secret_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"hex","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}'
```

Common failure mode:

- `-1022 Signature for this request is not valid.`
  - Usually means the `API key` and signing material came from different Binance key records.
  - Ed25519 requires the Binance-displayed `API key` for that uploaded public key record.
  - The uploaded public key itself is not the `API key`.

## Public Read Examples

```bash
# Server time
binance-spot-mainnet-openapi-cli get:/api/v3/time

# Exchange filters and symbol metadata
binance-spot-mainnet-openapi-cli get:/api/v3/exchangeInfo symbol=BTCUSDT

# Latest ticker price
binance-spot-mainnet-openapi-cli get:/api/v3/ticker/price symbol=BTCUSDT

# 24h stats
binance-spot-mainnet-openapi-cli get:/api/v3/ticker/24hr symbol=BTCUSDT

# Recent trades
binance-spot-mainnet-openapi-cli get:/api/v3/trades symbol=BTCUSDT limit=20

# Klines
binance-spot-mainnet-openapi-cli get:/api/v3/klines symbol=BTCUSDT interval=1h limit=24

# Order book
binance-spot-mainnet-openapi-cli get:/api/v3/depth symbol=BTCUSDT limit=20
```

## Signed Read Examples

```bash
# Account balances on testnet
binance-spot-testnet-openapi-cli get:/api/v3/account omitZeroBalances=true recvWindow=5000

# Current open orders
binance-spot-testnet-openapi-cli get:/api/v3/openOrders symbol=BTCUSDT recvWindow=5000

# Query one order
binance-spot-testnet-openapi-cli get:/api/v3/order symbol=BTCUSDT orderId=123456 recvWindow=5000

# Order history
binance-spot-testnet-openapi-cli get:/api/v3/allOrders symbol=BTCUSDT limit=20 recvWindow=5000

# Trade history
binance-spot-testnet-openapi-cli get:/api/v3/myTrades symbol=BTCUSDT limit=20 recvWindow=5000

# Current order count usage
binance-spot-testnet-openapi-cli get:/api/v3/rateLimit/order recvWindow=5000
```

## Testnet-First Write Examples

```bash
# Validate order shape and signature without placing a real order
binance-spot-testnet-openapi-cli post:/api/v3/order/test \
  symbol=BTCUSDT side=BUY type=MARKET quoteOrderQty=100 recvWindow=5000

# Place a real testnet order
binance-spot-testnet-openapi-cli post:/api/v3/order \
  symbol=BTCUSDT side=BUY type=LIMIT timeInForce=GTC quantity=0.001 price=20000 recvWindow=5000

# Cancel a testnet order
binance-spot-testnet-openapi-cli delete:/api/v3/order \
  symbol=BTCUSDT orderId=123456 recvWindow=5000

# Cancel all open orders on a symbol
binance-spot-testnet-openapi-cli delete:/api/v3/openOrders \
  symbol=BTCUSDT recvWindow=5000
```

## Mainnet Write Guardrail

```bash
# Only run after explicit user confirmation
binance-spot-mainnet-openapi-cli post:/api/v3/order \
  symbol=BTCUSDT side=BUY type=MARKET quoteOrderQty=100 recvWindow=5000
```

## Fallback Equivalence

- `binance-spot-mainnet-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.binance.com --schema-url <binance_spot_openapi_schema> <operation> ...`.
- `binance-spot-testnet-openapi-cli <operation> ...` is equivalent to
  `uxc https://testnet.binance.vision --schema-url <binance_spot_openapi_schema> <operation> ...`.
