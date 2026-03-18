---
name: near-jsonrpc-skill
description: Operate NEAR JSON-RPC reads through UXC with a public provider default, provider-override guidance, and read-only guardrails.
---

# NEAR JSON-RPC Skill

Use this skill to run NEAR JSON-RPC operations through `uxc` + JSON-RPC.

Reuse the `uxc` skill for shared execution and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to a working NEAR RPC provider.
- This skill defaults to `https://free.rpc.fastnear.com`, which is listed on the official NEAR RPC providers page.
- Access to the curated NEAR OpenRPC schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json`

## Scope

This skill covers a safe read-first NEAR RPC surface:

- chain status
- account state query
- finalized block lookup
- chunk lookup by chunk hash
- gas price lookup
- validator set lookup

This skill does **not** cover:

- transaction submission methods
- signing or wallet flows
- archival assumptions for very old blocks or chunks
- deprecated `near.org` or `pagoda.co` public RPC endpoints

## Endpoint And Schema

This skill uses the public FastNear mainnet RPC by default:

- `https://free.rpc.fastnear.com`

The operation surface comes from the curated NEAR OpenRPC schema in this repo:

- `https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json`

`uxc` JSON-RPC discovery depends on OpenRPC or `rpc.discover`. FastNear does not expose a discoverable method surface that UXC can consume directly, so this skill uses a fixed `--schema-url` link and request flow.

The official NEAR docs now treat `near.org` and `pagoda.co` RPC endpoints as deprecated. Do not use those old hosts as the default for new automation.

If the user already has a preferred provider from the official NEAR providers page, relink the same command to that provider instead of reusing the deprecated endpoints.

## Authentication

The default FastNear public RPC used by this skill does not require authentication.

If the user switches to a private NEAR provider, verify its auth model before reusing this skill unchanged.

## Core Workflow

1. Use the fixed link command by default:
   - `command -v near-jsonrpc-cli`
   - If missing, create it:
     `uxc link near-jsonrpc-cli https://free.rpc.fastnear.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json`
   - `near-jsonrpc-cli -h`

2. Inspect operation help first, then run known read methods:
   - `near-jsonrpc-cli -h`
   - `near-jsonrpc-cli query -h`
   - `near-jsonrpc-cli gas_price -h`
   - `near-jsonrpc-cli status`
   - `near-jsonrpc-cli gas_price --input-json '{"block_id":null}'`

3. Prefer narrow validation before deeper queries:
   - `near-jsonrpc-cli status`
   - `near-jsonrpc-cli block '{"finality":"final"}'`
   - `near-jsonrpc-cli query '{"request_type":"view_account","finality":"final","account_id":"near"}'`

4. Use object JSON for request objects, and `--input-json` when a method expects positional null/default params:
   - object request:
     `near-jsonrpc-cli chunk '{"chunk_id":"75cewvnKFLrJshoUft1tiUC9GriuxWTc4bWezjy2MoPR"}'`
   - positional null/default encoded from an object payload:
     `near-jsonrpc-cli gas_price --input-json '{"block_id":null}'`
     `near-jsonrpc-cli validators --input-json '{"epoch_reference":null}'`

## Recommended Read Operations

- `status`
- `query`
- `block`
- `chunk`
- `gas_price`
- `validators`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Stay on the read-only method surface by default.
- The fixed schema is required because FastNear is not directly discoverable by UXC; do not drop `--schema-url` from the linked command unless the provider proves it exposes usable OpenRPC or `rpc.discover`.
- Do not use this skill for transaction submission, signing, or wallet-authenticated flows.
- The official NEAR docs mark the old `near.org` and `pagoda.co` public RPC endpoints as deprecated. Prefer providers from the official RPC providers page instead.
- Public providers can differ in archival retention and rate limits. If `chunk` or older `block` lookups fail with unknown or garbage-collected errors, switch to a provider that explicitly supports the needed history.
- For methods such as `gas_price` and `validators` that expect positional params, use `--input-json '{"...":null}'` instead of array payloads; UXC CLI positional JSON accepts objects, not arrays.
- `near-jsonrpc-cli <operation> ...` is equivalent to `uxc https://free.rpc.fastnear.com --schema-url <near_openrpc_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenRPC schema: `references/near-public.openrpc.json`
- NEAR RPC introduction: https://docs.near.org/api/rpc/introduction
- NEAR RPC providers: https://docs.near.org/api/rpc/providers
