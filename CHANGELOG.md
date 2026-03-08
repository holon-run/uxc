# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0] - 2026-03-08

### Added
- Added API key query-parameter injection with `--query-param "<name>=<template>"` for HTTP-based protocols, enabling clean credential-based access to services such as Flipside MCP without embedding secrets in endpoint URLs.
- Added `bitquery-graphql-skill`.

### Changed
- CLI now detects daemon/CLI version mismatches after upgrades and automatically restarts the local daemon before continuing daemon-backed requests.

### Fixed
- Prevent stale daemon behavior after CLI upgrades by surfacing version mismatches and replacing the running daemon automatically.

## [0.9.0] - 2026-03-07

### Added
- Added schema-aware argument coercion and validation across protocols so `key=value` and positional JSON inputs are converted and checked against operation schemas before execution.
- Added legacy SSE MCP transport support.
- Added stdio child-process credential injection with `--inject-env NAME={{secret}}` and `uxc link --credential <credential_id>`.
- Added `thegraph-mcp-skill`, `thegraph-token-mcp-skill`, `dune-mcp-skill`, and `etherscan-mcp-skill`.

### Changed
- Stdio command parsing now supports single-quoted segments in command strings.
- Skill credential guidance now emphasizes agent-first setup for stdio MCP usage.

## [0.8.0] - 2026-03-06

### Added
- OAuth auth is now supported across OpenAPI, GraphQL, JSON-RPC, and gRPC in addition to MCP HTTP.
- Added non-interactive OAuth authorization-code flow with `uxc auth oauth start` and `uxc auth oauth complete` for agent-friendly two-step login.
- Added `uxc link --schema-url` so generated shortcuts can persist a default OpenAPI schema URL while still allowing runtime override.
- Added `uxc daemon restart` to simplify daemon lifecycle management.
- Added cache inspection and targeted invalidation with `uxc cache list` and `uxc cache clear --key`.
- Added `linear-mcp-skill` and expanded published skill coverage/documentation, including bilingual README updates and install guidance for `npx` and ClawHub.

### Changed
- Standardized skill naming around protocol-oriented MCP skill names such as `context7-mcp-skill` and `deepwiki-mcp-skill`.
- Discord/OpenAPI skill guidance now prefers Bot Token for primary auth and documents schema-persisted link usage.

### Fixed
- Repaired GraphQL mutation input object handling for nested mutation inputs.
- Stabilized MCP HTTP auth-required coverage flow in tests.

### CI
- Added skills validation and manual skill publish workflows.

## [0.7.1] - 2026-03-04

### Fixed
- Stabilized local E2E MCP tests by isolating test `HOME` so daemon state/socket conflicts from external runs do not cause release CI flakes.
- Kept test `HOME` path short (`/tmp/...`) to avoid Unix domain socket path length failures when daemon starts in tests.

## [0.7.0] - 2026-03-04

### Added
- MCP `call_result` output now includes `structuredContent` to preserve structured tool responses for downstream agents.
- Unified `okx-mcp-skill` with trial-key guidance and reusable usage patterns for market/onchain workflows.

### Changed
- API key auth now supports configurable header names (for example `OK-ACCESS-KEY`) instead of only `x-api-key`.
- Skill catalog/docs updated with renamed MCP wrapper skills and publish status notes.

## [0.6.0] - 2026-03-03

### Added
- Daemon exclusive state keys for MCP stdio session hand-off (`--daemon-exclusive` and `UXC_DAEMON_EXCLUSIVE`) to support safe process reuse/eviction across endpoints that share runtime state.
- `uxc link` support for persisting daemon exclusive keys in generated launchers.

### Changed
- Help/inference global-arg normalization now treats `--daemon-exclusive` as a true global option across dynamic forms.
- `UXC_DAEMON_EXCLUSIVE` parsing now avoids `C:\...` ambiguity on Windows by not using `:` splitting on native Windows.
- `~\\...` is now expanded for daemon exclusive keys in addition to `~/...`.

### Fixed
- Improve daemon busy-conflict diagnostics for exclusive keys with redacted owner details.
- Allow stale schema cache fallback in help flow when runtime invocation fails, improving resilience for degraded/offline targets.

