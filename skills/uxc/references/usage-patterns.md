# Usage Patterns

## Progressive Flow (Default)

1. Discover operations:

```bash
uxc <host> -h
```

2. Inspect operation input/output shape:

```bash
uxc <host> <operation> -h
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
<provider>-mcp-cli -h
<provider>-mcp-cli <operation> -h
<provider>-mcp-cli <operation> field=value
```

Examples:

```bash
notion-mcp-cli -h
# Equivalent:
uxc mcp.notion.com/mcp -h
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

## Input Modes

### Bare JSON positional payload

```bash
uxc <host> <operation> '{"field":"value"}'
```

### Key-value arguments

```bash
uxc <host> <operation> field=value
```

Do not pass raw JSON via `--args`; use bare JSON positional payload instead.

## Host-Level Help

```bash
uxc <host> -h
```

Use this when you need quick discovery context before choosing an operation.

## Auth-Protected Flow

1. Confirm mapping:

```bash
uxc auth binding match <endpoint>
```

2. Run intended read call directly (use as runtime validation).

3. If auth fails, recover in order:

```bash
uxc auth oauth info <credential_id>
uxc auth oauth refresh <credential_id>
uxc auth oauth login <credential_id> --endpoint <endpoint> --flow authorization_code
```

4. If multiple bindings match, verify explicit credential:

```bash
uxc --auth <credential_id> <endpoint> <operation> '{...}'
```

## Automation Safety Rules

- Keep JSON as default output for machine parsing.
- Treat stderr logs as diagnostic only; parse stdout JSON envelope.
- Start with smallest valid payload before expanding optional fields.
