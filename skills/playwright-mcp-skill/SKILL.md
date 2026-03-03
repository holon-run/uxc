---
name: playwright-mcp-skill
description: Run browser automation through @playwright/mcp over UXC stdio MCP, with daemon-friendly session reuse and safe action guardrails. Use when tasks need deterministic page navigation, DOM snapshots, and scripted browser interaction from CLI.
---

# Playwright MCP Skill

Use this skill to run Playwright MCP operations through `uxc` using fixed stdio endpoints.

Reuse the `uxc` skill for generic protocol discovery, auth/error handling, and envelope parsing rules.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- `npx` is available in `PATH` (Node.js installed).
- Network access for first-time `@playwright/mcp` package fetch.
- Browser runtime can start locally (headless mode is default in this skill).

## Core Workflow (Playwright-Specific)

Endpoint candidate inputs before finalizing:
- Raw package form from docs: `npx @playwright/mcp@latest`
- Reliable non-interactive form: `npx -y @playwright/mcp@latest`
- Isolated/headless stable form (default for this skill):
  - `npx -y @playwright/mcp@latest --headless --isolated`
- Shared-profile headless form (for persistent login state):
  - `npx -y @playwright/mcp@latest --headless --user-data-dir ~/.uxc/playwright-profile`
- Shared-profile headed form (for interactive debug with same login state):
  - `npx -y @playwright/mcp@latest --user-data-dir ~/.uxc/playwright-profile`

1. Verify protocol/path from official source and probe:
   - Official source: `https://github.com/microsoft/playwright-mcp`
   - probe candidate endpoints with:
     - `uxc "npx -y @playwright/mcp@latest --headless --isolated" -h`
   - Confirm protocol is MCP stdio (`protocol == "mcp"` in envelope).
2. Detect auth requirement explicitly:
   - Run host help or a minimal read call and inspect envelope.
   - `@playwright/mcp` default flow is no OAuth/API key for local stdio use.
3. Use fixed link command by default:
   - `command -v playwright-mcp-cli`
   - If missing, create it:
     - `uxc link playwright-mcp-cli "npx -y @playwright/mcp@latest --headless --isolated"`
   - Optional shared-profile dual command setup for persistent sessions:
     - `command -v playwright-mcp-headless`
     - `command -v playwright-mcp-ui`
     - `uxc link playwright-mcp-headless "npx -y @playwright/mcp@latest --headless --user-data-dir ~/.uxc/playwright-profile"`
     - `uxc link playwright-mcp-ui "npx -y @playwright/mcp@latest --user-data-dir ~/.uxc/playwright-profile"`
   - `playwright-mcp-cli -h`
   - If command conflict is detected and cannot be safely reused, stop and ask skill maintainers to pick another fixed command name.
4. Inspect operation schema before execution:
   - `playwright-mcp-cli browser_navigate -h`
   - `playwright-mcp-cli browser_snapshot -h`
   - `playwright-mcp-cli browser_click -h`
5. Prefer read-first interaction:
   - Start with navigation/snapshot before mutating page state.
6. Execute actions with explicit confirmation when impact is high:
   - Confirm before form submission, checkout, delete/destructive actions, or irreversible multi-step flows.

## Guardrails

- Keep automation on JSON output envelope; do not rely on `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `playwright-mcp-cli` as default command path.
- `playwright-mcp-cli <operation> ...` is equivalent to `uxc "npx -y @playwright/mcp@latest --headless --isolated" <operation> ...`.
- Use direct `uxc "<endpoint>" ...` only as temporary fallback when link setup is unavailable.
- If browser profile conflict appears, keep `--isolated` in endpoint and retry via the same fixed link command.
- When using shared `--user-data-dir`, run headless/headed links serially (not concurrently).
- When switching between `playwright-mcp-headless` and `playwright-mcp-ui` with the same `--user-data-dir`, uxc will evict the previous cached stdio session so the profile lock is released.
  - If you're on an older uxc version (or the profile is still busy), fallback: `uxc daemon stop`
- Prefer `browser_snapshot` over screenshots for model-action loops.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
