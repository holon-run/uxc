# Logging and Troubleshooting

UXC uses structured logging through `tracing`.
Logs are written to `stderr` so JSON output on `stdout` remains machine-parseable.

## Default Behavior

Default level is effectively warning and above.

```bash
uxc petstore3.swagger.io/api/v3 -h
```

## Set Log Level

## info

```bash
RUST_LOG=info uxc petstore3.swagger.io/api/v3 -h
```

## debug

```bash
RUST_LOG=debug uxc petstore3.swagger.io/api/v3 -h
```

## trace

```bash
RUST_LOG=trace uxc petstore3.swagger.io/api/v3 -h
```

## Module-Scoped Logging

Limit verbosity to selected modules:

```bash
RUST_LOG=uxc::adapters::openapi=debug uxc petstore3.swagger.io/api/v3 -h
RUST_LOG=uxc::adapters::mcp=trace uxc mcp.deepwiki.com/mcp -h
```

## Suggested Debug Flow

1. Re-run with `RUST_LOG=info`.
2. If protocol detection is unclear, move to `RUST_LOG=debug`.
3. For transport-level behavior, use module-scoped `trace`.
4. Keep command output and logs separated:
   - stdout: JSON envelope
   - stderr: diagnostic logs

## Common Cases

## Endpoint detection looks wrong

```bash
RUST_LOG=debug uxc <host> -h
```

Inspect adapter selection and probing sequence in logs.

## Auth not applied as expected

```bash
RUST_LOG=debug uxc auth binding match https://example.com/path
RUST_LOG=debug uxc https://example.com/path -h
```

Check binding match details and header application logs.

## MCP HTTP / OAuth failures

Use debug logs plus OAuth docs:

- [`oauth-mcp-http.md`](oauth-mcp-http.md)

## Related Docs

- Quickstart: [`quickstart.md`](quickstart.md)
- Public no-key endpoints: [`public-endpoints.md`](public-endpoints.md)
- Schema mapping: [`schema-mapping.md`](schema-mapping.md)
