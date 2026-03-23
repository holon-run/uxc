---
name: notion-openapi-skill
description: Operate Notion Public API through UXC with a curated OpenAPI schema for search, block traversal, page reads, content writes, and data source/database inspection. Use when tasks need recursive reads or structured writes that Notion MCP does not expose directly.
---

# Notion Public API Skill

Use this skill to run Notion Public API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, OAuth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.notion.com/v1`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/notion-openapi-skill/references/notion-public.openapi.json`
- A Notion integration token or OAuth credential with content read access.
- For writes, the integration also needs the corresponding Notion insert/update content capabilities.

## Scope

This skill covers a read-first Notion REST surface focused on traversal plus common content writes:

- token identity validation
- title search for pages, data sources, and databases
- page lookup
- block lookup
- recursive traversal via block-children pagination
- page property retrieval
- page creation
- page updates, including trash/restore via `in_trash`
- block append
- block updates
- block deletion
- data source retrieval and query
- legacy database retrieval and query

This skill does **not** cover:

- full Notion REST coverage
- comments, file uploads, webhooks, page move, or schema mutation
- automatic recursive traversal loops inside one single command

## Endpoint And Version

- base URL: `https://api.notion.com/v1`
- required version header for this skill: `Notion-Version: 2026-03-11`

The schema is intentionally curated around traversal and schema discovery. It is not a full dump of the Notion API.

## Authentication

Notion Public API requires:

- `Authorization: Bearer <token>`
- `Notion-Version: 2026-03-11`

### Recommended: dedicated REST credential

If you already have an internal integration token:

```bash
uxc auth credential set notion-openapi \
  --auth-type api_key \
  --header "Authorization=Bearer {{secret}}" \
  --header "Notion-Version=2026-03-11" \
  --secret-env NOTION_API_TOKEN

uxc auth binding add \
  --id notion-openapi \
  --host api.notion.com \
  --path-prefix /v1 \
  --scheme https \
  --credential notion-openapi \
  --priority 100
```

How to get the internal integration token:

1. Open the Notion integrations dashboard and create an internal integration for your workspace.
2. In the integration configuration page, copy the API secret shown by Notion.
3. In Notion UI, open each target page, data source, or database and add this integration via `Share` or `Connections`.
4. Use that API secret as `NOTION_API_TOKEN` or pass it directly to `uxc auth credential set`.

Without connecting the integration to the target content, REST calls may authenticate successfully but still fail with access errors or return incomplete search results.

If you want OAuth-managed tokens for the REST host:

```bash
uxc auth oauth start notion-openapi \
  --endpoint https://api.notion.com/v1 \
  --redirect-uri http://127.0.0.1:8788/callback \
  --client-id <client_id> \
  --scope read

uxc auth oauth complete notion-openapi \
  --session-id <session_id> \
  --authorization-response 'http://127.0.0.1:8788/callback?code=...'

uxc auth credential set notion-openapi \
  --auth-type oauth \
  --header "Authorization=Bearer {{secret}}" \
  --header "Notion-Version=2026-03-11"

uxc auth binding add \
  --id notion-openapi \
  --host api.notion.com \
  --path-prefix /v1 \
  --scheme https \
  --credential notion-openapi \
  --priority 100
```

### Advanced: reuse the same OAuth credential as `notion-mcp`

This is technically possible in `uxc` if the existing credential already has a valid Notion OAuth access token.

Important:
- once an OAuth credential uses custom headers, include `Authorization=Bearer {{secret}}` explicitly
- adding `Notion-Version=2026-03-11` on the shared credential means the same header will also be sent to `mcp.notion.com/mcp`
- that extra header is expected to be harmless, but this is an interoperability assumption rather than an explicit Notion guarantee

Shared-credential setup:

```bash
uxc auth credential set notion-mcp \
  --auth-type oauth \
  --header "Authorization=Bearer {{secret}}" \
  --header "Notion-Version=2026-03-11"

uxc auth binding add \
  --id notion-openapi-shared \
  --host api.notion.com \
  --path-prefix /v1 \
  --scheme https \
  --credential notion-mcp \
  --priority 100
```

Validate the effective mapping when auth looks wrong:

