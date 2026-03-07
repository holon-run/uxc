# Authentication Configuration Guide

Complete guide for configuring credentials and auth bindings for different API authentication patterns.

## Credential Types

UXC supports three non-OAuth credential types:

### 1. API Key with Custom Headers (Most Flexible)

Use when the API requires specific header names or formats:

```bash
# Single header with secret
uxc auth credential set <credential_id> \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret "api_key_value"

# Multiple headers
uxc auth credential set <credential_id> \
  --auth-type api_key \
  --header "X-API-Key:{{secret}}" \
  --header "X-API-Secret:{{env:API_SECRET}}" \
  --secret-env API_KEY

# Using 1Password
uxc auth credential set <credential_id> \
  --auth-type api_key \
  --header "Authorization:Bearer {{secret}}" \
  --secret-op "op://Engineering/api/token"
```

**Header Template Syntax:**
- `{{secret}}` - Resolved from credential secret source
- `{{env:VAR_NAME}}` - Resolved from environment variable
- `{{op://path/to/secret}}` - Resolved through 1Password CLI

### 2. Bearer Token (Standard OAuth2 Format)

Use when API accepts standard `Authorization: Bearer <token>` format:

```bash
# With literal secret
uxc auth credential set <credential_id> \
  --auth-type bearer \
  --secret "bearer_token_value"

# From environment variable
uxc auth credential set <credential_id> \
  --auth-type bearer \
  --secret-env BEARER_TOKEN

# From 1Password
uxc auth credential set <credential_id> \
  --auth-type bearer \
  --secret-op "op://Private/bearer/token"
```

**Note:** This automatically adds `Bearer ` prefix to the Authorization header.

### 3. Secret Sources

All credential types support three secret source kinds:

#### Literal Secret
```bash
--secret "actual_secret_value"
```
- Stored in plaintext in credentials.json
- **Only for development/testing**
- Avoid for production credentials

#### Environment Variable
```bash
--secret-env CREDENTIAL_NAME
```
- Secret resolved from environment variable at runtime
- More secure than literal
- Requires environment variable to be set in daemon's environment

#### 1Password Reference
```bash
--secret-op "op://Vault/item/field"
```
- Most secure option
- Requires 1Password CLI (`op`) installed and authenticated
- Resolved at request execution time

## Auth Bindings

Credentials must be bound to endpoint patterns for automatic use:

```bash
uxc auth binding add \
  --id <binding_id> \
  --host <api_host> \
  --path-prefix <path_prefix> \
  --scheme https \
  --credential <credential_id> \
  --priority 100
```

**Binding Resolution:**
1. Explicit `--auth <credential_id>` (highest priority)
2. Best match by `scheme + host + path_prefix + priority`
3. First match wins when priorities are equal

**Check binding for an endpoint:**
```bash
uxc auth binding match <endpoint>
```

## Common Authentication Patterns

### Pattern 1: Linear API (Direct API Key, No Prefix)

Linear expects `Authorization: lin_api_XXX` (no prefix):

```bash
uxc auth credential set linear-mcp \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret "lin_api_XXX"

uxc auth binding add \
  --id linear-mcp \
  --host api.linear.app \
  --path-prefix /graphql \
  --scheme https \
  --credential linear-mcp \
  --priority 100
```

**Wrong:** `--auth-type bearer` (adds "Bearer " prefix, rejected by Linear)

### Pattern 2: Standard Bearer Token

APIs that accept `Authorization: Bearer <token>`:

```bash
uxc auth credential set myapi \
  --auth-type bearer \
  --secret-env MYAPI_TOKEN
```

### Pattern 3: Custom API Key Header

APIs that use non-standard header names:

```bash
uxc auth credential set custom-api \
  --auth-type api_key \
  --header "X-API-Key:{{secret}}" \
  --secret-env CUSTOM_API_KEY
```

### Pattern 4: Multiple Headers

APIs requiring multiple authentication headers:

```bash
uxc auth credential set complex-api \
  --auth-type api_key \
  --header "X-API-Key:{{secret}}" \
  --header "X-API-Secret:{{env:API_SECRET}}" \
  --secret-env API_KEY
```

## Troubleshooting

### Error: "Bearer token" prefix rejected

**Symptom:** API returns error about removing Bearer prefix

**Cause:** Using `--auth-type bearer` when API expects raw token

**Solution:**
```bash
# Wrong
uxc auth credential set myapi --auth-type bearer --secret "token"

# Correct
uxc auth credential set myapi \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret "token"
```

### Error: Credential not found

**Symptom:** `Credential 'xxx' not found`

**Check:** List available credentials
```bash
uxc auth credential list
```

### Error: No binding matched

**Symptom:** Auth failures despite valid credential

**Check:** Verify binding exists and matches
```bash
uxc auth binding list
uxc auth binding match <endpoint>
```

**Solution:** Create or fix binding
```bash
uxc auth binding add \
  --id my-binding \
  --host api.example.com \
  --credential my-credential \
  --priority 100
```

### Error: Environment variable not set

**Symptom:** `expects env var 'XXX' but it is not set`

**Cause:** Credential uses `--secret-env` but variable not exported in daemon's environment

**Solutions:**
1. Set environment variable before starting daemon:
   ```bash
   export MY_API_KEY="value"
   uxc daemon restart
   ```

2. Or switch to literal secret (less secure):
   ```bash
   uxc auth credential set mycred --secret "actual_value"
   ```

3. Or switch to 1Password (most secure):
   ```bash
   uxc auth credential set mycred --secret-op "op://Vault/item/field"
   ```

### Error: 1Password CLI not found

**Symptom:** `'op' CLI was not found in PATH`

**Cause:** 1Password CLI not installed or not in daemon's PATH

**Solution:**
1. Install 1Password CLI: https://developer.1password.com/docs/cli/get-started
2. Ensure it's in PATH for daemon process
3. Authenticate: `eval "$(op signin)"` or use service account token

## Verification Steps

After configuring authentication, verify with:

1. **Check credential:**
   ```bash
   uxc auth credential info <credential_id>
   ```

2. **Check binding:**
   ```bash
   uxc auth binding match <endpoint>
   ```

3. **Test with read operation:**
   ```bash
   uxc <endpoint> <read_operation>
   ```

4. **Test with explicit auth:**
   ```bash
   uxc --auth <credential_id> <endpoint> <read_operation>
   ```

## Best Practices

1. **Use environment variables or 1Password** for production credentials
2. **Create bindings** for frequently-used endpoints
3. **Test with read operations first** before write operations
4. **Check credential info** to verify auth_headers are configured correctly
5. **Restart daemon** after changing environment variables
6. **Use priority** when multiple bindings could match (higher = preferred)

## See Also

- OAuth flow: `oauth-and-binding.md`
- Error handling: `error-handling.md`
- Usage patterns: `usage-patterns.md`
