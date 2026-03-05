---
name: linear-mcp-skill
description: Operate Linear workspace issues, projects, and teams through Linear GraphQL API using UXC. Use when tasks require querying or creating issues, managing projects, or interacting with Linear workflow. Supports both Personal API Key and OAuth authentication.
---

# Linear MCP Skill

Use this skill to run Linear GraphQL API operations through `uxc`.

Reuse the `uxc` skill guidance for discovery, schema inspection, auth lifecycle, and error recovery.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.linear.app/graphql`.
- Linear API key or OAuth credentials available.

## Authentication

Linear supports two authentication methods:

### Option 1: Personal API Key (Recommended for development)

1. Get your API key from Linear: https://linear.app/settings/api
2. Set credentials:
   ```bash
   uxc auth credential set linear-mcp --auth-type bearer --secret-env LINEAR_API_KEY
   ```

### Option 2: OAuth 2.0 (For production/user-delegated access)

1. Create an OAuth app in Linear: https://linear.app/settings/api
2. Start OAuth login:
   ```bash
   uxc auth oauth login linear-mcp \
     --endpoint https://api.linear.app/graphql \
     --flow authorization_code \
     --redirect-uri http://127.0.0.1:8788/callback \
     --scope read \
     --scope write
   ```
3. Bind endpoint:
   ```bash
   uxc auth binding add \
     --id linear-mcp \
     --host api.linear.app \
     --path-prefix /graphql \
     --scheme https \
     --credential linear-mcp \
     --priority 100
   ```

## Core Workflow

1. Use fixed link command by default:
   - `command -v linear-mcp-cli`
   - If the command exists, confirm it is the expected `uxc link` for this skill before using it.
   - If a command conflict is detected and cannot be safely reused, stop and ask skill maintainers for guidance instead of dynamically renaming the command.
   - If missing, create it: `uxc link linear-mcp-cli https://api.linear.app/graphql`
   - `linear-mcp-cli -h`

2. Discover operations:
   - `linear-mcp-cli -h`
   - Returns hundreds of GraphQL operations

3. Inspect specific operation:
   - `linear-mcp-cli query/issues -h`
   - `linear-mcp-cli mutation/issueCreate -h`

4. Execute queries:
   ```bash
   # Query issues
   linear-mcp-cli query/issues first=10

   # Query teams
   linear-mcp-cli query/teams first=10

   # Create issue (requires write scope)
   linear-mcp-cli mutation/issueCreate '{
     "issueCreateInput": {
       "teamId": "TEAM_ID",
       "title": "New Issue Title",
       "description": "Issue description"
     }
   }'
   ```

## Available Operations

### Queries
- `query/issues` - List and filter issues
- `query/issue` - Get single issue
- `query/teams` - List teams
- `query/team` - Get single team
- `query/projects` - List projects
- `query/workflowStates` - List workflow states

### Mutations
- `mutation/issueCreate` - Create new issue
- `mutation/issueUpdate` - Update issue
- `mutation/issueArchive` - Archive issue
- `mutation/commentCreate` - Add comment

## Usage Examples

### List recent issues
```bash
linear-mcp-cli query/issues first=20
```

### Get issue by ID
```bash
linear-mcp-cli query/issue id=ISSUE_ID
```

### List teams
```bash
linear-mcp-cli query/teams
```

### Create issue
```bash
linear-mcp-cli mutation/issueCreate '{"issueCreateInput":{"teamId":"YOUR_TEAM_ID","title":"Fix bug"}}'
```

## Guardrails

- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `data`, `error`.
- Use `linear-mcp-cli` as the default command path.
- `linear-mcp-cli <operation> ...` is equivalent to `uxc https://api.linear.app/graphql <operation> ...`.
- Prefer read operations first (query/*), then write operations (mutation/*).
- For write operations, always confirm user intent before execution.
- If auth fails, check credential with `uxc auth credential info linear-mcp`.

## References

- Linear API Documentation: https://developers.linear.app
- GraphQL Schema: https://studio.apollographql.com/public/Linear-API
- Invocation patterns: `references/usage-patterns.md`
