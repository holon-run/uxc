# OAuth And Binding

## Goal

Authenticate once with OAuth and let `uxc` auto-attach credentials to `https://mcp.notion.com/mcp`.

## Recommended Login (Dynamic Client Registration First)

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
5. Verify with `uxc auth oauth info notion-mcp`.

Do not request users to extract raw access tokens from browser/network logs.

## Verify Credential

```bash
uxc auth oauth info notion-mcp
```

Expect:
- `auth_type: "oauth"`
- `oauth.flow: "authorization_code"`
- `oauth.has_refresh_token` depending on provider response

## Create Endpoint Binding

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

## Runtime Use

After binding, normal MCP calls can omit `--auth`:

```bash
uxc https://mcp.notion.com/mcp list
```

Optional explicit auth:

```bash
uxc --auth notion-mcp https://mcp.notion.com/mcp list
```

## Refresh And Logout

```bash
uxc auth oauth refresh notion-mcp
uxc auth oauth logout notion-mcp
```

Cleanup:

```bash
uxc auth binding remove notion-mcp
uxc auth credential remove notion-mcp
```
