# Schema Mapping

Some services expose runtime APIs and OpenAPI schemas at different URLs.

UXC handles that with a layered strategy:

1. `--schema-url` CLI override
2. user mapping file
3. builtin mappings
4. default OpenAPI probing

## CLI Override

```bash
uxc https://api.github.com -h \
  --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json
```

`uxc <url>` remains the execution target.
`--schema-url` only changes schema discovery.

## User Mapping File

Path:

```text
~/.uxc/schema_mappings.json
```

Typical fields:

- `host`
- `path_prefix`
- `schema_url`
- `priority`
- `enabled`

## Matching Rules

- host matching is exact and case-insensitive
- path prefix must match the target path
- user mapping beats builtin mapping
- higher priority wins
- longer path prefix wins when priority ties
