# Auth Secret Sources

UXC supports three secret source kinds for non-OAuth credentials:

- `literal`: secret is provided directly with `--secret`
- `env`: secret is resolved from environment variable via `--secret-env`
- `op`: secret is resolved from 1Password CLI reference via `--secret-op`

Examples:

```bash
uxc auth credential set demo --secret sk-demo-token
uxc auth credential set demo --secret-env DEMO_TOKEN
uxc auth credential set demo --secret-op op://Engineering/demo/token
```

Notes:

- `--secret`, `--secret-env`, and `--secret-op` are mutually exclusive.
- `auth credential info` and `auth credential list` expose only `secret_source.kind`.
- Resolved values from `env` and `op` are used at runtime and are not persisted as plaintext values.
- `op` mode requires 1Password CLI (`op`) in `PATH` and an active session.
