# Run as a Managed Service

Run `uxc` daemon under a service manager when runtime auth environment must stay
stable across requests.

This matters especially for:

- `--secret-op` resolution through daemon execution
- long-running local integrations
- background subscriptions

Use foreground daemon entrypoint:

```bash
uxc daemon _serve
```

`uxc daemon start` is auto-spawn mode and is not the recommended
`systemd`/`launchd` entrypoint.

## Linux (`systemd`)

Recommended flow:

1. create a dedicated service user
2. place runtime env in an env file
3. run `uxc daemon _serve` from the unit
4. validate with `uxc daemon status`

## macOS (`launchd`)

Use a user LaunchAgent that runs:

```bash
/usr/local/bin/uxc daemon _serve
```

## Security Notes

- prefer least-privilege service accounts
- inject secrets through the service manager environment
- restart the service after rotating long-lived tokens
