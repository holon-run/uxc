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
```

## Fallback Equivalence

- `sui-jsonrpc-cli <operation> ...` is equivalent to
  `uxc https://fullnode.mainnet.sui.io <operation> ...`.
