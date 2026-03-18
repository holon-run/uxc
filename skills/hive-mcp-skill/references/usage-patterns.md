# Hive Intelligence MCP Skill - Usage Patterns

## Link Setup

```bash
command -v hive-mcp-cli
uxc link hive-mcp-cli https://hiveintelligence.xyz/mcp
hive-mcp-cli -h
```

## Discovery Examples

```bash
# Discover market-oriented endpoint groups
hive-mcp-cli get_market_and_price_endpoints

# Discover onchain DEX and pool analytics endpoints
hive-mcp-cli get_onchain_dex_pool_endpoints

# Discover wallet and portfolio endpoints
hive-mcp-cli get_portfolio_wallet_endpoints

# Discover token and contract endpoints
hive-mcp-cli get_token_contract_endpoints

# Discover risk and security endpoints
hive-mcp-cli get_security_risk_endpoints
```

## Schema And Invocation

```bash
# Inspect schema before invoking an endpoint
hive-mcp-cli get_api_endpoint_schema endpoint=get_token_price

# Invoke a narrow endpoint after confirming the schema
hive-mcp-cli invoke_api_endpoint \
  endpoint_name=get_token_price \
  args:='{"symbol":"BTC"}'
```

## Fallback Equivalence

- `hive-mcp-cli <operation> ...` is equivalent to
  `uxc https://hiveintelligence.xyz/mcp <operation> ...`.
