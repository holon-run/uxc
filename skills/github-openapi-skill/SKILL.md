---
name: github-openapi-skill
description: Operate GitHub REST API through UXC with the official OpenAPI schema, explicit gh-to-uxc auth import, and read-first guardrails for repo, issue, pull request, and event workflows.
---

# GitHub REST API Skill

Use this skill to run GitHub REST API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- `gh` is installed and already authenticated with `gh auth login`.
- Network access to `https://api.github.com`.
- Access to the official GitHub REST OpenAPI schema URL:
  - `https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json`

## Authentication

### Preferred Path: Import local `gh` auth into `uxc`

Import the current GitHub CLI token into a local `uxc` bearer credential and create a standard GitHub API binding:

```bash
uxc auth credential import github --from gh
```

This creates:

- credential: `github`
- binding: `github-api`
- bound host: `https://api.github.com`

Verify the binding:

```bash
uxc auth binding match https://api.github.com/repos/holon-run/uxc
```

### Optional Path: Manual bearer token

If `gh` is unavailable, configure a PAT manually:

```bash
uxc auth credential set github \
  --auth-type bearer \
  --secret-env GITHUB_TOKEN

uxc auth binding add \
  --id github-api \
  --host api.github.com \
  --path-prefix / \
  --scheme https \
  --credential github \
  --priority 100
```

Do not commit GitHub tokens or hardcode them in this skill or repository.

## Core Workflow

1. Use fixed link command by default:
   - `command -v github-openapi-cli`
   - If missing, create it:
     `uxc link github-openapi-cli https://api.github.com --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json`
   - `github-openapi-cli -h`

2. Inspect operation schema first:
   - `github-openapi-cli get:/user -h`
   - `github-openapi-cli get:/repos/{owner}/{repo} -h`
   - `github-openapi-cli get:/repos/{owner}/{repo}/issues -h`

3. Execute reads before writes:
   - `github-openapi-cli get:/user`
   - `github-openapi-cli get:/repos/{owner}/{repo} owner=holon-run repo=uxc`
   - `github-openapi-cli get:/repos/{owner}/{repo}/issues owner=holon-run repo=uxc state=open per_page=10`
   - `github-openapi-cli get:/repos/{owner}/{repo}/pulls owner=holon-run repo=uxc state=open per_page=10`

4. Treat mutations as explicit-confirmation operations:
   - `github-openapi-cli post:/repos/{owner}/{repo}/issues '{"owner":"holon-run","repo":"uxc","title":"Example issue"}'`

## Guardrails

- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `github-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.github.com --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json <operation> ...`.
- Prefer read operations before write operations.
- Treat issue creation, issue edits, comments, labels, repo administration, releases, and workflow-trigger endpoints as write/high-risk operations; require explicit user confirmation before execution.

## References

- Usage patterns: `references/usage-patterns.md`
- GitHub REST OpenAPI schema:
  `https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json`
- GitHub REST docs: `https://docs.github.com/rest`
