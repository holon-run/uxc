# NEAR JSON-RPC Skill - Usage Patterns

## Link Setup

```bash
command -v near-jsonrpc-cli
uxc link near-jsonrpc-cli https://free.rpc.fastnear.com
near-jsonrpc-cli -h
```

## Read Examples

```bash
# Read node and chain status
near-jsonrpc-cli status

# Read the latest finalized block
near-jsonrpc-cli block '{"finality":"final"}'

# Read an account state
near-jsonrpc-cli query '{"request_type":"view_account","finality":"final","account_id":"near"}'

# Read gas price for the latest block context
near-jsonrpc-cli gas_price '[null]'

# Read validator sets
near-jsonrpc-cli validators '[null]'

# Read a chunk by chunk hash
near-jsonrpc-cli chunk '{"chunk_id":"75cewvnKFLrJshoUft1tiUC9GriuxWTc4bWezjy2MoPR"}'
```

## Provider Override

```bash
# Relink the same command to another provider from the official NEAR RPC providers page
uxc link near-jsonrpc-cli https://<near-rpc-provider-host>
```

Do not relink to deprecated `near.org` or `pagoda.co` public RPC hosts.

## Fallback Equivalence

- `near-jsonrpc-cli <operation> ...` is equivalent to
  `uxc https://free.rpc.fastnear.com <operation> ...`.
