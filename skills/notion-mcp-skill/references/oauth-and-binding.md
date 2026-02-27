# OAuth And Binding

## Goal

Authenticate once with OAuth and let `uxc` auto-attach credentials to `https://mcp.notion.com/mcp`.

## Probe First (No Auth Assumption)

Before starting OAuth, check whether endpoint access already works via existing binding/credential:

```bash
uxc https://mcp.notion.com/mcp describe notion-fetch
```

If probe succeeds, skip OAuth login and continue with normal calls.
If probe fails with auth-related error, continue with OAuth login below.

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

## Duplicate Binding Handling

If multiple bindings target the same endpoint, default calls may hit a stale token.

Detect duplicates:

```bash
uxc auth binding list
```

If more than one binding matches `https://mcp.notion.com/mcp`:
1. Verify with explicit credential first:
   - `uxc --auth <credential_id> https://mcp.notion.com/mcp describe notion-fetch`
2. Remove stale binding(s) that point to invalid credentials:
   - `uxc auth binding remove <stale_binding_id>`
3. Re-check default path:
   - `uxc https://mcp.notion.com/mcp describe notion-fetch`

## Runtime Use

After binding, verify runtime works with a lightweight probe:

```bash
uxc https://mcp.notion.com/mcp describe notion-fetch
```

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
