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

4. Runtime transport failure
- Symptoms: timeout, connection reset, TLS error
- Actions:
  - retry with bounded attempts
  - verify endpoint health with native tooling (`curl`, `grpcurl`)

## Retry Guidance

- Retry only idempotent read-like operations by default.
- Suggested backoff: 1s, 2s, 4s (max 3 attempts).
- Do not retry validation errors without payload change.
