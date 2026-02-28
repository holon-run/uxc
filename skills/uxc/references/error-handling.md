# Error Handling

## Envelope-First Handling

Always parse `ok` first.

- `ok=true`: consume `data`
- `ok=false`: branch by `error.code`

## Common Failure Classes

1. Discovery failure
- Symptoms: `list` fails, protocol not detected, endpoint mismatch
- Actions:
  - verify host URL/scheme/port
  - run with `RUST_LOG=debug`
  - for OpenAPI schema separation, provide `--schema-url` or mapping if needed

2. Operation not found
- Symptoms: `describe` or call reports unknown operation
- Actions:
  - refresh with `list`
  - check exact operation naming convention per protocol

3. Input validation failure
- Symptoms: invalid argument / missing field
- Actions:
  - inspect `describe` schema
  - start from minimal required payload
  - prefer `key=value` or bare positional JSON for primary calls
  - use `--input-json` only when positional JSON is inconvenient
  - do not combine positional JSON and `--input-json` in one command

4. Runtime transport failure
- Symptoms: timeout, connection reset, TLS error
- Actions:
  - retry with bounded attempts
  - verify endpoint health with native tooling (`curl`, `grpcurl`)

5. OAuth authentication failure
- Symptoms: `OAUTH_REQUIRED`, `OAUTH_REFRESH_FAILED`, `401 invalid_token`
- Actions:
  - verify binding with `uxc auth binding match <endpoint_url>`
  - inspect credential with `uxc auth oauth info <credential_id>`
  - run `uxc auth oauth refresh <credential_id>`
  - if refresh fails, run login again

6. OAuth scope failure
- Symptoms: `OAUTH_SCOPE_INSUFFICIENT`, HTTP `403`
- Actions:
  - login again with broader scopes
  - confirm provider/workspace policy grants requested scopes

## OAuth Code Playbooks

`OAUTH_REQUIRED`:
1. verify endpoint/credential mapping with binding match
2. login with the expected credential
3. retry original read operation

`OAUTH_DISCOVERY_FAILED`:
1. check endpoint and network reachability
2. retry login
3. if needed, use explicit provider metadata flags supported by CLI

`OAUTH_TOKEN_EXCHANGE_FAILED`:
1. ensure callback URL/code is complete and unmodified
2. restart login flow

`OAUTH_REFRESH_FAILED`:
1. retry refresh once manually
2. if still failing, re-login (refresh token may be revoked/expired)

`OAUTH_SCOPE_INSUFFICIENT`:
1. login with required scopes
2. rerun original operation

## MCP Probe Note

- For MCP HTTP endpoints, `uxc` may refresh OAuth during protocol probe when probe receives `401`.
- If refresh still fails, expect OAuth-related errors instead of a generic protocol mismatch.

## Retry Guidance

- Retry only idempotent read-like operations by default.
- Suggested backoff: 1s, 2s, 4s (max 3 attempts).
- Do not retry validation errors without payload change.
