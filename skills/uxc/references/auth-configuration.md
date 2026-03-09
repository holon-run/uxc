# Authentication Configuration Guide

Use this guide for non-OAuth authentication. The model now has two tracks:

- simple auth: one primary `secret`
- complex auth: multiple named `fields`

Bindings can optionally attach a typed signer for APIs that require request signing.

## The Two Tracks

### Track 1: Simple `secret`

Keep using `--secret`, `--secret-env`, or `--secret-op` when the provider only needs one secret value.

Good fits:

- bearer tokens
- one API key header
- one API key query param
- stdio `--inject-env NAME={{secret}}`

Examples:

```bash
uxc auth credential set deepwiki \
  --auth-type bearer \
  --secret-env DEEPWIKI_TOKEN

uxc auth credential set okx-market \
  --auth-type api_key \
  --secret-env OKX_ACCESS_KEY \
  --api-key-header OK-ACCESS-KEY

uxc auth credential set flipside \
  --auth-type api_key \
  --query-param "apiKey={{secret}}" \
  --secret-env FLIPSIDE_API_KEY
```

### Track 2: Named `fields`

Use `--field` when one credential needs multiple values.

Good fits:

- `api_key + secret_key`
- `api_key + private_key`
- `api_key + passphrase`
- multiple headers that should draw from different values
- signer-backed APIs

Example:

```bash
uxc auth credential set exchange \
  --auth-type api_key \
  --field api_key=env:EXCHANGE_API_KEY \
  --field secret_key=env:EXCHANGE_SECRET_KEY
```

## Credential Sources And Templates

### Primary secret sources

`--secret`, `--secret-env`, and `--secret-op` are mutually exclusive.

- literal:
  ```bash
  --secret "actual_secret_value"
  ```
- env:
  ```bash
  --secret-env CREDENTIAL_NAME
  ```
- 1Password:
  ```bash
  --secret-op "op://Vault/item/field"
  ```

### Named field sources

`--field` is repeatable. Each field uses an explicit source form:

```bash
--field api_key=literal:abc123
--field api_key=env:BINANCE_API_KEY
--field private_key=op://Trading/binance/private_key
```

`--field` is not supported for OAuth credentials.

### Template syntax

- `{{secret}}`: primary secret source
- `{{field:<name>}}`: named field on the credential
- `{{env:VAR_NAME}}`: direct environment variable lookup
- `{{op://...}}`: direct 1Password lookup

Examples:

```bash
uxc auth credential set linear \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret-env LINEAR_API_KEY

uxc auth credential set complex-api \
  --auth-type api_key \
  --field api_key=env:API_KEY \
  --field api_secret=env:API_SECRET \
  --header "X-API-Key:{{field:api_key}}" \
  --header "X-API-Secret:{{field:api_secret}}"
```

## Common Credential Patterns

### Bearer token

```bash
uxc auth credential set myapi \
  --auth-type bearer \
  --secret-env MYAPI_TOKEN
```

### Raw token in `Authorization`

Use this when the provider rejects `Bearer ` prefix:

```bash
uxc auth credential set linear \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret-env LINEAR_API_KEY
```

### API key in one custom header

```bash
uxc auth credential set custom-api \
  --auth-type api_key \
  --header "X-API-Key:{{secret}}" \
  --secret-env CUSTOM_API_KEY
```

### Multiple headers from different values

```bash
uxc auth credential set okx-advanced \
  --auth-type api_key \
  --field access_key=env:OKX_ACCESS_KEY \
  --field passphrase=env:OKX_PASSPHRASE \
  --header "OK-ACCESS-KEY:{{field:access_key}}" \
  --header "OK-ACCESS-PASSPHRASE:{{field:passphrase}}"
```

### Query-string API key

```bash
uxc auth credential set flipside \
  --auth-type api_key \
  --query-param "apiKey={{secret}}" \
  --secret-env FLIPSIDE_API_KEY
```

## Bindings

