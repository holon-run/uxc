# Sui JSON-RPC Skill - Usage Patterns

## Link Setup

```bash
command -v sui-jsonrpc-cli
uxc link sui-jsonrpc-cli https://fullnode.mainnet.sui.io
sui-jsonrpc-cli -h
```

## Read Examples

```bash
# Read the chain identifier
sui-jsonrpc-cli sui_getChainIdentifier

# Read the latest executed checkpoint sequence number
sui-jsonrpc-cli sui_getLatestCheckpointSequenceNumber

# Read one checkpoint by sequence number
sui-jsonrpc-cli sui_getCheckpoint id=254502592

# Read the current reference gas price
sui-jsonrpc-cli suix_getReferenceGasPrice

# Read the latest system state
sui-jsonrpc-cli suix_getLatestSuiSystemState
```

## Object Lookup Examples

```bash
# Read an object by id using key=value input
sui-jsonrpc-cli sui_getObject object_id=0x6

# Read an object by id using positional JSON
sui-jsonrpc-cli sui_getObject '{"object_id":"0x6"}'
```

## Help-First Examples

```bash
sui-jsonrpc-cli sui_getLatestCheckpointSequenceNumber -h
sui-jsonrpc-cli sui_getCheckpoint -h
sui-jsonrpc-cli sui_getObject -h
sui-jsonrpc-cli suix_subscribeEvent -h
```

## Subscription Examples

```bash
# Subscribe to events from a package and write NDJSON to a sink file
uxc subscribe start \
  wss://<verified-sui-rpc-host> \
  suix_subscribeEvent \
  '{"params":[{"Package":"0x2"}]}' \
  --sink file:$HOME/.uxc/subscriptions/sui-events.ndjson

# Subscribe to transaction effects that match a filter
uxc subscribe start \
  wss://<verified-sui-rpc-host> \
  suix_subscribeTransaction \
  '{"params":[{"FromAddress":"0x0000000000000000000000000000000000000000000000000000000000000000"}]}' \
  --sink file:$HOME/.uxc/subscriptions/sui-transactions.ndjson

# Inspect and stop a running job
uxc subscribe status <job_id>
uxc subscribe stop <job_id>
```

Use a WebSocket endpoint you have validated with your Sui provider; do not assume the public HTTPS fullnode host automatically maps to a working `wss://` pubsub endpoint.

## Fallback Equivalence

- `sui-jsonrpc-cli <operation> ...` is equivalent to
  `uxc https://fullnode.mainnet.sui.io <operation> ...`.
