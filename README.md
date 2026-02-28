# UXC

**Universal X-Protocol Call**

[![CI](https://github.com/holon-run/uxc/workflows/CI/badge.svg)](https://github.com/holon-run/uxc/actions)
[![Coverage](https://github.com/holon-run/uxc/workflows/Coverage/badge.svg)](https://github.com/holon-run/uxc/actions/workflows/coverage.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)

UXC is a universal API calling CLI that lets you discover and invoke OpenAPI, gRPC, GraphQL,
MCP, and JSON-RPC interfaces directly from a URL.

It turns remote schema-exposed interfaces into executable command-line operations without SDKs,
code generation, or endpoint pre-registration.

## What Is UXC

Modern services increasingly expose machine-readable interface metadata.
UXC treats those schemas as runtime execution contracts:

- Discover operations from a host
- Inspect operation inputs/outputs
- Execute operations with structured input
- Return deterministic JSON envelopes by default

If a target can describe itself, UXC can usually call it.

## Why It Exists

Teams and agents often need to interact with many protocol styles:
OpenAPI, GraphQL, gRPC, MCP, and JSON-RPC.

Traditional workflows create repeated overhead:

- language-specific SDK setup
- generated clients that drift from server reality
- one-off wrappers for each endpoint
- large embedded tool schemas in agent prompts

UXC provides one URL-first CLI contract across protocols.

## Why UXC Works Well With Skills

UXC is a practical fit for skill-based agents:

- On-demand discovery and invocation, without preloading large MCP tool definitions into prompt context
- Portable by endpoint URL and auth binding, not tied to per-user local MCP server names
- Reusable as one shared calling interface across many skills

## Core Capabilities

- URL-first usage: call endpoints directly, no server alias required
- Multi-protocol detection and adapter routing
- Schema-driven operation discovery (`<host> -h`, `<host> <operation_id> -h`)
- Structured invocation (positional JSON, key-value args)
- Deterministic JSON envelopes for automation and agents
- Auth model with reusable credentials and endpoint bindings
- Host shortcut commands via `uxc link`

Supported protocols:

- OpenAPI / Swagger
- gRPC (server reflection)
- GraphQL (introspection)
- MCP (HTTP and stdio)
- JSON-RPC (OpenRPC-based discovery)

## Architecture Snapshot

UXC keeps protocol diversity behind one execution contract:

```text
User / Skill / Agent
        ↓
      UXC CLI
        ↓
 Protocol Detector
        ↓
   Adapter Layer
 (OpenAPI/gRPC/GraphQL/MCP/JSON-RPC)
        ↓
  Remote Endpoint
```

This design keeps invocation UX stable while allowing protocol-specific internals.

## Target Use Cases

- AI agents and skills that need deterministic remote tool execution
- CI/CD and automation jobs that need schema-driven calls without SDK setup
- Cross-protocol integration testing with one command contract
- Controlled runtime environments where JSON envelopes and predictable errors matter

## Non-Goals

UXC is not:

- a code generator
- an SDK framework
- an API gateway or reverse proxy

UXC is an execution interface for schema-exposed remote capabilities.

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
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash -s -- -v v0.4.0
```

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

## Quickstart (3 Minutes)

Most HTTP examples omit the scheme for brevity.
For public hosts, UXC infers `https://` when omitted.

1. Discover operations:

```bash
uxc petstore3.swagger.io/api/v3 -h
```

2. Inspect operation schema:

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
```

3. Execute with structured input:

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

Use only these endpoint forms:
- `uxc <host> -h`
- `uxc <host> <operation_id> -h`
- `uxc <host> <operation_id> key=value` or `uxc <host> <operation_id> '{...}'`

## Protocol Examples (One Each)

Operation ID conventions:

- OpenAPI: `method:/path` (example: `get:/users/{id}`)
- gRPC: `Service/Method`
- GraphQL: `query/viewer`, `mutation/createUser`
- MCP: tool name (example: `ask_question`)
- JSON-RPC: method name (example: `eth_getBalance`)

### OpenAPI

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

For schema-separated services, you can override schema source:

```bash
uxc api.github.com -h \
  --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json
```

### gRPC

```bash
uxc grpcb.in:9000 -h
uxc grpcb.in:9000 addsvc.Add/Sum a=1 b=2
```

Note: gRPC unary runtime invocation requires `grpcurl` on `PATH`.

### GraphQL

```bash
uxc countries.trevorblades.com -h
uxc countries.trevorblades.com query/country code=US
```

### MCP

```bash
uxc mcp.deepwiki.com/mcp -h
uxc mcp.deepwiki.com/mcp ask_question repoName=holon-run/uxc question='What does this project do?'
```

### JSON-RPC

```bash
uxc fullnode.mainnet.sui.io -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber
```

## Skills

UXC provides one canonical skill plus scenario-specific official wrappers.
Use `uxc` skill as the shared execution layer, and add wrappers when they fit your workflow.

| Skill | Purpose | Path |
| --- | --- | --- |
| `uxc` | Canonical schema discovery and multi-protocol execution layer | [`skills/uxc/SKILL.md`](skills/uxc/SKILL.md) |
| `deepwiki` | Query repository documentation and ask codebase questions | [`skills/deepwiki/SKILL.md`](skills/deepwiki/SKILL.md) |
| `context7` | Query up-to-date library documentation/examples over MCP | [`skills/context7/SKILL.md`](skills/context7/SKILL.md) |
| `notion-mcp-skill` | Operate Notion MCP workflows with OAuth-aware guidance | [`skills/notion-mcp-skill/SKILL.md`](skills/notion-mcp-skill/SKILL.md) |

See [`docs/skills.md`](docs/skills.md) for install methods and maintenance rules.

## Output and Help Conventions

UXC is JSON-first by default.
Use `--text` (or `--format text`) when you want human-readable CLI output.

Examples:

```bash
uxc
uxc help
uxc <host> -h
uxc <host> <operation_id> -h
uxc --text help
```

Note: In endpoint routing, `help` is treated as a literal operation name, not a help alias.

Success envelope shape:

```json
{
  "ok": true,
  "kind": "call_result",
  "protocol": "openapi",
  "endpoint": "https://petstore3.swagger.io/api/v3",
  "operation": "get:/pet/{petId}",
  "data": {},
  "meta": {
    "version": "v1",
    "duration_ms": 128
  }
}
```

Failure envelope shape:

```json
{
  "ok": false,
  "error": {
    "code": "INVALID_ARGUMENT",
    "message": "Field 'id' must be an integer"
  },
  "meta": {
    "version": "v1"
  }
}
```

## Auth (Credentials + Bindings)

UXC authentication has two resources:

- Credentials: secret material and auth type
- Bindings: endpoint matching rules that select a credential

Example:

```bash
uxc auth credential set deepwiki --auth-type bearer --secret-env DEEPWIKI_TOKEN
uxc auth binding add --id deepwiki-mcp --host mcp.deepwiki.com --path-prefix /mcp --scheme https --credential deepwiki --priority 100
```

OAuth for MCP HTTP is supported (device code, client credentials, authorization code + PKCE).
See [`docs/oauth-mcp-http.md`](docs/oauth-mcp-http.md) for full workflows.

## Docs Map

- Extended quickstart and protocol walkthroughs: [`docs/quickstart.md`](docs/quickstart.md)
- Public no-key endpoints for protocol checks: [`docs/public-endpoints.md`](docs/public-endpoints.md)
- Logging and troubleshooting with `RUST_LOG`: [`docs/logging.md`](docs/logging.md)
- OpenAPI schema mapping and `--schema-url`: [`docs/schema-mapping.md`](docs/schema-mapping.md)
- Skills overview and install/maintenance guidance: [`docs/skills.md`](docs/skills.md)
- Release process: [`docs/release.md`](docs/release.md)

## Contributing

Contributions are welcome.

- Development workflow and quality bar: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- CI and release flows: [GitHub Actions](https://github.com/holon-run/uxc/actions)

## License

MIT License - see [`LICENSE`](LICENSE).