### Removed
- Official Windows native support; UXC should be run through WSL on Windows hosts.

## [0.5.3] - 2026-03-02

> Note: `v0.5.0`, `v0.5.1`, and `v0.5.2` were intermediate tags that were not released.
> `v0.5.3` is the first published `0.5.x` release and includes all changes listed below.

### Added
- `uxcd` runtime daemon with auto-start endpoint execution path and MCP session reuse (stdio + HTTP).
- Daemon troubleshooting logs with basic rotation for easier local diagnostics.
- Playwright MCP wrapper skill and validation script to exercise stdio-based MCP usage.
- Expanded integration coverage for offline cache fallback, daemon reuse, and daemon logging.

### Changed
- Prefer schema cache-first resolution across protocols for help and execution paths to improve reliability.
- Authentication model now supports a dual-track approach: local convenience storage plus external secret sources (for example `env`/`op`) for advanced users.

### Fixed
- Improve daemon idle cleanup to avoid global lock stalls under load.
- Add MCP stdio request timeout to prevent indefinite hangs.
- Stabilize MCP stdio framing/transport behavior for large payloads.
- Fix Windows release builds by fully gating Unix domain socket usage behind `cfg(unix)`.

### Documentation
- Update Playwright MCP skill guidance for shared profile usage.

## [0.4.2] - 2026-02-28

### Changed
- Rename product title from "Universal X-Protocol Call" to "Universal X-Protocol CLI" across CLI help/about text, package metadata, Homebrew formula description, and docs.

## [0.4.1] - 2026-02-28

### Fixed
- Use a relative `.claude/skills` symlink (`../skills`) so `cargo publish` can archive the package in CI environments.

## [0.4.0] - 2026-02-28

### Changed
- Endpoint CLI interaction is now single-path and help-first:
  - `uxc <host> -h`
  - `uxc <host> <operation_id> -h`
  - `uxc <host> <operation_id> key=value | '{...}'`

### Removed
- Legacy endpoint command forms have been removed:
  - `uxc <host> list`
  - `uxc <host> describe <operation_id>`
  - `uxc <host> call <operation_id> ...`
  - `uxc <host> inspect`
- Endpoint `help` word alias is removed; `help` is treated as a literal operation name in endpoint routing.

## [0.3.0] - 2026-02-27

### Added
- `host_help` now includes MCP service metadata to improve tool discovery context for agents
- Added and refined Notion MCP skill workflows with reusable OAuth/binding guidance

### Changed
- CLI payload input is standardized on `--input-json` (with optional positional JSON object)
- Help commands are unified to JSON output (`uxc`, `uxc help`, `uxc <host> help`, and subcommand help)
- Help guidance now uses `examples` instead of `data.next` for follow-up commands

### Fixed
- MCP HTTP probing now attempts OAuth refresh before protocol fallback, reducing false negatives
- HTTP client construction now guards proxy edge cases with `no_proxy` fallback handling
- Homebrew tap update script now uses token-based authenticated push

## [0.2.0] - 2026-02-27

### Added
- OAuth `authorization_code` + PKCE login flow for MCP HTTP (`uxc auth oauth login --flow authorization_code`)
- OAuth discovery fallback via `/.well-known/oauth-protected-resource` when `WWW-Authenticate` metadata is missing

### Changed
- Authentication model refactored to credential + binding storage in JSON files:
  - `~/.uxc/credentials.json`
  - `~/.uxc/auth_bindings.json`
- Auth CLI redesigned around credential/binding operations (`uxc auth credential ...`, `uxc auth binding ...`)

### Fixed
- MCP OAuth compatibility improvements for real providers (device polling and discovery behavior)
- OpenAPI GitHub `GET /user` execution decode handling
- Local E2E/contract test coverage and stability improvements across protocols

## [0.1.1] - 2026-02-25

### Fixed
- `call --help` no longer conflicts with clap auto-help; operation help uses `--op-help`
- CLI failures now return structured JSON error envelope
- gRPC detection no longer treats common ports as implicit gRPC
- gRPC `execute` no longer returns placeholder payload
- OpenAPI fetch now reuses discovered schema endpoint (`/swagger.json`, `/api-docs`, etc.)
- MCP stdio request/response correlation restored
- MCP HTTP endpoint discovery now probes host-level endpoints
- Auth integration tests now isolate `HOME` mutations with a process-wide lock

