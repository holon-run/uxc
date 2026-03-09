# Binance Web3 API Skill - Usage Patterns

## Link Setup

```bash
command -v binance-web3-openapi-cli
uxc link binance-web3-openapi-cli https://web3.binance.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/binance-web3-openapi-skill/references/binance-web3.openapi.json
binance-web3-openapi-cli -h
```

## Read Examples

```bash
# Search tokens by keyword
binance-web3-openapi-cli get:/bapi/defi/v5/public/wallet-direct/buw/wallet/market/token/search \
  keyword=bnb chainIds=56 orderBy=volume24h

# Get token metadata
binance-web3-openapi-cli get:/bapi/defi/v1/public/wallet-direct/buw/wallet/dex/market/token/meta/info \
  chainId=56 contractAddress=0x55d398326f99059ff775485246999027b3197955

# Get token market snapshot
binance-web3-openapi-cli get:/bapi/defi/v4/public/wallet-direct/buw/wallet/market/token/dynamic/info \
  chainId=56 contractAddress=0x55d398326f99059ff775485246999027b3197955

# List active positions for an address
binance-web3-openapi-cli get:/bapi/defi/v3/public/wallet-direct/buw/wallet/address/pnl/active-position-list \
  address=0x0000000000000000000000000000000000000001 chainId=56 offset=0 \
  clienttype=web clientversion=1.2.0
```

## Ranking And Signal Examples

```bash
# Social hype leaderboard
binance-web3-openapi-cli get:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/social/hype/rank/leaderboard \
  chainId=56 targetLanguage=en timeRange=1 sentiment=All socialLanguage=ALL

# Unified token rank
binance-web3-openapi-cli post:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/unified/rank/list \
  '{"rankType":10,"chainId":"56","period":50,"page":1,"size":20}'

# Meme rush rank
binance-web3-openapi-cli post:/bapi/defi/v1/public/wallet-direct/buw/wallet/market/token/pulse/rank/list \
  '{"chainId":"CT_501","rankType":10,"limit":20}'

# Smart money signals
binance-web3-openapi-cli post:/bapi/defi/v1/public/wallet-direct/buw/wallet/web/signal/smart-money \
  '{"smartSignalType":"","page":1,"pageSize":50,"chainId":"56"}'
```

## Token Audit Example

```bash
request_id="$(uuidgen)"

binance-web3-openapi-cli post:/bapi/defi/v1/public/wallet-direct/security/token/audit \
  "{\"binanceChainId\":\"56\",\"contractAddress\":\"0x55d398326f99059ff775485246999027b3197955\",\"requestId\":\"${request_id}\"}"
```

## Fallback Equivalence

- `binance-web3-openapi-cli <operation> ...` is equivalent to
  `uxc https://web3.binance.com --schema-url <binance_web3_openapi_schema> <operation> ...`.
