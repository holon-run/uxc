---
name: notion-mcp-skill
description: Operate Notion workspace content through Notion MCP using the UXC CLI, including search, fetch, users/teams lookup, page/database creation and updates, and comments. Use when tasks require calling Notion tools over MCP with OAuth (authorization_code + PKCE), especially when safe write controls and JSON-envelope parsing are required.
---

# Notion MCP Skill

Use this skill to run Notion MCP operations through `uxc` with OAuth and guarded write behavior.

Reuse the `uxc` skill guidance for discovery, schema inspection, OAuth lifecycle, and error recovery.
Do not assume another skill is auto-triggered in every runtime. Keep this skill executable on its own.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://mcp.notion.com/mcp`.
- OAuth callback listener is reachable (default examples use `http://127.0.0.1:8788/callback`).
- `uxc` skill is available for generic discovery/describe/execute patterns.

## Core Workflow (Notion-Specific)

Endpoint argument style in this skill:
- Prefer shorthand `mcp.notion.com/mcp` (scheme omitted).
- Full URL `https://mcp.notion.com/mcp` is also valid.

1. Ensure endpoint mapping exists:
   - `uxc auth binding match mcp.notion.com/mcp`
2. If mapping/auth is not ready, start OAuth login:
   - `uxc auth oauth login notion-mcp --endpoint mcp.notion.com/mcp --flow authorization_code --redirect-uri http://127.0.0.1:8788/callback --scope read --scope write`
   - Prompt user to open the printed authorization URL.
   - Ask user to paste the full callback URL after consent.
3. Bind endpoint to the credential:
   - `uxc auth binding add --id notion-mcp --host mcp.notion.com --path-prefix /mcp --scheme https --credential notion-mcp --priority 100`
4. Use fixed link command by default:
   - `command -v notion-mcp-cli`
   - If missing, create it: `uxc link notion-mcp-cli mcp.notion.com/mcp`
   - `notion-mcp-cli -h`
   - If command conflict is detected and cannot be safely reused, stop and ask skill maintainers to pick a different fixed command name.
5. Discover tools and inspect schema before execution:
   - `notion-mcp-cli -h`
   - `notion-mcp-cli notion-fetch -h`
   - `notion-fetch` requires `id` (URL or UUID). Examples:
     - `notion-mcp-cli notion-fetch id="https://notion.so/your-page-url"`
     - `notion-mcp-cli notion-fetch id="12345678-90ab-cdef-1234-567890abcdef"`
   - Common operations include `notion-search`, `notion-fetch`, and `notion-update-page`.
6. Prefer read path first:
   - Search/fetch current state before any write.
7. Execute writes only after explicit user confirmation:
   - For `notion-update-page` operations that may delete content, always confirm intent first.

## OAuth Interaction Template

Use this exact operator-facing flow:

1. Start login command and wait for authorization URL output.
2. Tell user:
   - Open this URL in browser and approve Notion access.
   - Copy the full callback URL from browser address bar.
   - Paste the callback URL back in chat.
3. Resume the waiting `uxc` login prompt with the pasted callback URL.
4. Optionally confirm success with:
   - `uxc auth oauth info <credential_id>`

Do not ask user to manually extract or copy bearer tokens. Token exchange is handled by `uxc`.

## Guardrails

- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Use `notion-mcp-cli` as the default command path for all Notion MCP calls in this skill.
- `notion-mcp-cli <operation> ...` is equivalent to `uxc mcp.notion.com/mcp <operation> ...`.
- Use direct `uxc mcp.notion.com/mcp ...` only as a temporary fallback when link setup is unavailable.
- Call `notion-fetch` before `notion-create-pages` or `notion-update-page` when targeting database-backed content to obtain exact schema/property names.
- Treat operations as high impact by default:
  - Require explicit user confirmation before create/update/move/delete-style actions.
- If OAuth/auth fails, use `uxc` skill OAuth/error playbooks first, then apply Notion-specific checks in this skill's references.

## References

- Notion-specific auth notes (thin wrapper over `uxc` skill OAuth guidance):
  - `references/oauth-and-binding.md`
- Invocation patterns by task:
  - `references/usage-patterns.md`
- Notion-specific failure notes (thin wrapper over `uxc` skill error guidance):
  - `references/error-handling.md`
