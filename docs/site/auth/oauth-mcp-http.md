# MCP HTTP OAuth

This page summarizes OAuth support for MCP HTTP in UXC.

## Supported Flows

- `device_code`
- `authorization_code` with PKCE
- `client_credentials`

## What UXC Handles

- token persistence in the local credential store
- refresh before expiry
- one-time refresh and retry on `401 Unauthorized`
- structured OAuth error reporting

## Typical Commands

Device Code:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <mcp_url> \
  --flow device_code \
  --client-id <client_id> \
  --scope "openid profile"
```

Client Credentials:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <mcp_url> \
  --flow client_credentials \
  --client-id <client_id> \
  --client-secret <client_secret> \
  --scope "tools.read"
```

Authorization Code + PKCE:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <mcp_url> \
  --flow authorization_code \
  --redirect-uri <redirect_uri> \
  --scope "openid profile"
```

Agent-friendly two-step flow:

```bash
uxc auth oauth start <credential_id> \
  --endpoint <mcp_url> \
  --redirect-uri <redirect_uri> \
  --client-id <client_id> \
  --scope "openid profile"
```

```bash
uxc auth oauth complete <credential_id> \
  --session-id <session_id> \
  --authorization-response "http://127.0.0.1:11111/callback?code=..."
```

## Runtime Behavior

When calling MCP HTTP with an OAuth credential:

1. Refresh before expiry when needed.
2. Retry once after `401` if refresh succeeds.
3. Return structured OAuth errors if recovery fails.

## Common Error Codes

- `OAUTH_REQUIRED`
- `OAUTH_DISCOVERY_FAILED`
- `OAUTH_TOKEN_EXCHANGE_FAILED`
- `OAUTH_REFRESH_FAILED`
- `OAUTH_SCOPE_INSUFFICIENT`
