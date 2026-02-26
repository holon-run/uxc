# MCP HTTP OAuth (Issue #97)

This document describes OAuth support for MCP HTTP in `uxc`.

## Scope

Implemented in MVP:

- OAuth login via `device_code` and `client_credentials`
- Token persistence in `~/.uxc/profiles.toml`
- Auto refresh before expiry (60s skew)
- One-time refresh + retry on `401 Unauthorized`
- Structured error codes for OAuth failures

## Commands

Login with Device Code:

```bash
uxc auth oauth login <profile> \
  --endpoint <mcp_url> \
  --flow device_code \
  --client-id <client_id> \
  --scope "openid profile"
```

Login with Client Credentials:

```bash
uxc auth oauth login <profile> \
  --endpoint <mcp_url> \
  --flow client_credentials \
  --client-id <client_id> \
  --client-secret <client_secret> \
  --scope "tools.read"
```

Refresh token manually:

```bash
uxc auth oauth refresh <profile>
```

Inspect OAuth profile:

```bash
uxc auth oauth info <profile>
```

Logout (clear OAuth token data in the profile):

```bash
uxc auth oauth logout <profile>
```

## Runtime behavior

When calling MCP HTTP with an OAuth profile:

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

- OAuth profile data is currently stored in plaintext (same as existing profile storage).
- Local encrypted storage is tracked separately (Issue #29).
