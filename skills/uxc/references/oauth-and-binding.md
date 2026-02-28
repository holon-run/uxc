# OAuth And Binding

Use this guide for OAuth-backed endpoints across protocols and providers.

## Credential And Binding Model

- Credential stores auth material (`oauth`, bearer, api key).
- Binding maps endpoint patterns to credential IDs.
- Runtime resolution order:
  1. explicit `--auth <credential_id>`
  2. binding match by `scheme + host + path_prefix + priority`

## Setup Flow

1. Login:

```bash
uxc auth oauth login <credential_id> \
  --endpoint <endpoint> \
  --flow authorization_code
```

2. Bind endpoint to credential:

```bash
uxc auth binding add \
  --id <binding_id> \
  --host <host> \
  --path-prefix <path_prefix> \
  --scheme <scheme> \
  --credential <credential_id> \
  --priority 100
```

3. Verify binding:

```bash
uxc auth binding match <endpoint>
```

`<endpoint>` accepts either:
- shorthand host/path like `mcp.notion.com/mcp`
- full URL like `https://mcp.notion.com/mcp`

## Validation Strategy

- Prefer local precheck with `binding match`.
- Do not add redundant preflight read calls by default.
- Use first real read operation as runtime validation.

## Auto Refresh Behavior

For OAuth credentials, `uxc` may refresh automatically:

1. before request when token is near expiry
2. after a `401` retry path in runtime calls
3. during MCP HTTP probe when endpoint returns `401` and OAuth profile is available

If refresh succeeds, call continues with new token.

## Manual Recovery Commands

```bash
uxc auth oauth info <credential_id>
uxc auth oauth refresh <credential_id>
uxc auth oauth logout <credential_id>
```

Use manual `refresh` when troubleshooting or when auto-refresh cannot recover.

## Multi-Binding Troubleshooting

If auth failures persist:

1. list bindings:
   - `uxc auth binding list`
2. confirm current default match:
   - `uxc auth binding match <endpoint>`
3. verify candidate credentials explicitly:
   - `uxc --auth <credential_id> <endpoint> <same_read_operation> ...`
4. remove only bindings confirmed stale/invalid.

## Common Error Codes

- `OAUTH_REQUIRED`
- `OAUTH_DISCOVERY_FAILED`
- `OAUTH_TOKEN_EXCHANGE_FAILED`
- `OAUTH_REFRESH_FAILED`
- `OAUTH_SCOPE_INSUFFICIENT`

See `references/error-handling.md` for full recovery playbooks.
