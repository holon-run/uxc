---
name: ethereum-jsonrpc-skill
description: Operate Ethereum execution JSON-RPC through UXC with the official execution OpenRPC schema, public EVM read methods, and eth_subscribe pubsub guardrails.
---

# Ethereum JSON-RPC Skill

Use this skill to run Ethereum execution JSON-RPC operations through `uxc` + JSON-RPC.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to a compatible Ethereum execution JSON-RPC provider.
- Access to the official execution OpenRPC schema URL:
  - `https://raw.githubusercontent.com/ethereum/execution-apis/assembled-spec/refs-openrpc.json`

## Scope

This skill covers a safe read-first Ethereum execution surface:

- chain identity and public state reads
- block, balance, logs, and call-based state reads
- `eth_subscribe` pubsub subscriptions for new heads, logs, and pending transactions

This skill does **not** cover:

- `eth_sendRawTransaction`
- `personal_*`, `admin_*`, `debug_*`, `engine_*`, `txpool_*`
- wallet signing flows
- private/authenticated RPC providers with custom auth models

## Endpoint And Schema

This skill defaults to a public read provider:

- HTTPS RPC: `https://ethereum-rpc.publicnode.com`

The operation surface comes from the official Ethereum execution OpenRPC schema:

- `https://raw.githubusercontent.com/ethereum/execution-apis/assembled-spec/refs-openrpc.json`

`uxc` JSON-RPC discovery normally depends on OpenRPC or `rpc.discover`. Ethereum RPC providers often do not expose discovery directly, so this skill uses a fixed `--schema-url` link and request flow.

The official execution OpenRPC document is strong enough for normal request/response methods, but it does not currently expose pubsub methods such as `eth_subscribe`. Use the schema-backed link for reads, and use `uxc subscribe start` directly for subscriptions.

For subscriptions, use a WebSocket Ethereum RPC provider that you have verified actually accepts `eth_subscribe`. Do not assume a public HTTPS host automatically guarantees the same `wss://` host is stable for pubsub.

## Authentication

The default public read provider used by this skill does not require authentication.

If a user later points the same workflow at a private Ethereum RPC provider, verify its auth model first before reusing this skill unchanged.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v ethereum-jsonrpc-cli`
   - If missing, create it:
     `uxc link ethereum-jsonrpc-cli https://ethereum-rpc.publicnode.com --schema-url https://raw.githubusercontent.com/ethereum/execution-apis/assembled-spec/refs-openrpc.json`
   - `ethereum-jsonrpc-cli -h`

2. Inspect operation schema first:
   - `ethereum-jsonrpc-cli eth_blockNumber -h`
   - `ethereum-jsonrpc-cli eth_getBlockByNumber -h`
   - `ethereum-jsonrpc-cli eth_getBalance -h`

3. Prefer chain and balance/block reads before deeper state queries:
   - `ethereum-jsonrpc-cli eth_chainId`
   - `ethereum-jsonrpc-cli eth_blockNumber`
   - `ethereum-jsonrpc-cli eth_getBalance Address=0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 Block=latest`

4. Execute with key/value or positional JSON:
   - key/value:
     `ethereum-jsonrpc-cli eth_getBalance Address=0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 Block=latest`
   - positional JSON:
     `ethereum-jsonrpc-cli eth_getBlockByNumber '["latest", false]'`
   - nested positional JSON:
     `ethereum-jsonrpc-cli eth_call '[{"to":"0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","data":"0x313ce567"},"latest"]'`

5. Use `uxc subscribe start` directly for pubsub streams:
   - `uxc subscribe start wss://<verified-ethereum-rpc-host> eth_subscribe '{"params":["newHeads"]}' --sink file:$HOME/.uxc/subscriptions/eth-new-heads.ndjson`
   - `uxc subscribe start wss://<verified-ethereum-rpc-host> eth_subscribe '{"params":["logs",{"address":"0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"}]}' --sink file:$HOME/.uxc/subscriptions/eth-logs.ndjson`
   - `uxc subscribe status <job_id>`
   - `uxc subscribe stop <job_id>`

## Recommended Read Operations

- `net_version`
- `eth_chainId`
- `eth_blockNumber`
- `eth_getBlockByNumber`
- `eth_getBalance`
- `eth_getLogs`
- `eth_call`

## Recommended Subscription Operations

- `eth_subscribe`

Subscription `params[0]` modes that are usually most useful:

- `newHeads`
- `logs`
- `newPendingTransactions`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Stay on read-only methods and `eth_subscribe` by default.
- The official execution OpenRPC schema drives read help and request execution, but it does not cover `eth_subscribe`; treat subscriptions as runtime-only flows, not schema-discovered operations.
- Do not call write/admin/debug/engine methods through this skill without explicit follow-up design and review.
- Use `uxc subscribe start` for pubsub methods; the fixed `ethereum-jsonrpc-cli` link is for normal request/response methods.
- Subscription jobs should always write to a sink file so events can be inspected and replayed safely.
- Before documenting or automating a public Ethereum pubsub host, confirm the specific provider actually exposes WebSocket JSON-RPC subscriptions; public HTTPS endpoints do not guarantee a matching `wss://` endpoint.
- `ethereum-jsonrpc-cli <operation> ...` is equivalent to `uxc https://ethereum-rpc.publicnode.com --schema-url <ethereum_execution_openrpc_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Ethereum execution API specs: https://github.com/ethereum/execution-apis
- Ethereum JSON-RPC overview: https://ethereum.org/developers/docs/apis/json-rpc/
