---
name: uxc
description: Discover and call remote schema-exposed interfaces with UXC. Use when an agent or skill needs to list operations, inspect operation schemas, and execute OpenAPI, GraphQL, gRPC, MCP, or JSON-RPC calls via one CLI contract.
metadata:
  short-description: Discover and call remote schema APIs via UXC
---

# UXC Skill

Use this skill when a task requires calling a remote interface and the endpoint can expose machine-readable schema metadata.

## When To Use

- You need to call APIs/tools from another skill and want one consistent CLI workflow.
- The interface may be OpenAPI, GraphQL, gRPC reflection, MCP, or JSON-RPC/OpenRPC.
- You need deterministic, machine-readable output (`ok`, `kind`, `data`, `error`).

Do not use this skill for pure local file operations with no remote interface.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- For gRPC runtime calls, `grpcurl` is installed and available in `PATH`.

### Install uxc

Choose one of the following methods:

**Homebrew (macOS/Linux):**
```bash
brew tap holon-run/homebrew-tap
brew install uxc
```

**Install Script (macOS/Linux, review before running):**
```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh -o install-uxc.sh
# Review the script before running it
less install-uxc.sh
bash install-uxc.sh
```

**Cargo:**
```bash
cargo install uxc
```

For more options, see the [Installation](https://github.com/holon-run/uxc#installation) section in the UXC README.

## Core Workflow

1. Discover operations:
   - `uxc <host> list`
2. Inspect a specific operation:
   - `uxc <host> describe <operation>`
   - or `uxc <host> <operation> help`
3. Execute with structured input:
   - `uxc <host> <operation> --input-json '<payload-json>'`
   - `uxc <host> <operation> '<payload-json>'`
   - `uxc <host> <operation> --args key=value`
4. Parse result as JSON envelope:
   - Success: `.ok == true`, consume `.data`
   - Failure: `.ok == false`, inspect `.error.code` and `.error.message`
5. If operation name conflicts with keywords such as `help`/`list`, use explicit form:
   - `uxc <host> call <operation> --input-json '<payload-json>'`

## Input Modes

- Bare JSON positional:
  - `uxc <host> <operation> '{"field":"value"}'`
- Explicit JSON flag:
  - `uxc <host> <operation> --input-json '{"field":"value"}'`
- Key/value arguments:
  - `uxc <host> <operation> field=value`
  - `uxc <host> <operation> --args field=value`

Do not pass both positional JSON and `--input-json` in one call.

## Migration Note

- `--json` has been removed. Use `--input-json` or a bare JSON positional payload.

## Output Contract For Reuse

Other skills should treat this skill as the interface execution layer and consume only the stable envelope:

- Success fields: `ok`, `kind`, `protocol`, `endpoint`, `operation`, `data`, `meta`
- Failure fields: `ok`, `error.code`, `error.message`, `meta`

Default output is JSON. Do not use `--text` in agent automation paths.

## Reuse Rule For Other Skills

- If a skill needs remote API/tool execution, reuse this skill instead of embedding protocol-specific calling logic.
- Upstream skill inputs should be limited to:
  - target host
  - operation id/name
  - JSON payload
  - required fields to extract from `.data`

## Reference Files (Load On Demand)

- Workflow details and progressive invocation patterns:
  - `references/usage-patterns.md`
- Protocol operation naming quick reference:
  - `references/protocol-cheatsheet.md`
- Public endpoint examples and availability notes:
  - `references/public-endpoints.md`
- Failure handling and retry strategy:
  - `references/error-handling.md`
