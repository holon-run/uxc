---
name: sui-jsonrpc-skill
description: Operate Sui public JSON-RPC through UXC with OpenRPC-driven discovery, mainnet fullnode defaults, and read-only query plus pubsub subscription guardrails.
---

# Sui JSON-RPC Skill

Use this skill to run Sui JSON-RPC operations through `uxc` + JSON-RPC.

Reuse the `uxc` skill for shared execution and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://fullnode.mainnet.sui.io`.
- No API key is required for the public mainnet fullnode in this skill's default flow.

## Scope

This skill covers a safe read-first Sui JSON-RPC surface:

- chain identity and latest checkpoint reads
- checkpoint lookup
- object lookup
- reference gas price reads
- latest system state reads
- pubsub subscriptions for events and transaction effects

This skill does **not** cover:

- `unsafe_*` transaction-building methods
- `sui_executeTransactionBlock`
- wallet signing flows
- custom/private Sui RPC providers with different auth or rate limits

## Endpoint And Discovery

This skill targets the public Sui fullnode endpoint:

- `https://fullnode.mainnet.sui.io`

For pubsub, use a Sui provider WebSocket endpoint that you have verified actually accepts JSON-RPC subscriptions. Do not assume the public HTTPS fullnode host automatically supports the same `wss://` hostname for pubsub.

`uxc` JSON-RPC discovery depends on OpenRPC or `rpc.discover`. Sui exposes a discoverable method surface, so help-first flow works directly against the endpoint.

## Authentication

The default public endpoint used by this skill does not require authentication.

If a user later points the same workflow at a private Sui RPC provider, verify its auth model first before reusing this skill unchanged.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v sui-jsonrpc-cli`
   - If missing, create it:
     `uxc link sui-jsonrpc-cli https://fullnode.mainnet.sui.io`
   - `sui-jsonrpc-cli -h`

2. Inspect operation schema first:
   - `sui-jsonrpc-cli sui_getLatestCheckpointSequenceNumber -h`
   - `sui-jsonrpc-cli sui_getCheckpoint -h`
   - `sui-jsonrpc-cli sui_getObject -h`

3. Prefer read/setup validation before any deeper query:
   - `sui-jsonrpc-cli sui_getChainIdentifier`
   - `sui-jsonrpc-cli sui_getLatestCheckpointSequenceNumber`
   - `sui-jsonrpc-cli suix_getReferenceGasPrice`

4. Execute with key/value or positional JSON:
   - key/value:
     `sui-jsonrpc-cli sui_getCheckpoint id=254502592`
   - positional JSON:
     `sui-jsonrpc-cli sui_getObject '{"object_id":"0x6"}'`

5. Use `uxc subscribe start` directly for pubsub streams:
   - `uxc subscribe start wss://<verified-sui-rpc-host> suix_subscribeEvent '{"params":[{"Package":"0x2"}]}' --sink file:$HOME/.uxc/subscriptions/sui-events.ndjson`
   - `uxc subscribe start wss://<verified-sui-rpc-host> suix_subscribeTransaction '{"params":[{"FromAddress":"0x..."}]}' --sink file:$HOME/.uxc/subscriptions/sui-transactions.ndjson`
   - `uxc subscribe status <job_id>`
   - `uxc subscribe stop <job_id>`

## Recommended Read Operations

- `sui_getChainIdentifier`
- `sui_getLatestCheckpointSequenceNumber`
- `sui_getCheckpoint`
- `sui_getObject`
- `suix_getReferenceGasPrice`
- `suix_getLatestSuiSystemState`

## Recommended Subscription Operations

- `suix_subscribeEvent`
- `suix_subscribeTransaction`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Stay on the public read-only method surface by default.
- Do not call any `unsafe_*` methods through this skill without explicit follow-up design and review.
- Do not use this skill for write/sign/submit flows; those need separate wallet/auth guidance.
- Use `uxc subscribe start` for pubsub methods; the fixed `sui-jsonrpc-cli` link is for normal request/response methods.
- Subscription jobs should always write to a sink file so events can be inspected and replayed safely.
- Before documenting or automating a Sui pubsub host, confirm the specific provider actually exposes JSON-RPC WebSocket subscriptions; public HTTPS endpoints do not guarantee a matching `wss://` endpoint.
- Public RPC availability and rate limits can change over time; if discovery or execution starts failing, re-check the endpoint before assuming a `uxc` bug.
- `sui-jsonrpc-cli <operation> ...` is equivalent to `uxc https://fullnode.mainnet.sui.io <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Sui documentation: https://docs.sui.io/
