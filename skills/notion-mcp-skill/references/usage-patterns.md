# Usage Patterns

All commands assume endpoint `mcp.notion.com/mcp`.
This skill defaults to fixed link command `notion-mcp-cli`.
Create it when missing:

```bash
command -v notion-mcp-cli
uxc link notion-mcp-cli mcp.notion.com/mcp
```

## Discover And Inspect

```bash
notion-mcp-cli -h
notion-mcp-cli notion-fetch -h
```

## Read-First Flows

Search content:

```bash
notion-mcp-cli notion-search query="Q1 plan" query_type=internal
```

Fetch entity by URL/ID:

```bash
notion-mcp-cli notion-fetch id="https://notion.so/your-page-url"
```

List users or teams:

```bash
notion-mcp-cli notion-get-users '{}'
notion-mcp-cli notion-get-teams '{}'
```

## Write Flows (Require Explicit User Confirmation)

Create page:

```bash
notion-mcp-cli notion-create-pages '{
  "pages":[
    {
      "properties":{"title":"Release Notes"},
      "content":"# Release Notes\nInitial draft"
    }
  ]
}'
```

Update page properties:

```bash
notion-mcp-cli notion-update-page '{
  "page_id":"00000000-0000-0000-0000-000000000000",
  "command":"update_properties",
  "properties":{"title":"Updated Title"}
}'
```

Add comment:

```bash
notion-mcp-cli notion-create-comment '{
  "page_id":"00000000-0000-0000-0000-000000000000",
  "rich_text":[{"text":{"content":"Looks good"}}]
}'
```

## Schema Discipline For Database Writes

Before writing to database-backed pages:
1. Fetch database/data source first with `notion-fetch`.
2. Use exact property names from fetched schema.
3. Use expanded formats for date/place fields when required by Notion tool schema.

## Output Parsing

Rely on stable envelope fields:
- Success: `ok == true`, consume `data`
- Failure: `ok == false`, inspect `error.code` and `error.message`

## Fallback Equivalence

- `notion-mcp-cli <operation> ...` is equivalent to `uxc mcp.notion.com/mcp <operation> ...`.
- If link setup is temporarily unavailable, use the `uxc mcp.notion.com/mcp ...` form as a fallback.
