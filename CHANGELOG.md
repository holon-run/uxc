# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/holon-run/uxc/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/holon-run/uxc/releases/tag/v0.4.0
[0.3.0]: https://github.com/holon-run/uxc/releases/tag/v0.3.0
[0.2.0]: https://github.com/holon-run/uxc/releases/tag/v0.2.0
[0.1.1]: https://github.com/holon-run/uxc/releases/tag/v0.1.1
[0.1.0]: https://github.com/holon-run/uxc/releases/tag/v0.1.0
[0.0.1]: https://github.com/holon-run/uxc/releases/tag/v0.0.1
