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
uxc <host> <operation> field=value
```

4. Parse success/failure envelope:

```bash
uxc <host> <operation> field=value | jq '.ok, .kind, .data'
```

## Wrapper Pattern (Link-First)

For provider-specific wrapper skills, use a fixed local link command as the default interface:

```bash
command -v <provider>-mcp-cli
# If missing:
uxc link <provider>-mcp-cli <host>
<provider>-mcp-cli list
<provider>-mcp-cli describe <operation>
<provider>-mcp-cli <operation> field=value
```

Examples:

```bash
notion-mcp-cli list
# Equivalent:
uxc mcp.notion.com/mcp list
```

```bash
context7-mcp-cli query-docs libraryId=/reactjs/react.dev query=useState
# Equivalent:
uxc mcp.context7.com/mcp query-docs libraryId=/reactjs/react.dev query=useState
```

### Conflict Handling For Wrapper Skills

- Fixed link command names are decided by skill authors at development time.
- Do not dynamically rename link commands at runtime.
- If a conflicting command name exists and cannot be safely reused, stop and ask a maintainer to update the skill's fixed command name.

## Conflict-Safe Flow

If operation name collides with CLI keywords (`help`, `list`, `describe`), use explicit `call`:

```bash
uxc <host> call <operation> field=value
uxc <host> call <operation> '{"field":"value"}'
```

## Input Modes

### Bare JSON positional payload

```bash
uxc <host> <operation> '{"field":"value"}'
uxc <host> call <operation> '{"field":"value"}'
```

### Key-value arguments

```bash
uxc <host> <operation> field=value
```

### Explicit JSON flag (fallback)

```bash
uxc <host> <operation> --input-json '{"field":"value"}'
```

### Precedence and conflict

- Use exactly one JSON payload source:
  - bare positional JSON, or
  - `--input-json`
- Supplying both should fail with `INVALID_ARGUMENT`.
- Do not pass raw JSON via `--args`; use bare JSON positional payload instead.

## Host-Level Help

```bash
uxc <host> help
```

Use this when you need quick discovery context before full `list` + `describe`.

## Auth-Protected Flow

1. Confirm mapping:

```bash
uxc auth binding match <endpoint_url>
```

2. Run intended read call directly (use as runtime validation).

3. If auth fails, recover in order:

```bash
uxc auth oauth info <credential_id>
uxc auth oauth refresh <credential_id>
uxc auth oauth login <credential_id> --endpoint <endpoint_url> --flow authorization_code
```

4. If multiple bindings match, verify explicit credential:

```bash
uxc --auth <credential_id> <endpoint_url> <operation> '{...}'
```

## Automation Safety Rules

- Keep JSON as default output for machine parsing.
- Treat stderr logs as diagnostic only; parse stdout JSON envelope.
- Start with smallest valid payload before expanding optional fields.
