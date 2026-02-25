# Usage Patterns

## Progressive Flow (Default)

1. Discover operations:

```bash
uxc <host> list
```

2. Inspect operation input/output shape:

```bash
uxc <host> describe <operation>
```

3. Execute with minimal valid payload:

```bash
uxc <host> <operation> --json '{"field":"value"}'
```

4. Parse success/failure envelope:

```bash
uxc <host> <operation> --json '{"field":"value"}' | jq '.ok, .kind, .data'
```

## Conflict-Safe Flow

If operation name collides with CLI keywords (`help`, `list`, `describe`), use explicit `call`:

```bash
uxc <host> call <operation> --json '{"field":"value"}'
```

## Host-Level Help

```bash
uxc <host> help
```

Use this when you need quick discovery context before full `list` + `describe`.

## Automation Safety Rules

- Keep JSON as default output for machine parsing.
- Treat stderr logs as diagnostic only; parse stdout JSON envelope.
- Start with smallest valid payload before expanding optional fields.
