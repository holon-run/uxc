# NEAR JSON-RPC Skill - Usage Patterns

## Link Setup

```bash
command -v near-jsonrpc-cli
uxc link near-jsonrpc-cli https://free.rpc.fastnear.com \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json
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
near-jsonrpc-cli gas_price --input-json '{"block_id":null}'

# Read validator sets
near-jsonrpc-cli validators --input-json '{"epoch_reference":null}'

# Read a chunk by chunk hash
near-jsonrpc-cli chunk '{"chunk_id":"75cewvnKFLrJshoUft1tiUC9GriuxWTc4bWezjy2MoPR"}'
```

## Provider Override

```bash
# Relink the same command to another provider from the official NEAR RPC providers page
uxc link near-jsonrpc-cli https://<near-rpc-provider-host> \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json
```

Do not relink to deprecated `near.org` or `pagoda.co` public RPC hosts.

If a provider later exposes usable OpenRPC or `rpc.discover`, validate that first before dropping the fixed schema override.

## Fallback Equivalence

- `near-jsonrpc-cli <operation> ...` is equivalent to
  `uxc https://free.rpc.fastnear.com --schema-url <near_openrpc_schema> <operation> ...`.
