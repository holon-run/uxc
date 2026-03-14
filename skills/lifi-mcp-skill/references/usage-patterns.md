# LI.FI MCP Usage Patterns

Use these patterns after the host and optional auth binding are working.

## 1. Basic endpoint discovery

```bash
uxc https://mcp.li.quest/mcp -h
lifi-mcp-cli -h
```

Inspect one operation before use:

```bash
lifi-mcp-cli get-quote -h
```

## 2. Chain and token resolution

Resolve chain names and IDs dynamically instead of hardcoding them:

```bash
lifi-mcp-cli get-chain-by-name name=base
lifi-mcp-cli get-chain-by-id id=42161
lifi-mcp-cli get-chains chainTypes=EVM
```

Resolve token metadata before building a quote:

```bash
lifi-mcp-cli get-token chain=base token=USDC
lifi-mcp-cli get-tokens chains=8453,42161
```

## 3. Route existence check

Use this when the user asks whether a route is available:

```bash
lifi-mcp-cli get-connections fromChain=8453 toChain=42161
```

Narrow by bridge or token only when the user cares about a specific route family.

## 4. Best-route quote

Use `get-quote` for the default path when the user wants the best executable route:

```bash
lifi-mcp-cli get-quote \
  fromChain=8453 \
  toChain=42161 \
  fromToken=USDC \
  toToken=USDC \
  fromAddress=0xYourWallet \
  fromAmount=1000000
```

Notes:

- `fromAmount` is in the token's smallest unit.
- `toAddress` is optional and defaults to `fromAddress`.
- The response contains an unsigned `transactionRequest` object.

## 5. Multiple-route comparison

Use this only when the user explicitly wants alternatives:

```bash
lifi-mcp-cli get-routes \
  fromChainId=8453 \
  toChainId=42161 \
  fromTokenAddress=0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913 \
  toTokenAddress=0xaf88d065e77c8cC2239327C5EDb3A432268e5831 \
  fromAddress=0xYourWallet \
  fromAmount=1000000
```

Then inspect a single chosen step:

```bash
lifi-mcp-cli get-step-transaction '<step-json>'
```

## 6. Pre-execution checks

Check native gas balance:

```bash
lifi-mcp-cli get-native-token-balance chain=base address=0xYourWallet
```

Check ERC20 balance:

```bash
lifi-mcp-cli get-token-balance \
  chain=base \
  tokenAddress=0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913 \
  walletAddress=0xYourWallet
```

Check allowance using the approval address from a quote:

```bash
lifi-mcp-cli get-allowance \
  chain=base \
  tokenAddress=0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913 \
  ownerAddress=0xYourWallet \
  spenderAddress=0xApprovalAddress
```

## 7. Gas and status monitoring

Get general gas data:

```bash
lifi-mcp-cli get-gas-prices
lifi-mcp-cli get-gas-suggestion chainId=8453
```

Track an already submitted cross-chain transaction:

```bash
lifi-mcp-cli get-status txHash=0x...
```

## 8. Auth and health checks

Validate the optional API key setup:

```bash
lifi-mcp-cli test-api-key
```

Check service health:

```bash
lifi-mcp-cli health-check
```