Credentials do nothing until they are bound to endpoint patterns:

```bash
uxc auth binding add \
  --id <binding_id> \
  --host <api_host> \
  --path-prefix <path_prefix> \
  --scheme https \
  --credential <credential_id> \
  --priority 100
```

Resolution order:

1. explicit `--auth <credential_id>`
2. best binding match by `scheme + host + path_prefix + priority`
3. first match wins if all else is equal

Check the effective match:

```bash
uxc auth binding match <endpoint>
```

## Request Signers

Some HTTP APIs need more than static headers. In those cases:

- store values on the credential as `fields`
- attach the signer on the binding with `--signer-json`

Current typed signer kinds:

- `hmac_query_v1`
- `ed25519_query_v1`

### HMAC signed query example

```bash
uxc auth credential set binance-hmac \
  --auth-type api_key \
  --field api_key=env:BINANCE_API_KEY \
  --field secret_key=env:BINANCE_SECRET_KEY

uxc auth binding add \
  --id binance-hmac \
  --host api.binance.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-hmac \
  --signer-json '{"kind":"hmac_query_v1","algorithm":"hmac_sha256","signing_field":"secret_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"hex","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100
```

### Ed25519 signed query example

```bash
uxc auth credential set binance-ed25519 \
  --auth-type api_key \
  --field api_key=env:BINANCE_API_KEY \
  --field private_key=env:BINANCE_ED25519_PRIVATE_KEY

uxc auth binding add \
  --id binance-ed25519 \
  --host api.binance.com \
  --path-prefix /api/v3 \
  --scheme https \
  --credential binance-ed25519 \
  --signer-json '{"kind":"ed25519_query_v1","algorithm":"ed25519","signing_field":"private_key","key_field":"api_key","key_placement":"header","key_name":"X-MBX-APIKEY","signature_param":"signature","signature_encoding":"base64","timestamp_param":"timestamp","timestamp_unit":"milliseconds","canonicalization":{"mode":"preserve_order"}}' \
  --priority 100
```

## Troubleshooting

### Error: "Bearer token" prefix rejected

Cause: using `--auth-type bearer` when the provider expects a raw token in `Authorization`.

Use:

```bash
uxc auth credential set myapi \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret "token"
```

### Error: Credential not found

```bash
uxc auth credential list
```

### Error: No binding matched

```bash
uxc auth binding list
uxc auth binding match <endpoint>
```

### Error: Environment variable not set

If the credential uses env-backed `secret` or `fields`, export the variable and restart daemon:

```bash
export MY_API_KEY="value"
uxc daemon restart
```

### Error: `-1022` or provider-specific invalid signature

Check all three:

1. the binding matched the intended path
2. the `API key` and signing material come from the same provider key record
3. the signer kind matches the provider contract

Useful checks:

```bash
uxc auth credential info <credential_id>
uxc auth binding match <endpoint>
```

### Error: 1Password CLI not found

Install `op`, ensure it is on the daemon's `PATH`, and authenticate before runtime use.

## Verification Steps

After configuring auth:

1. inspect the credential
   ```bash
   uxc auth credential info <credential_id>
   ```
2. inspect binding match
   ```bash
   uxc auth binding match <endpoint>
   ```
3. test a read operation first
   ```bash
   uxc <endpoint> <read_operation>
   ```
4. optionally force the credential explicitly
   ```bash
   uxc --auth <credential_id> <endpoint> <read_operation>
   ```

## Best Practices

1. Keep `--secret` for single-secret auth. Do not force everything into `fields`.
2. Use `fields` for multi-value credentials and signer-backed APIs.
3. Attach signers to bindings, not to endpoint URLs or shell wrappers.
4. Prefer environment variables or 1Password over literal values.
5. Test with reads before writes.
6. Restart daemon after environment changes.

## See Also

- OAuth flow: `oauth-and-binding.md`
- Error handling: `error-handling.md`
- Usage patterns: `usage-patterns.md`
