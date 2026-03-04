# Auth Secret Sources

UXC supports three secret source kinds for non-OAuth credentials:

- `literal`: secret is provided directly with `--secret`
- `env`: secret is resolved from environment variable via `--secret-env`
- `op`: secret is resolved from 1Password CLI reference via `--secret-op`

For `api_key` credentials, you can also configure custom auth headers with templates:

- `{{secret}}`: resolved from the credential secret source
- `{{env:VAR_NAME}}`: resolved from environment variable
- `{{op://...}}`: resolved through 1Password CLI secret reference

Examples:

```bash
uxc auth credential set demo --secret sk-demo-token
uxc auth credential set demo --secret-env DEMO_TOKEN
uxc auth credential set demo --secret-op op://Engineering/demo/token
uxc auth credential set demo --auth-type api_key --api-key-header OK-ACCESS-KEY --secret-env OKX_ACCESS_KEY
uxc auth credential set demo --auth-type api_key --header "OK-ACCESS-KEY={{secret}}" --header "OK-ACCESS-PASSPHRASE={{env:OKX_PASSPHRASE}}"
```

## Behavior

- `--secret`, `--secret-env`, and `--secret-op` are mutually exclusive.
- `--api-key-header` is a shortcut for one header using `{{secret}}`.
- `--header` can be repeated and supports `{{secret}}`, `{{env:...}}`, and `{{op://...}}`.
- `auth credential info` and `auth credential list` expose `secret_source.kind` and `auth_headers` (header names with masked values), but not underlying secret values.
- Resolved values from `env` and `op` are used at runtime and are not persisted as plaintext values.
- `op` mode requires 1Password CLI (`op`) in `PATH`.
- `op` references are resolved during request execution, not at `auth credential set` time.

## 1Password Prerequisites

For interactive user sessions:

```bash
eval "$(op signin)"
op whoami
```

For non-interactive environments (agents/CI), prefer Service Account token:

```bash
export OP_SERVICE_ACCOUNT_TOKEN='ops_...'
op whoami
```

## Daemon and Environment Scope

Endpoint calls are handled by `uxc` daemon. `--secret-op` is resolved in the daemon execution path.

This means:

- Daemon must have usable 1Password auth context (`OP_SERVICE_ACCOUNT_TOKEN` or a valid session).
- If daemon was started before you exported new env vars, restart daemon to pick up the new environment.

```bash
uxc daemon stop
# ensure env is set in this shell
uxc https://petstore3.swagger.io/api/v3/openapi.json -h --auth <credential_id>
```

For long-running use, run daemon as a managed service (for example `systemd`/`launchd`) and inject `OP_SERVICE_ACCOUNT_TOKEN` in the service environment.
See [`daemon-service.md`](daemon-service.md) for setup templates.

## Recommended Security Model (Service Account)

- Create a dedicated vault (for example `agents`) with only required secrets.
- Grant Service Account read-only access to that vault (`read_items`).
- Do not grant broad vault access or write permissions unless required.
- Rotate `OP_SERVICE_ACCOUNT_TOKEN` regularly.

## Troubleshooting

- Error: `'op' CLI was not found in PATH`
  - Install 1Password CLI and ensure `op` is on `PATH` for daemon process.
- Error: `Failed to resolve 1Password secret ...`
  - Check `op whoami` in the same runtime environment.
  - Validate the reference with `op read op://...`.
  - If env changed, restart daemon.

## Manual E2E

See reusable manual test case:

- [`e2e/manual/1password-secret-op/README.md`](../e2e/manual/1password-secret-op/README.md)
