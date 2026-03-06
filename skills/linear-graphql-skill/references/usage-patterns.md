# Linear GraphQL Skill - Usage Patterns

## Authentication Setup

### Personal API Key (Recommended)
```bash
# Set credential with environment variable
uxc auth credential set linear-graphql \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret-env LINEAR_API_KEY

# Or with literal secret (not recommended for security)
uxc auth credential set linear-graphql \
  --auth-type api_key \
  --header "Authorization:{{secret}}" \
  --secret lin_api_xxxx
```

### OAuth Flow
```bash
# Start OAuth login
uxc auth oauth start linear-graphql \
  --endpoint https://api.linear.app/graphql \
  --redirect-uri http://127.0.0.1:8788/callback \
  --scope read \
  --scope write

# After user approves, complete exchange
uxc auth oauth complete linear-graphql \
  --session-id <session_id> \
  --authorization-response 'http://127.0.0.1:8788/callback?code=...&state=...'

# Then bind endpoint
uxc auth binding add \
  --id linear-graphql \
  --host api.linear.app \
  --path-prefix /graphql \
  --scheme https \
  --credential linear-graphql \
  --priority 100
```

`uxc auth oauth login linear-graphql ... --flow authorization_code` is still available as a single-process interactive fallback.

## Link Setup
```bash
# Create link command
uxc link linear-graphql-cli https://api.linear.app/graphql

# Verify
linear-graphql-cli -h
```

## Query Examples

### List Issues
```bash
linear-graphql-cli query/issues '{"first":20}'
```

### List Issues With Explicit Fields
```bash
linear-graphql-cli query/issues '{"first":20,"_select":"nodes { identifier title url state { name } assignee { name } }"}'
```

### Filter Issues by Team
```bash
linear-graphql-cli query/issues filter='{"team":{"id":{"eq":"TEAM_ID"}}}'
```

### Get Single Issue
```bash
linear-graphql-cli query/issue id=ISSUE_123
```

### List Teams
```bash
linear-graphql-cli query/teams
```

### List Projects
```bash
linear-graphql-cli query/projects '{"first":10}'
```

## Mutation Examples

### Create Issue
```bash
linear-graphql-cli mutation/issueCreate '{
  "input": {
    "teamId": "TEAM_ID",
    "title": "New Feature Request",
    "description": "Description here",
    "priority": 2
  }
}'
```

### Update Issue
```bash
linear-graphql-cli mutation/issueUpdate '{
  "id": "ISSUE_ID",
  "input": {
    "title": "Updated Title",
    "description": "Updated description"
  }
}'
```

### Archive Issue
```bash
linear-graphql-cli mutation/issueArchive id=ISSUE_ID
```

### Add Comment
```bash
linear-graphql-cli mutation/commentCreate '{
  "input": {
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
