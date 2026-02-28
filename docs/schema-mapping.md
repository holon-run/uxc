# OpenAPI Schema Mapping

Some services expose runtime APIs and OpenAPI schemas at different URLs.

- Runtime endpoint: where requests are executed (`uxc <url> ...`)
- Schema URL: where operation metadata is discovered

UXC supports this with a layered strategy:

1. `--schema-url` CLI override (highest priority)
2. User mapping file (`~/.uxc/schema_mappings.json`)
3. Builtin mappings (for known services)
4. Default OpenAPI probing (`/openapi.json`, `/swagger.json`, ...)

## CLI Override

```bash
uxc https://api.github.com -h \
  --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json
```

`uxc <url>` stays the execution target. `--schema-url` only changes schema discovery.

## User Mapping File

Path: `~/.uxc/schema_mappings.json`

Example:

```json
{
  "version": 1,
  "openapi": [
    {
      "host": "api.github.com",
      "path_prefix": "/",
      "schema_url": "https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json",
      "priority": 100
    }
  ]
}
```

Field notes:

- `host`: exact host match (required)
- `path_prefix`: optional path prefix filter (default all paths)
- `schema_url`: OpenAPI document URL (required)
- `priority`: higher wins when multiple rules match (default `0`)
- `enabled`: optional, defaults to `true`

## Matching Rules

- Host matching is exact (case-insensitive).
- Path prefix must match the target URL path.
- If multiple rules match:
  1. user mapping beats builtin mapping
  2. higher `priority` wins
  3. longer `path_prefix` wins

## Override Mapping File Path

For CI or testing:

```bash
UXC_SCHEMA_MAPPINGS_FILE=/tmp/schema_mappings.json uxc https://service.example.com -h
```
