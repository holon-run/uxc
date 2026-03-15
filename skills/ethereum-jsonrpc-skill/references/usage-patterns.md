# Ethereum JSON-RPC Usage Patterns

## Link And Discover

```bash
command -v ethereum-jsonrpc-cli
uxc link ethereum-jsonrpc-cli https://ethereum-rpc.publicnode.com \
  --schema-url https://raw.githubusercontent.com/ethereum/execution-apis/assembled-spec/refs-openrpc.json
ethereum-jsonrpc-cli -h
```

## Read Operations

```bash
ethereum-jsonrpc-cli eth_chainId
```

```bash
ethereum-jsonrpc-cli eth_blockNumber
```

```bash
ethereum-jsonrpc-cli eth_getBalance Address=0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 Block=latest
```

```bash
ethereum-jsonrpc-cli eth_getBlockByNumber '["latest", false]'
```

```bash
ethereum-jsonrpc-cli eth_call '[{"to":"0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","data":"0x313ce567"},"latest"]'
```

```bash
ethereum-jsonrpc-cli eth_getLogs '[{"fromBlock":"latest","toBlock":"latest","address":["0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"]}]'
```

## Help-First Checks

```bash
ethereum-jsonrpc-cli eth_blockNumber -h
ethereum-jsonrpc-cli eth_getBlockByNumber -h
ethereum-jsonrpc-cli eth_getBalance -h
```

## Subscription Patterns

Use a WebSocket Ethereum RPC provider you have validated with your provider before automating pubsub. Public HTTPS hosts do not guarantee a matching WebSocket endpoint. `eth_subscribe` is a runtime subscription method and is not described by the current official execution OpenRPC schema.

```bash
uxc subscribe start wss://<verified-ethereum-rpc-host> eth_subscribe \
  '{"params":["newHeads"]}' \
  --sink file:$HOME/.uxc/subscriptions/eth-new-heads.ndjson
```

```bash
uxc subscribe start wss://<verified-ethereum-rpc-host> eth_subscribe \
  '{"params":["logs",{"address":"0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"}]}' \
  --sink file:$HOME/.uxc/subscriptions/eth-usdc-logs.ndjson
```

```bash
uxc subscribe status <job_id>
uxc subscribe stop <job_id>
```

## Notes

- `ethereum-jsonrpc-cli <operation> ...` is equivalent to
  `uxc https://ethereum-rpc.publicnode.com --schema-url <ethereum_execution_openrpc_schema> <operation> ...`.
- Prefer key/value parameters for simple reads.
- Use bare JSON positional payloads when a method expects nested objects or arrays.
