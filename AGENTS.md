# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs`: CLI entrypoint, argument parsing, and command routing.
- `src/lib.rs`: public exports and shared crate surface.
- `src/adapters/`: protocol implementations (`openapi`, `graphql`, `grpc`, `mcp`).
- `src/auth/`, `src/cache/`, `src/error.rs`, `src/output.rs`, `src/protocol.rs`: cross-cutting modules.
- `tests/`: integration and regression tests (for example, `auth_integration_test.rs`, `cli_help_regression_test.rs`).
- `docs/`: design notes/plans. `.github/workflows/`: CI, lint, build matrix, and E2E smoke checks.

## Build, Test, and Development Commands
- `make build` or `cargo build --release`: build optimized CLI binary.
- `make run` or `cargo run -- <args>`: run locally.
- `make test` or `cargo test`: run test suite.
- `make check`: quick validation (`cargo check` + `cargo clippy`).
- `make fmt` or `cargo fmt -- --check`: format/check formatting.
- `cargo clippy -- -D warnings`: enforce lint cleanliness before PR.

## Coding Style & Naming Conventions
- Use Rust 2021 conventions and `rustfmt` defaults (4-space indentation, no manual alignment tricks).
- Naming: files/modules in `snake_case`; types/traits in `CamelCase`; constants in `SCREAMING_SNAKE_CASE`.
- Keep CLI behavior predictable: JSON output envelope is the default contract, text mode is opt-in (`--text`).
- Prefer small, focused modules; add protocol-specific logic under `src/adapters/<protocol>.rs`.

## Testing Guidelines
- Add/extend integration tests in `tests/` with descriptive names ending in `_test.rs`.
- Cover both success and failure paths (argument validation, protocol errors, output shape).
- For output assertions, validate stable keys like `ok`, `kind`, and `protocol`.
- Avoid cross-test shared state (cache, daemon socket, temp files, ports). Prefer per-test isolated HOME/tmp dirs; only share state within a single test when the test explicitly validates cache/session reuse behavior.
- Run `cargo test -- --test-threads=1` when reproducing CI behavior.

## Commit & Pull Request Guidelines
- Follow Conventional Commits: `feat: ...`, `fix: ...`, `docs: ...`, `test: ...`, `refactor: ...`.
- Use branch prefixes from project practice: `feature/`, `fix/`, `docs/`, `refactor/`.
- PR titles should match commit style and include clear scope.
- Complete the PR template sections (`What`, `Why`, `How`, `Testing`) and link issues (`Closes #<id>`).
- Before opening a PR, ensure format, clippy, and tests all pass.

## Security & Configuration Tips
- Do not commit secrets or API keys. Use local auth profiles and environment variables.
- If changing protocol adapters, verify behavior against real endpoints (see E2E workflow examples).
