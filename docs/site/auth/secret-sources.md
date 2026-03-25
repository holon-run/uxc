# Secret Sources

UXC supports two layers of non-OAuth auth values:

- one primary `secret`
- optional named `fields`

## Primary Secret Sources

- `literal`: provided directly with `--secret`
- `env`: resolved from an environment variable via `--secret-env`
- `op`: resolved from a 1Password reference via `--secret-op`

## Templated Auth Values

For `api_key` credentials, auth headers and query params can use templates:

- `{{secret}}`
- `{{field:<name>}}`
- `{{env:VAR_NAME}}`
- `{{op://...}}`

## Examples

```bash
uxc auth credential set demo --secret sk-demo-token
uxc auth credential set demo --secret-env DEMO_TOKEN
uxc auth credential set demo --secret-op op://Engineering/demo/token
uxc auth credential set demo --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-env OKX_ACCESS_KEY
uxc auth credential set binance --auth-type api_key --field api_key=env:BINANCE_API_KEY --field secret_key=env:BINANCE_SECRET_KEY
uxc auth credential set flipside --auth-type api_key --query-param "apiKey={{secret}}" --secret-env FLIPSIDE_API_KEY
```

## Behavior Notes

- `--secret`, `--secret-env`, and `--secret-op` are mutually exclusive
- `--field` is repeatable
- `--header` and `--query-param` can be repeated and templated
- resolved values from `env` and `op` are used at runtime and are not stored as plaintext

## 1Password and Daemon Scope

When using `--secret-op`, resolution happens in the daemon execution path.

That means:

- daemon must have a valid 1Password auth context
- if environment changes, restart daemon so it picks up the new environment

## Related

- [MCP HTTP OAuth](./oauth-mcp-http.md)
- [Daemon](../daemon/)
