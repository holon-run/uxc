---
name: qmd-mcp-skill
description: Use a local QMD knowledge base through UXC over MCP stdio, with daemon-backed session reuse and typed retrieval flows that avoid repeated model warmup and unnecessary query-expansion latency.
metadata:
  short-description: Query local QMD indexes via UXC MCP stdio
---

# QMD MCP Skill

Use this skill to query a local QMD index through `uxc` using a fixed MCP stdio link.

Reuse the `uxc` skill for generic protocol discovery, JSON envelope parsing, and daemon lifecycle basics.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- `qmd` is installed and available in the runtime `PATH`, or can be launched through a shell wrapper.
- A QMD index already exists and is healthy:
  - `qmd status`
  - `qmd update`
  - `qmd embed`
- For GPU-backed setups, the shell used by `qmd mcp` already exports any required runtime environment such as `CUDA_PATH`, `CUDACXX`, `LD_LIBRARY_PATH`, or Node/nvm initialization.

## Core Workflow

1. Verify the local QMD index first:
   - `qmd status`
   - Confirm collections, vector count, and device look reasonable before linking MCP.
2. Use a fixed link command by default:
   - `command -v qmd-mcp-cli`
   - If missing and `qmd` already works in the current shell:
     - `uxc link --daemon-idle-ttl 0 qmd-mcp-cli "qmd mcp"`
   - If `qmd` depends on `nvm`, CUDA env, or other shell setup, wrap it explicitly:
     - `uxc link --daemon-idle-ttl 0 qmd-mcp-cli "/bin/bash -lc 'export NVM_DIR=$HOME/.nvm; . $NVM_DIR/nvm.sh; nvm use 23 >/dev/null; export CUDA_PATH=/usr/local/cuda-11.6; export CUDA_HOME=/usr/local/cuda-11.6; export CUDACXX=/usr/local/cuda-11.6/bin/nvcc; export LD_LIBRARY_PATH=/usr/local/cuda-11.6/lib64:${LD_LIBRARY_PATH:-}; export NODE_LLAMA_CPP_CMAKE_OPTION_CMAKE_CUDA_ARCHITECTURES=86; export NODE_LLAMA_CPP_GPU=cuda; qmd mcp'"`
   - `qmd-mcp-cli -h`
   - If command conflict is detected and cannot be safely reused, stop and ask skill maintainers to pick another fixed command name.
3. Confirm the daemon-backed stdio path is active:
   - `uxc daemon status`
   - `uxc daemon sessions`
4. Inspect operation schema before execution:
   - `qmd-mcp-cli query -h`
   - `qmd-mcp-cli get -h`
   - `qmd-mcp-cli multi_get -h`
   - `qmd-mcp-cli status -h`
5. Prefer typed retrieval over CLI-style auto expansion:
   - Start with `query` using explicit `lex` / `vec` / `hyde` searches
   - Use `get` or `multi_get` only after narrowing candidates

## Recommended Usage Pattern

1. Health check the index:
   - `qmd-mcp-cli status`
2. Start with a fast explicit search payload:
   - `qmd-mcp-cli query '{"searches":[{"type":"lex","query":"\"execution layer\" MCP CLI"},{"type":"vec","query":"What is the missing execution surface between MCP and CLI?"}],"collections":["workspace"],"limit":5,"intent":"Find the article explaining capability description, execution surface, and workflow orchestration"}'`
3. Retrieve the chosen file:
   - `qmd-mcp-cli get file=workspace/public/mcp-is-not-the-problem/readme.md`
4. Use `multi_get` for a short candidate set only:
   - `qmd-mcp-cli multi_get pattern='workspace/public/*.md,workspace/research/*.md' maxBytes=20480`

## Capability Map

- Search:
  - `query`
- Retrieval:
  - `get`
  - `multi_get`
- Health:
  - `status`

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `qmd-mcp-cli` as the default command path.
- `qmd-mcp-cli <operation> ...` is equivalent to `uxc "qmd mcp" <operation> ...` when `qmd` already runs correctly in the current shell.
- If `qmd` requires shell initialization or GPU env exports, use the same wrapped shell command in the link and any direct `uxc "<host>" ...` fallback.
- Prefer explicit typed `query` payloads over the standalone QMD CLI hybrid mode when latency matters.
- Treat `lex` as the default fast path:
  - exact names
  - quoted phrases
  - negation
- Add `vec` when the question is semantic but still bounded.
- Add `hyde` only for nuanced or sparse topics; it is the most expensive query type.
- Use `intent` to disambiguate ambiguous search terms instead of over-expanding the query text itself.
- Keep `limit` and `candidateLimit` modest for interactive use.
- `--daemon-idle-ttl 0` is recommended for QMD because the first heavy request may warm models and sessions; long-lived reuse makes repeated calls much faster.

## Notes

- The first MCP request can still be slow while the `uxc` daemon creates the stdio session and QMD warms model state.
- Repeated calls through the same `uxc` daemon session can drop sharply in latency once the session is warm.
- This skill is best for local knowledge bases, notes, and markdown corpora already indexed by QMD.
- If you need the highest-quality but slowest retrieval path, you can still express it through `query` with richer `searches` arrays instead of shelling out to the standalone QMD CLI hybrid mode.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
