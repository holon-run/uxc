# Notion Public API Skill - Usage Patterns

All commands below assume the credential injects both:

- `Authorization: Bearer <token>`
- `Notion-Version: 2026-03-11`

## Link Setup

```bash
command -v notion-openapi-cli
uxc link notion-openapi-cli https://api.notion.com/v1 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/notion-openapi-skill/references/notion-public.openapi.json
notion-openapi-cli -h
```

## Auth Setup

Existing internal integration token:

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

Internal integration token source:

- Create an internal integration in Notion's integrations dashboard.
- Copy the API secret from the integration configuration page.
- Connect that integration to each target page, data source, or database in Notion UI.
- Export it as `NOTION_API_TOKEN` or substitute the secret directly in the credential command.

Shared OAuth credential from Notion MCP:

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

## Read Examples

```bash
# Validate token identity
notion-openapi-cli get:/users/me

# Search pages by title
notion-openapi-cli post:/search \
  '{"query":"Roadmap","filter":{"property":"object","value":"page"},"page_size":10}'

# Retrieve one page
notion-openapi-cli get:/pages/{page_id} page_id=<page_uuid>

# Retrieve one block
notion-openapi-cli get:/blocks/{block_id} block_id=<block_uuid>

# List one level of child blocks
notion-openapi-cli get:/blocks/{block_id}/children \
  block_id=<block_uuid> \
  page_size=100

# Continue block traversal with a pagination cursor
notion-openapi-cli get:/blocks/{block_id}/children \
  block_id=<block_uuid> \
  page_size=100 \
  start_cursor=<cursor>

# Read one page property item
notion-openapi-cli get:/pages/{page_id}/properties/{property_id} \
  page_id=<page_uuid> \
  property_id=<property_id_or_name>

# Read current data source schema
notion-openapi-cli get:/data_sources/{data_source_id} data_source_id=<data_source_uuid>

# Query pages in a data source
notion-openapi-cli post:/data_sources/{data_source_id}/query \
  data_source_id=<data_source_uuid> \
  '{"page_size":25}'

# Query a data source with filter + sort
notion-openapi-cli post:/data_sources/{data_source_id}/query \
  data_source_id=<data_source_uuid> \
  '{"filter":{"property":"Status","status":{"equals":"In progress"}},"sorts":[{"property":"Updated time","direction":"descending"}],"page_size":25}'

# Compatibility path for older database-based workflows
notion-openapi-cli get:/databases/{database_id} database_id=<database_uuid>
```

## Write Examples (Confirm Intent First)

```bash
# Create a page under a parent page
notion-openapi-cli post:/pages \
  '{"parent":{"page_id":"<parent_page_uuid>"},"properties":{"title":[{"text":{"content":"Draft from UXC"}}]}}'

# Create a page under a data source
notion-openapi-cli post:/pages \
  '{"parent":{"data_source_id":"<data_source_uuid>"},"properties":{"Name":{"title":[{"text":{"content":"Task from UXC"}}]}}}'

# Update page properties or trash state
notion-openapi-cli patch:/pages/{page_id} \
  page_id=<page_uuid> \
  '{"properties":{"Status":{"status":{"name":"In progress"}}},"in_trash":false}'

# Append paragraph blocks to a page or block
notion-openapi-cli patch:/blocks/{block_id}/children \
  block_id=<parent_block_or_page_uuid> \
  '{"children":[{"object":"block","type":"paragraph","paragraph":{"rich_text":[{"type":"text","text":{"content":"Hello from UXC"}}]}}]}'

# Append after a specific sibling using 2026-03-11 position syntax
notion-openapi-cli patch:/blocks/{block_id}/children \
  block_id=<parent_block_uuid> \
  '{"children":[{"object":"block","type":"to_do","to_do":{"rich_text":[{"type":"text","text":{"content":"Follow-up item"}}]}}],"position":{"type":"after_block","after_block":{"id":"<sibling_block_uuid>"}}}'

# Update one block's content
notion-openapi-cli patch:/blocks/{block_id} \
  block_id=<block_uuid> \
  '{"paragraph":{"rich_text":[{"type":"text","text":{"content":"Updated paragraph text"}}]}}'

# Delete (trash/archive) a block
notion-openapi-cli delete:/blocks/{block_id} block_id=<block_uuid>
```

## Recursive Traversal Pattern

```bash
# 1. Retrieve one parent block or page
notion-openapi-cli get:/blocks/{block_id} block_id=<root_block_uuid>

# 2. Retrieve its first-level children
notion-openapi-cli get:/blocks/{block_id}/children block_id=<root_block_uuid> page_size=100

# 3. For each returned child block where has_children=true, call:
notion-openapi-cli get:/blocks/{block_id}/children block_id=<child_block_uuid> page_size=100
```

## Fallback Equivalence

- `notion-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.notion.com/v1 --schema-url <notion_openapi_schema> <operation> ...`.
