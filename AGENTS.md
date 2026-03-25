# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs`: CLI entrypoint, argument parsing, and command routing.
- `src/lib.rs`: public exports and shared crate surface.
- `src/adapters/`: protocol implementations (`openapi`, `graphql`, `grpc`, `mcp`).
- `src/auth/`, `src/cache/`, `src/error.rs`, `src/output.rs`, `src/protocol.rs`: cross-cutting modules.
- `tests/`: integration and regression tests (for example, `auth_integration_test.rs`, `cli_help_regression_test.rs`).
- `docs/site/`: public documentation site content served via `mdorigin`.
- `docs/`: internal notes, release/maintenance docs, and compatibility pointers for docs that moved to the site.
- `.github/workflows/`: CI, lint, build matrix, E2E smoke checks, and docs-site deployment.

## Build, Test, and Development Commands
- `make build` or `cargo build --release`: build optimized CLI binary.
- `make run` or `cargo run -- <args>`: run locally.
- `make test` or `cargo test`: run test suite.
- `make check`: quick validation (`cargo check` + `cargo clippy`).
- `make fmt` or `cargo fmt -- --check`: format/check formatting.
- `cargo clippy -- -D warnings`: enforce lint cleanliness before PR.
- `npm run docs:dev`: preview the docs site locally.
- `npm run docs:index`: regenerate directory indexes for `docs/site/`.
- `npm run docs:build`: build the Cloudflare docs-site bundle.

## Coding Style & Naming Conventions
- Use Rust 2021 conventions and `rustfmt` defaults (4-space indentation, no manual alignment tricks).
- Naming: files/modules in `snake_case`; types/traits in `CamelCase`; constants in `SCREAMING_SNAKE_CASE`.
- Keep CLI behavior predictable: JSON output envelope is the default contract, text mode is opt-in (`--text`).
- Prefer small, focused modules; add protocol-specific logic under `src/adapters/<protocol>.rs`.

## Documentation Guidelines
- Put public product documentation in `docs/site/`. Do not add new public-facing docs under top-level `docs/`.
- Keep top-level `docs/` for internal decision notes, release/maintenance workflows, and short compatibility pointers.
- Treat `README.md` and `README.zh-CN.md` as GitHub landing pages; prefer linking user-facing docs to `https://uxc.holon.run`.
- Treat `skills/README.md` and `https://uxc.holon.run/skills/` as the browsable skills catalog.
- Keep `docs/skills.md` as the skills publish and maintenance log, not as the primary skills directory.
- When changing `docs/site/`, run `npm run docs:index` and `npm run docs:build` before pushing.

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
