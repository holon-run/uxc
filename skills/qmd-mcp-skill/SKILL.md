---
name: qmd-mcp-skill
description: "Search and retrieve documents from a local QMD knowledge base through UXC over MCP stdio with daemon-backed session reuse. Use when the task involves querying local markdown corpora, notes, or indexed documents via lex/vec/hyde search types."
user-invocable: true
triggers:
  - qmd
  - qmd search
  - local knowledge base
  - document retrieval
  - qmd query
---

# QMD MCP Skill

Use this skill to query a local QMD index through `uxc` using a fixed MCP stdio link.

Reuse the `uxc` skill for generic protocol discovery, JSON envelope parsing, and daemon lifecycle basics.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- `qmd` is installed and available in the runtime `PATH`, or launchable through a shell wrapper.
- A healthy QMD index (`qmd status`, `qmd update`, `qmd embed`).
- For GPU-backed setups, the shell must export `CUDA_PATH`, `CUDACXX`, `LD_LIBRARY_PATH`, or Node/nvm initialization. See `references/usage-patterns.md` for a full wrapper example.

## Core Workflow

1. Verify the local QMD index:
   - `qmd status`
2. Use a fixed link command by default:
   - `command -v qmd-mcp-cli`
   - If missing: `uxc link --daemon-idle-ttl 0 qmd-mcp-cli "qmd mcp"`
   - For GPU/nvm setups, see the wrapped command in `references/usage-patterns.md`.
   - `qmd-mcp-cli -h`
3. Confirm the daemon-backed stdio path is active:
   - `uxc daemon status`
   - `uxc daemon sessions`
4. Inspect operation schema before execution:
   - `qmd-mcp-cli query -h`
   - `qmd-mcp-cli get -h`
   - `qmd-mcp-cli multi_get -h`
   - `qmd-mcp-cli status -h`
5. Execute typed retrieval — start with `query` using explicit `lex`/`vec`/`hyde` searches, then narrow with `get` or `multi_get`.

## Recommended Usage Pattern

1. Health check: `qmd-mcp-cli status`
2. Fast explicit search:
   - `qmd-mcp-cli query '{"searches":[{"type":"lex","query":"\"execution layer\" MCP CLI"},{"type":"vec","query":"What is the missing execution surface between MCP and CLI?"}],"collections":["workspace"],"limit":5,"intent":"Find the article explaining capability description, execution surface, and workflow orchestration"}'`
3. Retrieve chosen file:
   - `qmd-mcp-cli get file=workspace/public/mcp-is-not-the-problem/readme.md`
4. Batch retrieve (short candidate set only):
   - `qmd-mcp-cli multi_get pattern='workspace/public/*.md,workspace/research/*.md' maxBytes=20480`

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `qmd-mcp-cli <operation> ...` is equivalent to `uxc "qmd mcp" <operation> ...` when `qmd` already runs correctly in the current shell.
- If `qmd` requires shell initialization or GPU env exports, use the same wrapped shell command in the link and any direct `uxc "<host>" ...` fallback.
- Prefer `lex` as the default fast path (exact names, quoted phrases, negation). Add `vec` for semantic queries; add `hyde` only for nuanced/sparse topics (most expensive).
- Use `intent` to disambiguate ambiguous search terms instead of over-expanding the query text.
- Keep `limit` and `candidateLimit` modest for interactive use.
- `--daemon-idle-ttl 0` is recommended — the first request warms models; long-lived reuse makes repeated calls faster.

## References

- Invocation patterns and GPU wrapper examples: `references/usage-patterns.md`
