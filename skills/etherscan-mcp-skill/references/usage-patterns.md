# Usage Patterns

This skill defaults to fixed link command `etherscan-mcp-cli`.

## Setup

```bash
command -v etherscan-mcp-cli
uxc link etherscan-mcp-cli https://mcp.etherscan.io/mcp
etherscan-mcp-cli -h
```

Auth setup:

```bash
uxc auth credential set etherscan-mcp --auth-type bearer --secret-env ETHERSCAN_API_KEY
uxc auth binding add --id etherscan-mcp --host mcp.etherscan.io --path-prefix /mcp --scheme https --credential etherscan-mcp --priority 100
```

Optional secret manager source:

```bash
uxc auth credential set etherscan-mcp --auth-type bearer --secret-op op://Engineering/etherscan/api-key
```

## Help-First Discovery

```bash
etherscan-mcp-cli balance -h
etherscan-mcp-cli tokenTopHolders -h
etherscan-mcp-cli getContractAbi -h
etherscan-mcp-cli transaction -h
```

If the first probe is unauthenticated, `uxc https://mcp.etherscan.io/mcp -h` currently returns `401 Unauthorized`. Configure bearer auth, then retry.

## Address Investigation

Start with operation help, then pass a single address:

```bash
etherscan-mcp-cli balance address=0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045
```

`balance` also accepts optional `chainid`:

```bash
etherscan-mcp-cli balance '{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045","chainid":1}'
```

## Token Holder Analysis

Inspect the exact accepted fields first:

```bash
etherscan-mcp-cli tokenTopHolders -h
etherscan-mcp-cli balanceERC20 -h
```

Real parameter names from current MCP schema:

```bash
etherscan-mcp-cli tokenTopHolders contractaddress=0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 limit=10 sortorder=desc
etherscan-mcp-cli balanceERC20 '{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045","contractaddress":"0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","chainid":1}'
```

Tier note:

```bash
etherscan-mcp-cli getTokenInfo contractaddress=0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```

With the current test key, `getTokenInfo` returned `NOTOK` and an API Pro upgrade message. Expect plan-gated behavior on some tools.

## Contract Research

Check the contract lookup schema first:

```bash
etherscan-mcp-cli getContractAbi -h
etherscan-mcp-cli getContractSourceCode -h
etherscan-mcp-cli getContractCreation -h
```

Then run the lookup with the smallest payload that matches help:

```bash
etherscan-mcp-cli getContractAbi address=0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
etherscan-mcp-cli getContractSourceCode '{"address":"0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","chainid":1}'
etherscan-mcp-cli getContractCreation contractaddresses=0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
```

## Transaction Investigation

Check the schema first:

```bash
etherscan-mcp-cli transaction -h
etherscan-mcp-cli checkTransaction -h
etherscan-mcp-cli txList -h
```

Typical read-only examples:

```bash
etherscan-mcp-cli transaction txhash=0x51453d98c6f7b1c6fd4e2e39d4f10b4a13c7e7f0f6f1f5c2457cb6c58a12f8ab
etherscan-mcp-cli checkTransaction txhash=0x51453d98c6f7b1c6fd4e2e39d4f10b4a13c7e7f0f6f1f5c2457cb6c58a12f8ab
etherscan-mcp-cli txList '{"address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045","startblock":0,"endblock":99999999,"page":1,"offset":5,"sort":"desc"}'
```

## Auth Failure Recovery

When calls fail with `401 Unauthorized`:

```bash
uxc auth binding match https://mcp.etherscan.io/mcp
uxc auth credential info etherscan-mcp
```

If needed, recreate the bearer credential:

```bash
uxc auth credential set etherscan-mcp --auth-type bearer --secret-env ETHERSCAN_API_KEY
```

## Fallback Equivalence

- `etherscan-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.etherscan.io/mcp <operation> ...`.
