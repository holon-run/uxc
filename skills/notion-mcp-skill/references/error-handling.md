# Error Handling

## Envelope Contract

Always parse structured output:
- Success: `ok: true`
- Failure: `ok: false` with `error.code` and `error.message`

## OAuth Error Codes

Handle these codes explicitly:
- `OAUTH_REQUIRED`
- `OAUTH_DISCOVERY_FAILED`
- `OAUTH_TOKEN_EXCHANGE_FAILED`
- `OAUTH_REFRESH_FAILED`
- `OAUTH_SCOPE_INSUFFICIENT`

## Additional Common Failures

- `PROTOCOL_DETECTION_FAILED` with `401 invalid_token` in MCP probe diagnostics

## Recovery Playbook

`OAUTH_REQUIRED`:
1. Ensure credential exists (`uxc auth oauth info notion-mcp`).
2. Ensure endpoint binding matches (`uxc auth binding match https://mcp.notion.com/mcp`).
3. Re-login if needed.

`OAUTH_DISCOVERY_FAILED`:
1. Check network reachability to endpoint.
2. Retry login.
3. If persistent, rerun with explicit `--client-id`.

`OAUTH_TOKEN_EXCHANGE_FAILED`:
1. Confirm callback URL is exact and URL-encoded query is intact.
2. Retry full login flow.
3. If dynamic registration was used, try explicit `--client-id`.

`OAUTH_REFRESH_FAILED`:
1. Try `uxc auth oauth refresh notion-mcp`.
2. If refresh token invalid/expired, perform login again.

`OAUTH_SCOPE_INSUFFICIENT`:
1. Re-login with broader scopes (for Notion MCP generally include `read` and `write`).

`PROTOCOL_DETECTION_FAILED` + `invalid_token`:
1. Check for duplicate endpoint bindings (`uxc auth binding list`).
2. Validate default probe path (`uxc https://mcp.notion.com/mcp describe notion-fetch`).
3. Remove stale duplicate binding(s) and retry probe.

## Write-Safety Failures

When `notion-update-page` signals deletion risk:
1. Do not retry automatically with permissive flags.
2. Show what would be deleted.
3. Ask for explicit confirmation before executing destructive change.
