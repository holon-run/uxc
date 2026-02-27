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

- First real call fails with `401 invalid_token`

## Recovery Playbook

`OAUTH_REQUIRED`:
1. Ensure endpoint binding matches (`uxc auth binding match https://mcp.notion.com/mcp`).
2. If you know the credential id, inspect it (`uxc auth oauth info <credential_id>`).
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
1. Try `uxc auth oauth refresh <credential_id>`.
2. If refresh token invalid/expired, perform login again.

`OAUTH_SCOPE_INSUFFICIENT`:
1. Re-login with broader scopes (for Notion MCP generally include `read` and `write`).

First real call + `invalid_token`:
1. Check for duplicate endpoint bindings (`uxc auth binding list`).
2. Confirm the binding that currently matches:
   - `uxc auth binding match https://mcp.notion.com/mcp`
3. If multiple candidates exist, verify each candidate with explicit credential:
   - `uxc --auth <credential_id> https://mcp.notion.com/mcp <same_read_operation> ...`
4. Remove only the binding(s) confirmed stale/invalid.
5. Retry the original read call that failed.

## Write-Safety Failures

When `notion-update-page` signals deletion risk:
1. Do not retry automatically with permissive flags.
2. Show what would be deleted.
3. Ask for explicit confirmation before executing destructive change.
