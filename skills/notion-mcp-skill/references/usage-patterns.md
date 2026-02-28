# Usage Patterns

All commands assume endpoint `https://mcp.notion.com/mcp`.
If shortcut exists, prefer:

```bash
notion-mcp-cli <operation> ...
```

Create shortcut:

```bash
uxc link notion-mcp-cli https://mcp.notion.com/mcp
```

## Discover And Inspect

```bash
uxc https://mcp.notion.com/mcp list
uxc https://mcp.notion.com/mcp describe notion-fetch
```

## Read-First Flows

Search content:

```bash
uxc https://mcp.notion.com/mcp notion-search query="Q1 plan" query_type=internal
```

Fetch entity by URL/ID:

```bash
uxc https://mcp.notion.com/mcp notion-fetch id="https://notion.so/your-page-url"
```

List users or teams:

```bash
uxc https://mcp.notion.com/mcp notion-get-users '{}'
uxc https://mcp.notion.com/mcp notion-get-teams '{}'
```

## Write Flows (Require Explicit User Confirmation)

Create page:

```bash
uxc https://mcp.notion.com/mcp notion-create-pages '{
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
uxc https://mcp.notion.com/mcp notion-update-page '{
  "page_id":"00000000-0000-0000-0000-000000000000",
  "command":"update_properties",
  "properties":{"title":"Updated Title"}
}'
```

Add comment:

```bash
uxc https://mcp.notion.com/mcp notion-create-comment '{
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
