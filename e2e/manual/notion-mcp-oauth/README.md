# Manual E2E: Notion MCP OAuth (Authorization Code + PKCE)

## Goal

Verify OAuth login and MCP calls against Notion MCP using `authorization_code` flow.

## Prerequisites

- `uxc` built locally (`target/debug/uxc`).
- Network access to `https://mcp.notion.com/mcp`.
- A Notion OAuth client configured for Notion MCP.
- The client allows your redirect URI (example: `http://127.0.0.1:8788/callback`).

## Test Data

- Credential ID: `notion-mcp-oauth`
- Binding ID: `notion-mcp`
- Endpoint: `https://mcp.notion.com/mcp`
- Redirect URI: `http://127.0.0.1:8788/callback`

## Steps

1. Start OAuth login:

```bash
target/debug/uxc auth oauth login notion-mcp-oauth \
  --endpoint https://mcp.notion.com/mcp \
  --flow authorization_code \
  --redirect-uri http://127.0.0.1:8788/callback \
  --scope "read write"
```

Notes:
- `--client-id` can be omitted to use dynamic client registration.
- If your workspace policy requires pre-created OAuth apps, pass `--client-id` explicitly.

2. Open the printed authorization URL and complete consent.

3. Paste the authorization code or full callback URL when prompted.

4. Verify OAuth credential is stored:

```bash
target/debug/uxc auth oauth info notion-mcp-oauth
```

Expected:
- `auth_type: "oauth"`
- `oauth.flow: "authorization_code"`
- `oauth.has_refresh_token: true` (if returned by provider)

5. Bind endpoint to credential:

```bash
target/debug/uxc auth binding add \
  --id notion-mcp \
  --host mcp.notion.com \
  --path-prefix /mcp \
  --scheme https \
  --credential notion-mcp-oauth \
  --priority 100
```

6. Verify binding auto-match:

```bash
target/debug/uxc auth binding match https://mcp.notion.com/mcp
```

Expected: `matched: true` and `credential: "notion-mcp-oauth"`.

7. Run MCP calls without `--auth`:

```bash
target/debug/uxc https://mcp.notion.com/mcp list
```

Expected: call succeeds and returns MCP tools.

8. Refresh token:

```bash
target/debug/uxc auth oauth refresh notion-mcp-oauth
```

Expected: refresh succeeds without interactive login.

9. Logout and verify failure:

```bash
target/debug/uxc auth oauth logout notion-mcp-oauth
target/debug/uxc https://mcp.notion.com/mcp list
```

Expected: MCP call fails with OAuth-required error.

10. Cleanup:

```bash
target/debug/uxc auth binding remove notion-mcp
target/debug/uxc auth credential remove notion-mcp-oauth
```