```bash
uxc auth binding match https://api.notion.com/v1
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v notion-openapi-cli`
   - If missing, create it:
     `uxc link notion-openapi-cli https://api.notion.com/v1 --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/notion-openapi-skill/references/notion-public.openapi.json`
   - `notion-openapi-cli -h`

2. Inspect operation schema first:
   - `notion-openapi-cli post:/search -h`
   - `notion-openapi-cli get:/blocks/{block_id}/children -h`
   - `notion-openapi-cli post:/pages -h`
   - `notion-openapi-cli patch:/blocks/{block_id}/children -h`
   - `notion-openapi-cli post:/data_sources/{data_source_id}/query -h`

3. Prefer read validation before broader traversal:
   - `notion-openapi-cli get:/users/me`
   - `notion-openapi-cli post:/search '{"query":"Roadmap","filter":{"property":"object","value":"page"},"page_size":10}'`
   - `notion-openapi-cli get:/blocks/{block_id}/children block_id=<uuid> page_size=100`

4. Traverse recursively outside the API call boundary:
   - use `get:/blocks/{block_id}/children` page by page
   - for every returned child block with `has_children=true`, call `get:/blocks/{block_id}/children` again on that child ID

5. Use data source or legacy database reads to discover schema before property-sensitive queries:
   - `notion-openapi-cli get:/data_sources/{data_source_id} data_source_id=<uuid>`
   - `notion-openapi-cli post:/data_sources/{data_source_id}/query data_source_id=<uuid> '{"page_size":25}'`
   - `notion-openapi-cli get:/databases/{database_id} database_id=<uuid>`

6. Execute writes only after explicit user confirmation:
   - create page: `notion-openapi-cli post:/pages '{...}'`
   - append blocks: `notion-openapi-cli patch:/blocks/{block_id}/children '{...}'`
   - update page or block: `notion-openapi-cli patch:/pages/{page_id} '{...}'`
   - delete block: `notion-openapi-cli delete:/blocks/{block_id} block_id=<uuid>`

## Operation Groups

### Session / Discovery

- `get:/users/me`
- `post:/search`

### Page And Block Traversal

- `get:/pages/{page_id}`
- `get:/pages/{page_id}/properties/{property_id}`
- `get:/blocks/{block_id}`
- `get:/blocks/{block_id}/children`

### Page And Block Writes

- `post:/pages`
- `patch:/pages/{page_id}`
- `patch:/blocks/{block_id}/children`
- `patch:/blocks/{block_id}`
- `delete:/blocks/{block_id}`

### Data Source Reads

- `get:/data_sources/{data_source_id}`
- `post:/data_sources/{data_source_id}/query`

### Legacy Database Reads

- `get:/databases/{database_id}`
- `post:/databases/{database_id}/query`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- This skill fixes `Notion-Version` at the credential/header layer instead of requiring it as an operation argument. Keep the credential header on `2026-03-11` unless you intentionally migrate the whole skill surface.
- This skill is read-first, but it now includes common content writes. Always confirm intent before `post:/pages`, `patch:/pages/{page_id}`, `patch:/blocks/{block_id}/children`, `patch:/blocks/{block_id}`, or `delete:/blocks/{block_id}`.
- On Notion API version `2026-03-11`, `archived` has been replaced by `in_trash` for request and response semantics. Prefer `in_trash` in update payloads.
- `patch:/blocks/{block_id}/children` supports up to 100 new children in one request and up to two levels of nested blocks in a single payload.
- `patch:/blocks/{block_id}` updates block content, but it does not update child lists. Use `patch:/blocks/{block_id}/children` to append nested content.
- Prefer `data_sources` endpoints over legacy `databases` endpoints for new workflows. Keep legacy database reads only for compatibility with older shared links and IDs.
- `get:/blocks/{block_id}/children` returns only one nesting level at a time. Recursive traversal must be performed by repeated calls.
- Notion may return fewer results than `page_size`; always check `has_more` and `next_cursor`.
- `notion-openapi-cli <operation> ...` is equivalent to `uxc https://api.notion.com/v1 --schema-url <notion_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/notion-public.openapi.json`
- Notion API reference: https://developers.notion.com/reference
- Retrieve a database: https://developers.notion.com/reference/retrieve-a-database
- Retrieve a block: https://developers.notion.com/reference/retrieve-a-block
- Versioning: https://developers.notion.com/reference/versioning
