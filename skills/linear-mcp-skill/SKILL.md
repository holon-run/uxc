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

2. Set credential with custom Authorization header:
   ```bash
   uxc auth credential set linear-mcp \
     --auth-type api_key \
     --header "Authorization:{{secret}}" \
     --secret "lin_api_XXX"
   ```

   Or use environment variable:
   ```bash
   export LINEAR_API_KEY="lin_api_XXX"
   uxc auth credential set linear-mcp \
     --auth-type api_key \
     --header "Authorization:{{secret}}" \
     --secret-env LINEAR_API_KEY
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

**Important:** Linear API requires `Authorization: lin_api_XXX` format (no "Bearer " prefix). The `--header "Authorization:{{secret}}"` configuration is required. See `uxc` skill's `references/auth-configuration.md` for detailed authentication patterns.

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
   # Query issues (simple)
   linear-mcp-cli query/issues '{"first":10}'

   # Query issues with explicit selection set for useful fields
   linear-mcp-cli query/issues '{"first":10,"_select":"nodes { identifier title url state { name } assignee { name } }"}'

   # Query teams
   linear-mcp-cli query/teams '{"first":10}'

   # Create issue (requires write scope)
   linear-mcp-cli mutation/issueCreate '{
     "input": {
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
linear-mcp-cli query/issues '{"first":20,"_select":"nodes { identifier title url state { name } assignee { name } }"}'
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
linear-mcp-cli mutation/issueCreate '{"input":{"teamId":"YOUR_TEAM_ID","title":"Fix bug"}}'
```

## Troubleshooting

### Authentication Errors

**Error: "Bearer token" prefix rejected**
- Linear API does not accept `Authorization: Bearer lin_api_XXX`
- Ensure credential uses `--auth-type api_key --header "Authorization:{{secret}}"`
- Do not use `--auth-type bearer`

**Error: "Credential not found"**
- Check credential exists: `uxc auth credential list`
- Verify binding: `uxc auth binding list`
- Create binding if missing (see Authentication section)

**Error: "No binding matched"**
- Check binding exists: `uxc auth binding match api.linear.app/graphql`
- If missing, create binding with `uxc auth binding add` (see Authentication section)

For detailed authentication troubleshooting, see `uxc` skill's `references/auth-configuration.md`.

### Common Issues

**Daemon issues after credential changes**
- Restart daemon: `uxc daemon restart`
- Check status: `uxc daemon status`

**Environment variable not found**
- Ensure variable is exported in daemon's environment
- Or use `--secret` for literal values (less secure)
- Or use `--secret-op` for 1Password (most secure)

## Guardrails

- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `data`, `error`.
- Prefer positional JSON for non-string and typed arguments (for example: `linear-mcp-cli query/issues '{"first":10}'` and `linear-mcp-cli mutation/issueCreate '{"input":{"teamId":"TEAM_ID","title":"Test"}}'`).
- Use reserved GraphQL argument `_select` (string) when you need explicit return fields, e.g. `{"_select":"nodes { identifier title }"}`.
- Use `linear-mcp-cli` as the default command path.
- `linear-mcp-cli <operation> ...` is equivalent to `uxc https://api.linear.app/graphql <operation> ...`.
- Prefer read operations first (query/*), then write operations (mutation/*).
- For write operations, always confirm user intent before execution.
- If auth fails, check credential with `uxc auth credential info linear-mcp`.

## References

- Linear API Documentation: https://developers.linear.app
- GraphQL Schema: https://studio.apollographql.com/public/Linear-API
- Invocation patterns: `references/usage-patterns.md`
