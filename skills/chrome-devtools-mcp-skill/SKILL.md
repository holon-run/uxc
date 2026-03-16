---
name: chrome-devtools-mcp-skill
description: Use Chrome DevTools MCP through UXC over local stdio for page navigation, DOM/a11y snapshots, network inspection, console inspection, and performance tooling, with a live-browser autoConnect default and optional browserUrl or isolated fallback modes.
---

# Chrome DevTools MCP Skill

Use this skill to run Chrome DevTools MCP operations through `uxc` using a fixed stdio endpoint.

Reuse the `uxc` skill for generic MCP discovery, daemon reuse, JSON envelope parsing, and error handling.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- `npx` is available in `PATH` (Node.js installed).
- Chrome 144+ is running locally with remote debugging enabled from `chrome://inspect/#remote-debugging` if you use the default live-browser flow.
- Network access is available for first-time `chrome-devtools-mcp` package fetch.

## Core Workflow (Chrome DevTools MCP-Specific)

Endpoint candidate inputs before finalizing:
- Raw package form from official docs:
  - `npx chrome-devtools-mcp@latest`
- Reliable non-interactive form:
  - `npx -y chrome-devtools-mcp@latest`
- Default live-browser endpoint for this skill:
  - `npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics`
- Explicit browser-url endpoint:
  - `npx -y chrome-devtools-mcp@latest --browserUrl http://127.0.0.1:9222 --no-usage-statistics`
- Fallback isolated endpoint:
  - `npx -y chrome-devtools-mcp@latest --headless --isolated --no-usage-statistics`
- Running local Chrome auto-connect mode:
  - `npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics`

1. Verify protocol/path from official source and probe:
   - Official source:
     - `https://github.com/ChromeDevTools/chrome-devtools-mcp`
   - probe candidate endpoints with:
     - `uxc "npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics" -h`
   - Confirm protocol is MCP stdio (`protocol == "mcp"` in envelope).
2. Detect auth requirement explicitly:
   - Run host help or a minimal read call and inspect envelope.
   - Default local stdio flow requires no OAuth/API key.
   - Existing Chrome attachment requires remote debugging to be enabled separately, but not API auth.
3. Use a fixed link command by default:
   - `command -v chrome-devtools-mcp-cli`
   - If missing, create it:
     - `uxc link chrome-devtools-mcp-cli "npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics"`
   - Optional explicit browser-url link:
     - `command -v chrome-devtools-mcp-port`
     - `uxc link chrome-devtools-mcp-port "npx -y chrome-devtools-mcp@latest --browserUrl http://127.0.0.1:9222 --no-usage-statistics"`
   - Optional isolated fallback link:
     - `command -v chrome-devtools-mcp-isolated`
     - `uxc link chrome-devtools-mcp-isolated "npx -y chrome-devtools-mcp@latest --headless --isolated --no-usage-statistics"`
   - `chrome-devtools-mcp-cli -h`
4. Inspect operation schema before execution:
   - `chrome-devtools-mcp-cli new_page -h`
   - `chrome-devtools-mcp-cli take_snapshot -h`
   - `chrome-devtools-mcp-cli list_network_requests -h`
   - `chrome-devtools-mcp-cli lighthouse_audit -h`
5. Prefer read-first interaction:
   - Start with `new_page`, `list_pages`, `take_snapshot`, `list_network_requests`, or `list_console_messages`.
6. Confirm before mutating page state:
   - `click`
   - `fill`
   - `fill_form`
   - `press_key`
   - `upload_file`
   - `evaluate_script`
   - `handle_dialog`

## Guardrails

- Keep automation on the JSON output envelope; do not rely on `--text`.
- Use `chrome-devtools-mcp-cli` as the default command path.
- Prefer the live-browser default endpoint when you need real logged-in state, current tabs, network diagnostics, console inspection, or performance analysis.
- Prefer `--autoConnect` first when browser-side remote debugging is available.
- Use `chrome-devtools-mcp-port` only when you intentionally run a Chrome instance with `--remote-debugging-port=9222`.
- If no debuggable Chrome is available, fallback to `chrome-devtools-mcp-isolated`.
- Prefer `take_snapshot` over screenshots for model-action loops.
- Prefer `list_network_requests` / `get_network_request` over raw script evaluation when inspecting network behavior.
- Treat `lighthouse_audit`, `performance_start_trace`, and `take_memory_snapshot` as heavier operations; use them intentionally.
- Use `evaluate_script` only when an existing higher-level DevTools tool cannot answer the question.

## References

- Invocation patterns:
  - `references/usage-patterns.md`
