---
name: sui-jsonrpc-skill
description: Operate Sui public JSON-RPC through UXC with OpenRPC-driven discovery, mainnet fullnode defaults, and read-only safety guardrails.
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

This skill does **not** cover:

- `unsafe_*` transaction-building methods
- `sui_executeTransactionBlock`
- wallet signing flows
- WebSocket subscriptions such as `suix_subscribeEvent`
- custom/private Sui RPC providers with different auth or rate limits

## Endpoint And Discovery

This skill targets the public Sui fullnode endpoint:

- `https://fullnode.mainnet.sui.io`

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

## Recommended Read Operations

- `sui_getChainIdentifier`
- `sui_getLatestCheckpointSequenceNumber`
- `sui_getCheckpoint`
- `sui_getObject`
- `suix_getReferenceGasPrice`
- `suix_getLatestSuiSystemState`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Stay on the public read-only method surface by default.
- Do not call any `unsafe_*` methods through this skill without explicit follow-up design and review.
- Do not use this skill for write/sign/submit flows; those need separate wallet/auth guidance.
- Public RPC availability and rate limits can change over time; if discovery or execution starts failing, re-check the endpoint before assuming a `uxc` bug.
- `sui-jsonrpc-cli <operation> ...` is equivalent to `uxc https://fullnode.mainnet.sui.io <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Sui documentation: https://docs.sui.io/
