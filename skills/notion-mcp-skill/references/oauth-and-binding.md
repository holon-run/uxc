# OAuth And Binding

## Scope

This file keeps Notion-specific OAuth notes only.
For canonical OAuth and binding workflow, use `$uxc` skill:
- section: `OAuth and credential/binding lifecycle`
- file name in `$uxc`: `references/oauth-and-binding.md`

## Notion Endpoint Defaults

- endpoint: `https://mcp.notion.com/mcp`
- suggested scopes: `read`, `write`
- callback example: `http://127.0.0.1:8788/callback`

## Recommended Notion Login

```bash
uxc auth oauth login notion-mcp \
  --endpoint https://mcp.notion.com/mcp \
  --flow authorization_code \
  --redirect-uri http://127.0.0.1:8788/callback \
  --scope read \
  --scope write
```

Notes:
- Omit `--client-id` by default. `uxc` will try dynamic client registration.
- If provider/workspace policy rejects dynamic registration, rerun with explicit `--client-id`.

## Interactive Callback Handoff

For agent-driven/manual runs:
1. Run the login command and capture the authorization URL printed by `uxc`.
2. Ask the user to open the URL and approve access.
3. Ask the user to paste the full callback URL (for example: `http://127.0.0.1:8788/callback?code=...&state=...`).
4. Paste that callback URL into the waiting `uxc` login prompt.
5. Optionally verify with `uxc auth oauth info <credential_id>` when you know the credential id.

## Notion Binding Example

```bash
uxc auth binding add \
  --id notion-mcp \
  --host mcp.notion.com \
  --path-prefix /mcp \
  --scheme https \
  --credential notion-mcp \
  --priority 100
```

Validate match:

```bash
uxc auth binding match https://mcp.notion.com/mcp
```

## Notion Duplicate-Binding Tip

If multiple bindings match Notion endpoint, verify with explicit credential against the same read call before removing stale bindings.

Recommended shortcut for repeated usage:

```bash
uxc link notion-mcp-cli https://mcp.notion.com/mcp
```

Then run operation discovery/calls:

```bash
uxc https://mcp.notion.com/mcp list
notion-mcp-cli list
notion-mcp-cli describe notion-fetch
```
