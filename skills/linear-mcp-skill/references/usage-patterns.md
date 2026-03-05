# Linear MCP Skill - Usage Patterns

## Authentication Setup

### Personal API Key (Recommended)
```bash
# Set credential with environment variable
uxc auth credential set linear-mcp --auth-type bearer --secret-env LINEAR_API_KEY

# Or with literal secret (not recommended for security)
uxc auth credential set linear-mcp --auth-type bearer --secret lin_api_xxxx
```

### OAuth Flow
```bash
# Start OAuth login
uxc auth oauth login linear-mcp \
  --endpoint https://api.linear.app/graphql \
  --flow authorization_code \
  --redirect-uri http://127.0.0.1:8788/callback \
  --scope read \
  --scope write

# After user approves, paste callback URL
# Then bind endpoint
uxc auth binding add \
  --id linear-mcp \
  --host api.linear.app \
  --path-prefix /graphql \
  --scheme https \
  --credential linear-mcp \
  --priority 100
```

## Link Setup
```bash
# Create link command
uxc link linear-mcp-cli https://api.linear.app/graphql

# Verify
linear-mcp-cli -h
```

## Query Examples

### List Issues
```bash
linear-mcp-cli query/issues first=20
```

### Filter Issues by Team
```bash
linear-mcp-cli query/issues filter='{"team":{"id":{"eq":"TEAM_ID"}}}'
```

### Get Single Issue
```bash
linear-mcp-cli query/issue id=ISSUE_123
```

### List Teams
```bash
linear-mcp-cli query/teams
```

### List Projects
```bash
linear-mcp-cli query/projects first=10
```

## Mutation Examples

### Create Issue
```bash
linear-mcp-cli mutation/issueCreate '{
  "issueCreateInput": {
    "teamId": "TEAM_ID",
    "title": "New Feature Request",
    "description": "Description here",
    "priority": 2
  }
}'
```

### Update Issue
```bash
linear-mcp-cli mutation/issueUpdate '{
  "id": "ISSUE_ID",
  "input": {
    "title": "Updated Title",
    "description": "Updated description"
  }
}'
```

### Archive Issue
```bash
linear-mcp-cli mutation/issueArchive id=ISSUE_ID
```

### Add Comment
```bash
linear-mcp-cli mutation/commentCreate '{
  "commentCreateInput": {
    "issueId": "ISSUE_ID",
    "body": "Comment body"
  }
}'
```

## Error Handling

### Invalid API Key
```
{"ok": false, "error": {"code": "UNAUTHENTICATED", "message": "API key invalid"}}
```
Fix: Check or regenerate API key at https://linear.app/settings/api

### Rate Limiting
```
{"ok": false, "error": {"code": "RATE_LIMITED", "message": "Too many requests"}}
```
Fix: Wait and retry, or reduce request frequency

### Invalid Operation
```
{"ok": false, "error": {"code": "INVALID_ARGUMENT", "message": "Invalid issue ID"}}
```
Fix: Verify the ID format and existence
