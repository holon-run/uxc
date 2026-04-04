# UXC

**One CLI for tools across protocols**

English | [简体中文](README.zh-CN.md)

[Documentation](https://uxc.holon.run) | [Skills Directory](https://uxc.holon.run/skills/)

[![CI](https://github.com/holon-run/uxc/workflows/CI/badge.svg)](https://github.com/holon-run/uxc/actions)
[![Coverage](https://github.com/holon-run/uxc/workflows/Coverage/badge.svg)](https://github.com/holon-run/uxc/actions/workflows/coverage.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)

UXC helps agents and automation discover and invoke APIs and tools across
OpenAPI, MCP, GraphQL, gRPC, and JSON-RPC through one consistent workflow.

From discovery to structured invocation, UXC aims to keep calling patterns
consistent across protocols while handling auth, output formatting, and
protocol-specific differences behind the scenes.

## Start Here

Most flows follow the same path:

```bash
uxc <host> -h
uxc <host> <operation_id> -h
uxc <host> <operation_id> key=value
```

Argument styles:

```bash
# nested fields
uxc <host> <operation_id> filter.status=active items[0].id=1

# one field as JSON
uxc <host> <operation_id> filter:='{"status":"active"}' tags:='["rust","cli"]'

# full payload
uxc <host> <operation_id> '{"filter":{"status":"active"}}'
```

Example:

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

## Why UXC

Remote capabilities are easy to access in isolation, but hard to reuse
consistently across systems.

Common friction points:

- each protocol has its own discovery and invocation style
- auth setup is scattered across shell scripts, prompts, SDKs, and local config
- tool and schema details are hard to inspect before execution
- automation breaks when output shape and error handling differ by provider
- local agent workflows do not want one-off wrappers for every service

UXC exists to turn schema-described remote capabilities into one reusable CLI
entrypoint for agents, skills, scripts, and local applications.

## How It Works

UXC keeps the top-level interaction model simple:

1. Discover what a host exposes.
2. Inspect the shape of a specific operation.
3. Invoke with structured arguments.
4. Reuse the same calling pattern across protocols.

This makes remote interfaces feel more like a stable command surface than a
collection of protocol-specific request styles.

## Why Not curl, SDKs, or MCP-Only Tooling

### `curl`

`curl` is request-first. It is excellent when the caller already knows the URL,
method, headers, and payload shape.

UXC is discovery-first. It helps callers inspect what is available, understand
expected input shape, and invoke with a more stable command contract.

### Provider SDKs or protocol-specific CLIs

Provider SDKs can be deep and powerful, but each one introduces its own usage
model, auth conventions, and output shape.

UXC trades provider-specific ergonomics for a shared interface across many
providers and protocol families.

### MCP-only tool calling

MCP is an important part of the ecosystem, but many useful systems are exposed
through OpenAPI, GraphQL, gRPC, or JSON-RPC instead.

UXC is designed to unify MCP with those adjacent protocol surfaces rather than
stopping at MCP-only workflows.

## What You Get

- help-first discovery with `<host> -h` and `<host> <operation_id> -h`
- structured invocation with key/value args, path-style nested args, `:=` per-field JSON, or positional JSON payloads
- deterministic JSON output by default, with opt-in text mode
- reusable auth credentials and endpoint bindings
- shortcut links for frequently used hosts
- MCP-first config import from common client/editor configs
- daemon-backed session reuse and background subscriptions
- a TypeScript daemon client for local integrations

## Protocol Coverage

UXC currently supports these protocol families behind one CLI contract:

- OpenAPI / Swagger
- MCP over HTTP and stdio
- GraphQL introspection and execution
- gRPC reflection-based discovery and unary invocation
- JSON-RPC with OpenRPC-style discovery

Related runtime support also includes:

- daemon-backed subscription lifecycle
- WebSocket-based subscription flows
- polling-based subscriptions
- provider-aware event intake for Slack, Discord, Feishu, and similar systems

## Auth Coverage

UXC is intended for more than public demo endpoints. It includes reusable auth
and binding primitives for real provider integrations.

Supported auth patterns include:

- bearer tokens
- API keys with configurable header or query placement
- multi-field credentials for signed APIs
- signer-backed request generation
- OAuth for supported MCP HTTP flows
- secret sources from literal values, environment variables, or external secret
  providers

The main auth model is:

- credentials store auth material
- bindings match endpoints and select which credential applies

That keeps auth setup reusable instead of embedding secrets and rules into every
individual command.

## Skills and Integrations

UXC is not only a CLI entrypoint. This repository also ships a growing set of
official skills built on top of the shared execution layer.

Representative categories include:

- browser and local tooling: `playwright-mcp-skill`, `chrome-devtools-mcp-skill`
- documentation and research: `context7-mcp-skill`, `deepwiki-mcp-skill`
- workspace and messaging: `notion-*`, `slack-*`, `discord-*`, `telegram-*`
- crypto and market data: `dune-*`, `etherscan-*`, `thegraph-*`, `coinmarketcap-*`

Use the base `uxc` skill as the shared execution layer, then add wrapper skills
when a service-specific workflow is worth packaging.

See [the Skills Directory](https://uxc.holon.run/skills/) for the full skill catalog and
[`docs/operations/skills.md`](docs/operations/skills.md) for publish and maintenance logs.

## Where It Fits

UXC is a good fit for:

- agent and skill authors who need one stable way to call many remote systems
- automation and scripts that need structured output and predictable failure modes
- local applications that want daemon-backed reuse instead of parsing CLI stdout
- multi-provider workflows where auth and invocation patterns would otherwise drift

UXC is not:

- a hosted platform
- an API gateway
- a replacement for every provider SDK
- a full bot framework or workflow orchestration system

## Install

### Homebrew (macOS/Linux)

```bash
brew tap holon-run/homebrew-tap
brew install uxc
```

### Install Script (macOS/Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash
```

Review before running:

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh -o install-uxc.sh
less install-uxc.sh
bash install-uxc.sh
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash -s -- -v v0.13.0
```

Windows note: native Windows is no longer supported; run UXC through WSL.

### Cargo

```bash
cargo install uxc
```

### From Source

```bash
git clone https://github.com/holon-run/uxc.git
cd uxc
cargo install --path .
```

## Quick Examples

### OpenAPI

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

### GraphQL

```bash
uxc countries.trevorblades.com -h
uxc countries.trevorblades.com query/country code=US
```

### MCP

```bash
uxc mcp.deepwiki.com/mcp -h
uxc mcp.deepwiki.com/mcp ask_question '{"repoName":"holon-run/uxc","question":"What does this project do?"}'
```

### JSON-RPC

```bash
uxc fullnode.mainnet.sui.io -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber
```

## Output and Help Conventions

UXC is JSON-first by default.
Use `--text` or `--format text` for human-readable CLI output.

Success responses return a stable JSON envelope with fields such as:

- `ok`
- `kind`
- `protocol`
- `endpoint`
- `operation`
- `data`
- `meta`

This makes UXC easier to consume from agents, scripts, and local applications.

## TypeScript Daemon Client

For local app integration, UXC also ships an official Node/TypeScript client:

```bash
npm install @holon-run/uxc-daemon-client
```

It connects directly to the local daemon socket and returns structured objects
instead of CLI stdout envelopes.

Use it when embedding UXC into applications that need runtime calls, daemon
status, or subscription lifecycle and event streaming.

For generated runtime clients (#334), you can export host-scoped codegen input
from CLI and generate a typed TypeScript client through the daemon client:

```bash
uxc <host> --codegen-schema
```

```ts
import { UxcDaemonClient } from "@holon-run/uxc-daemon-client";

const runtime = new UxcDaemonClient();
const source = await runtime.generateTypeScriptClient({
  endpoint: "https://petstore3.swagger.io/api/v3",
  options: { no_cache: true },
  emitter: { className: "PetstoreClient" },
});
```

See [the daemon API docs](https://uxc.holon.run/daemon/api/) for the daemon contract.

## Docs Map

- getting started: [https://uxc.holon.run/getting-started/](https://uxc.holon.run/getting-started/)
- public no-key endpoints: [https://uxc.holon.run/reference/public-endpoints/](https://uxc.holon.run/reference/public-endpoints/)
- auth secret sources: [https://uxc.holon.run/auth/secret-sources/](https://uxc.holon.run/auth/secret-sources/)
- MCP HTTP OAuth: [https://uxc.holon.run/auth/oauth-mcp-http/](https://uxc.holon.run/auth/oauth-mcp-http/)
- daemon service setup: [https://uxc.holon.run/daemon/service/](https://uxc.holon.run/daemon/service/)
- daemon API and TypeScript client: [https://uxc.holon.run/daemon/api/](https://uxc.holon.run/daemon/api/)
- logging and troubleshooting: [https://uxc.holon.run/daemon/logging/](https://uxc.holon.run/daemon/logging/)
- schema mapping and `--schema-url`: [https://uxc.holon.run/reference/schema-mapping/](https://uxc.holon.run/reference/schema-mapping/)
- skills catalog: [https://uxc.holon.run/skills/](https://uxc.holon.run/skills/)
- skills publish and maintenance log: [`docs/operations/skills.md`](docs/operations/skills.md)
- release process: [`docs/operations/release.md`](docs/operations/release.md)

## Contributing

Contributions are welcome.

- development workflow: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- CI and releases: [GitHub Actions](https://github.com/holon-run/uxc/actions)

## License

MIT License. See [`LICENSE`](LICENSE).