### Changed
- Enabled HTTPS support for HTTP-based adapters via `reqwest` + `rustls-tls`

## [0.1.0] - 2026-02-23

### Added

#### Authentication Profiles
- Multiple authentication profile storage with `uxc auth set` command
- Support for Bearer token authentication
- Support for API key authentication (X-API-Key header)
- Support for Basic HTTP authentication
- Profile management commands: `list`, `set`, `remove`, `info`
- `--profile` CLI flag for selecting profiles
- `UXC_PROFILE` environment variable support
- Profile selection precedence: CLI flag > env var > "default"
- API key masking in sensitive outputs

#### Protocol Support
- OpenAPI/Swagger specification support with full HTTP method coverage
- GraphQL API support with introspection and query execution
- gRPC service support with server reflection
- MCP (Model Context Protocol) server support with stdio and HTTP transports

#### CLI Features
- `uxc <url> list` - List available operations for any protocol
- `uxc <url> call <operation>` - Execute operations with parameters
- `uxc <url> inspect` - Inspect endpoint schema and capabilities
- `uxc auth` commands - Manage authentication profiles
- `uxc cache stats|clear` - View and clear schema cache
- JSON output envelope for `call` success/failure
- Schema caching with configurable TTL
- Cache configuration via `--cache-ttl` flag

#### Developer Experience
- Automatic protocol detection from URLs
- Built-in schema caching to reduce network calls
- Comprehensive error messages

#### Configuration
- Profile storage in `~/.uxc/profiles.toml`
- Schema cache in `~/.uxc/cache/`
- Environment variable support for all major settings
- TOML-based configuration format

### Security
- Input validation for profile names
- API key masking in logs and outputs
- Secure profile storage (non-encrypted in v0.1.0, encryption planned for v0.2.0)

### Technical
- Built with Rust 2021 edition
- Async runtime powered by Tokio 1.35
- Zero-copy parsing where possible
- Cross-platform support (Linux, macOS, Windows)

### Known Limitations
- gRPC invocation currently supports unary calls only
- gRPC runtime calls require `grpcurl` binary on PATH
- Profile encryption not implemented (planned for v0.2.0, see Issue #29)
- No per-endpoint profile configuration yet

### Documentation
- Comprehensive help text for all commands
- Usage examples in command descriptions
- Clear error messages with suggestions

## [0.0.1] - Initial Release

### Added
- Initial project structure
- Basic CLI framework
- Protocol detection infrastructure

---

[Unreleased]: https://github.com/holon-run/uxc/compare/v0.10.0...HEAD
[0.10.0]: https://github.com/holon-run/uxc/releases/tag/v0.10.0
[0.9.0]: https://github.com/holon-run/uxc/releases/tag/v0.9.0
[0.8.0]: https://github.com/holon-run/uxc/releases/tag/v0.8.0
[0.7.1]: https://github.com/holon-run/uxc/releases/tag/v0.7.1
[0.7.0]: https://github.com/holon-run/uxc/releases/tag/v0.7.0
[0.6.0]: https://github.com/holon-run/uxc/releases/tag/v0.6.0
[0.5.3]: https://github.com/holon-run/uxc/releases/tag/v0.5.3
[0.4.2]: https://github.com/holon-run/uxc/releases/tag/v0.4.2
[0.4.1]: https://github.com/holon-run/uxc/releases/tag/v0.4.1
[0.4.0]: https://github.com/holon-run/uxc/releases/tag/v0.4.0
[0.3.0]: https://github.com/holon-run/uxc/releases/tag/v0.3.0
[0.2.0]: https://github.com/holon-run/uxc/releases/tag/v0.2.0
[0.1.1]: https://github.com/holon-run/uxc/releases/tag/v0.1.1
[0.1.0]: https://github.com/holon-run/uxc/releases/tag/v0.1.0
[0.0.1]: https://github.com/holon-run/uxc/releases/tag/v0.0.1
