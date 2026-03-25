# Logging and Troubleshooting

UXC uses structured logging through `tracing`.
Logs go to `stderr` so JSON output on `stdout` remains machine-parseable.

## Levels

```bash
RUST_LOG=info uxc <host> -h
RUST_LOG=debug uxc <host> -h
RUST_LOG=trace uxc <host> -h
```

## Module-Scoped Logging

```bash
RUST_LOG=uxc::adapters::openapi=debug uxc <host> -h
RUST_LOG=uxc::adapters::mcp=trace uxc mcp.deepwiki.com/mcp -h
```

## Suggested Debug Flow

1. Start with `RUST_LOG=info`.
2. Move to `debug` when protocol detection or auth matching is unclear.
3. Use module-scoped `trace` for transport-level behavior.

## Common Cases

- endpoint detection looks wrong
- auth is not applied as expected
- MCP HTTP or OAuth flows fail unexpectedly
