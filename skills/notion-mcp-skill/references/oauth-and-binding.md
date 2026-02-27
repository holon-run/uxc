# OAuth And Binding

## Goal

Authenticate once with OAuth and let `uxc` auto-attach credentials to `https://mcp.notion.com/mcp`.

## Check Local State First (Cache-Safe)

Before starting OAuth, check endpoint binding state:

```bash
uxc auth binding match https://mcp.notion.com/mcp
```

If a valid binding match exists, continue with normal calls.
If binding is missing (or clearly stale), continue with OAuth login below.

## Runtime Validation Strategy

Do not add a dedicated preflight probe by default.
Use the first real read operation as runtime validation.
If that call fails with auth-related errors (`401`, `OAUTH_REQUIRED`, `invalid_token`), run OAuth recovery flow.

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
5. Optionally verify with `uxc auth oauth info <credential_id>` when you know the credential id.

Do not request users to extract raw access tokens from browser/network logs.

## Verify Credential (Optional)

```bash
uxc auth oauth info <credential_id>
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
   - `uxc --auth <credential_id> https://mcp.notion.com/mcp notion-fetch --input-json '{"id":"https://notion.so/your-page-url"}'`
2. Remove stale binding(s) that point to invalid credentials:
   - `uxc auth binding remove <stale_binding_id>`
3. Re-check default path:
   - retry your original read call (for example, `notion-fetch` or `notion-search`)

## Runtime Use

After binding, continue with your intended read operation and treat it as runtime validation.

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
uxc auth binding remove <binding_id>
uxc auth credential remove <credential_id>
```
