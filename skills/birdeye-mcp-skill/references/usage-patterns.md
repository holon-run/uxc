# Usage Patterns

This skill defaults to fixed link command `birdeye-mcp-cli`.

## Setup

```bash
command -v birdeye-mcp-cli
uxc link birdeye-mcp-cli https://mcp.birdeye.so/mcp
birdeye-mcp-cli -h
```

Auth setup:

```bash
uxc auth credential set birdeye-mcp --auth-type api_key --header "X-API-KEY={{secret}}" --secret-env BIRDEYE_API_KEY
uxc auth binding add --id birdeye-mcp --host mcp.birdeye.so --path-prefix /mcp --scheme https --credential birdeye-mcp --priority 100
```

Optional secret manager source:

```bash
uxc auth credential set birdeye-mcp --auth-type api_key --header "X-API-KEY={{secret}}" --secret-op op://Engineering/birdeye/api-key
```

## Help-First Discovery

```bash
birdeye-mcp-cli -h
```

Then inspect the concrete operation you want from the live tool list:

```bash
birdeye-mcp-cli <operation> -h
```

## Practical Rules

- Start with host help because Birdeye MCP is beta and the tool list can evolve.
- After picking the relevant operation, inspect `<operation> -h` before the first real call.
- Keep scopes narrow: one token, one pair, one chain, or one market slice at a time.
- Birdeye MCP is a read-only market/discovery surface; do not treat it as trading execution.
- If a probe returns `401 Unauthorized`, confirm the `X-API-KEY` credential binding first.

## Fallback Equivalence

- `birdeye-mcp-cli <operation> ...` is equivalent to `uxc https://mcp.birdeye.so/mcp <operation> ...`.
