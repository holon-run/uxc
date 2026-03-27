# Contributing to UXC

Thank you for your interest in contributing to UXC!

## Getting Started

### Prerequisites

- Rust 1.70 or later
- Git
- GitHub account

### Setup

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/uxc.git
   cd uxc
   ```

3. Add upstream remote:
   ```bash
   git remote add upstream https://github.com/holon-run/uxc.git
   ```

4. Install dependencies:
   ```bash
   cargo build
   ```

## Development Workflow

### Branch Naming

- `feature/` - New features
- `fix/` - Bug fixes
- `docs/` - Documentation
- `refactor/` - Code refactoring

Example: `feature/openapi-parser`

### Making Changes

1. Create a branch:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. Make your changes

3. Format code:
   ```bash
   cargo fmt
   ```

4. Build TypeScript package when touching `packages/**`:
   ```bash
   npm ci
   npm run build:ts
   ```

5. Run linter:
   ```bash
   cargo clippy -- -D warnings
   ```

6. Run tests:
   ```bash
   cargo test
   UXC_BIN=$PWD/target/debug/uxc npm run test:ts
   ```

6. Debug with logging:
   ```bash
   # Run with info logs to see HTTP requests/responses
   RUST_LOG=info cargo run -- https://api.example.com list

   # Run with debug logs for detailed diagnostics
   RUST_LOG=debug cargo run -- https://api.example.com list

   # Enable logs for specific modules only
   RUST_LOG=uxc::adapters::openapi=debug cargo run -- https://api.example.com list
   ```

7. Commit changes:
   ```bash
   git add .
   git commit -m "feat: add OpenAPI schema parser"
   ```

7. Push to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

8. Create a pull request

### Updating Skills

This repository includes a reusable Codex skill at `skills/uxc`.

When CLI syntax, operation naming, or output envelope changes, update skill files in the same PR:

- `skills/uxc/SKILL.md`
- `skills/uxc/references/*`
- `skills/uxc/agents/openai.yaml` (if prompt wording should change)

Run validation before opening a PR:

```bash
bash skills/uxc/scripts/validate.sh
```

## Commit Message Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>: <description>

[optional body]

[optional footer]
```

### Types

- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation changes
- `refactor` - Code refactoring
- `test` - Adding or updating tests
- `chore` - Maintenance tasks
- `perf` - Performance improvements

### Examples

```
feat: add GraphQL introspection support

Implement GraphQL introspection query to discover
available fields and types.

Closes #31
```

```
fix: handle OpenAPI 3.1 parsing errors

Add proper error handling for OpenAPI 3.1 specs
that use different schema formats.

Fixes #42
```

## Pull Request Process

### PR Title

Use the same convention as commit messages:
```
feat: add gRPC reflection support
```

### PR Description

Include:
- **What**: What changes were made
- **Why**: Why these changes are needed
- **How**: How it works
- **Testing**: How it was tested
- **Closes**: Issue number (if applicable)

### PR Template

```markdown
## What
Brief description of changes

## Why
Reason for these changes

## How
Technical approach

## Testing
- [ ] Unit tests added/updated
- [ ] Integration tests added/updated
- [ ] Manual testing performed

## Checklist
- [ ] Code formatted with `cargo fmt`
- [ ] No clippy warnings
- [ ] All tests pass
- [ ] Documentation updated
- [ ] Commit messages follow convention

## Closes
#issue_number
```

## Code Review

### Review Guidelines

- Be respectful and constructive
- Focus on code, not the person
- Provide specific feedback
- Suggest improvements

### Response Time

Maintainers will review PRs within 48 hours.
Feel free to ping after 3 days if no response.

## Release Process

Releases are tag-driven and automated by `.github/workflows/release.yml`.

### Before Tagging

1. Update `Cargo.toml` version
2. Update `CHANGELOG.md` for that version
3. Keep `packages/uxc-daemon-client/package.json` version in sync with `Cargo.toml`
4. Run:

```bash
./scripts/release-check.sh vX.Y.Z
```

### Publish

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

See `docs/operations/release.md` for full details, rollback, and troubleshooting.

See `docs/operations/skills.md` for skill install and maintenance details.

## Testing & Coverage

### Unit Tests

```bash
cargo test
```

### Integration Tests

```bash
cargo test --test '*'
```

### Code Coverage

We track code coverage using `cargo-llvm-cov` and enforce a minimum of 65% line coverage.

#### Generate Coverage Report Locally

To generate coverage reports locally (matching CI behavior):

```bash
# Install cargo-llvm-cov if you haven't already
cargo install cargo-llvm-cov

# Generate HTML report
cargo llvm-cov --html

# Generate coverage summary only
cargo llvm-cov --summary-only

# Open HTML report in browser
open target/llvm-cov/html/index.html  # macOS
xdg-open target/llvm-cov/html/index.html  # Linux
start target/llvm-cov/html/index.html  # Windows
```

#### Coverage Threshold

CI enforces a minimum of 65% line coverage. The CI workflow will fail if coverage falls below this threshold.

To check if your code meets the threshold locally:

```bash
cargo llvm-cov --summary-only --fail-under-lines 65
```

This command will:
1. Run all tests with coverage instrumentation
2. Display coverage summary
3. Exit with error if coverage is below 65%

#### CI Coverage Artifacts

GitHub Actions generates and uploads coverage artifacts on every PR and push to main:
- **JSON Report**: Machine-readable coverage data (`coverage.json`)
- **HTML Report**: Detailed browser-friendly coverage report

Download these artifacts from the Actions run page to review coverage in detail.

### Manual Testing

Test against real APIs:
```bash
# OpenAPI
cargo run -- https://petstore.swagger.io list

# GraphQL (when implemented)
cargo run -- https://graphqlzero.almansi.me/api list
```

## Adding New Adapters

When adding a new protocol adapter:

1. Implement the `Adapter` trait
2. Add protocol detection
3. Add unit tests
4. Add integration tests
5. Update documentation
6. Add examples

See `src/adapters/mod.rs` for the adapter interface.

## Documentation

### Code Documentation

Use rustdoc comments:
```rust
/// Parses an OpenAPI schema from the given URL.
///
/// # Arguments
///
/// * `url` - The OpenAPI schema URL
///
/// # Returns
///
/// Returns a `Result<Value>` with the parsed schema
///
/// # Errors
///
/// Returns an error if:
/// - The URL is invalid
/// - The schema is malformed
/// - Network error occurs
///
/// # Examples
///
/// ```no_run
/// use uxc::adapters::OpenAPIAdapter;
///
/// let adapter = OpenAPIAdapter::new();
/// let schema = adapter.fetch_schema("https://api.example.com").await?;
/// ```
pub async fn fetch_schema(&self, url: &str) -> Result<Value> {
    // ...
}
```

### User Documentation

Update relevant sections in:
- README.md
- docs/ directory
- Examples in examples/

## Questions?

- Open an issue for bugs or feature requests
- Start a discussion for questions
- Join our Discord (when available)

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
