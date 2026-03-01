# MCP HTTP OAuth (Issue #97)

This document describes OAuth support for MCP HTTP in `uxc`.

OAuth tokens are stored in a credential (`credentials.json`). Runtime selection can use
`--auth <credential_id>` or endpoint binding auto-match (`auth_bindings.json`).

## Scope

Implemented in MVP:

- OAuth login via `device_code`, `authorization_code` (PKCE), and `client_credentials`
- Token persistence in `~/.uxc/credentials.json`
- Auto refresh before expiry (60s skew)
- One-time refresh + retry on `401 Unauthorized`
- Structured error codes for OAuth failures

## Commands

Login with Device Code:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <mcp_url> \
  --flow device_code \
  --client-id <client_id> \
  --scope "openid profile"
```

Login with Client Credentials:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <mcp_url> \
  --flow client_credentials \
  --client-id <client_id> \
  --client-secret <client_secret> \
  --scope "tools.read"
```

Login with Authorization Code + PKCE:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <mcp_url> \
  --flow authorization_code \
  --redirect-uri <redirect_uri> \
  --scope "openid profile"
```

Use `--authorization-code` to provide the code directly, or run interactively and paste the
authorization code / callback URL when prompted.

Notes:
- `--client-id` is optional for `authorization_code`.
- When omitted, `uxc` will attempt OAuth Dynamic Client Registration via provider
  `registration_endpoint` (RFC 7591).
- If provider does not expose registration, pass `--client-id` explicitly.

Refresh token manually:

```bash
uxc auth oauth refresh <credential_id>
```

Inspect OAuth credential:

```bash
uxc auth oauth info <credential_id>
```

Logout (clear OAuth token data in the credential):

```bash
uxc auth oauth logout <credential_id>
```

## Runtime behavior

When calling MCP HTTP with an OAuth credential:

1. If token is near expiry, `uxc` refreshes first.
2. If server returns `401`, `uxc` refreshes once and retries once.
3. If refresh cannot continue, command returns structured OAuth errors.

## Error codes

- `OAUTH_REQUIRED`
- `OAUTH_DISCOVERY_FAILED`
- `OAUTH_TOKEN_EXCHANGE_FAILED`
- `OAUTH_REFRESH_FAILED`
- `OAUTH_SCOPE_INSUFFICIENT`

## Notes

- UXC supports mixed credential sources in the same store: `literal`, `env`, and `op`.
- For external sources (`env`, `op`), resolved secret values are used at runtime and are not written back as plaintext credential values.
