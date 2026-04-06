---
name: x-openapi-skill
description: Operate X API v2 through UXC with the official OpenAPI schema, OAuth2 PKCE user-context auth, app-only bearer guidance, and read-first guardrails for timeline/bookmark/post workflows.
---

# X API Skill

Use this skill to run X API v2 operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.x.com`.
- Access to the official OpenAPI schema URL:
  - `https://api.x.com/2/openapi.json`
- An X developer app with OAuth2 enabled.

## Authentication

### Preferred Path: OAuth2 Authorization Code + PKCE (User Context)

This path is required for user-context operations such as bookmarks, home timeline, posting, replying, and DM actions.

1. Start OAuth2 login:

```bash
uxc auth oauth start x-api-user \
  --endpoint https://api.x.com/2 \
  --client-id M3VseG9xOXZXZVREZnNqWk9jN1I6MTpjaQ \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope users.read \
  --scope tweet.read \
  --scope tweet.write \
  --scope tweet.moderate.write \
  --scope media.write \
  --scope like.read \
  --scope like.write \
  --scope follows.read \
  --scope follows.write \
  --scope list.read \
  --scope list.write \
  --scope timeline.read \
  --scope space.read \
  --scope mute.read \
  --scope mute.write \
  --scope block.read \
  --scope block.write \
  --scope bookmark.read \
  --scope bookmark.write \
  --scope dm.read \
  --scope dm.write \
  --authorization-endpoint https://x.com/i/oauth2/authorize \
  --token-endpoint https://api.x.com/2/oauth2/token
```

This default example intentionally includes the full OAuth2 user scope set exposed by the current X OpenAPI surface, excluding `offline.access` to avoid refresh-token compatibility failures observed on some X app configurations. Add `--scope offline.access` only when you explicitly need long-lived refresh behavior and have verified token refresh works for your app.

2. Open the returned authorization URL in browser and finish consent.
3. Complete login with the callback URL:

```bash
uxc auth oauth complete x-api-user \
  --session-id <session_id> \
  --authorization-response '<callback_url>'
```

4. Bind credential to X API:

```bash
uxc auth binding add \
  --id x-api-user \
  --host api.x.com \
  --path-prefix /2 \
  --scheme https \
  --credential x-api-user \
  --priority 100
```

5. Verify binding:

```bash
uxc auth binding match https://api.x.com/2
```

### Optional Path: App-only Bearer Token (Public Reads)

App-only bearer auth is useful for selected public read endpoints.

Do not embed bearer tokens in this skill or repository. Configure local credentials only.

```bash
uxc auth credential set x-api-app \
  --auth-type bearer \
  --secret-env X_API_BEARER_TOKEN

uxc auth binding add \
  --id x-api-app \
  --host api.x.com \
  --path-prefix /2 \
  --scheme https \
  --credential x-api-app \
  --priority 90
```

Use `--auth x-api-app` when you intentionally want app-only requests.

## Core Workflow

1. Use fixed link command by default:
   - `command -v x-openapi-cli`
   - If missing, create it:
     `uxc link x-openapi-cli https://api.x.com --schema-url https://api.x.com/2/openapi.json`
   - `x-openapi-cli -h`

2. Inspect operation schema first:
   - `x-openapi-cli get:/2/users/me -h`
   - `x-openapi-cli get:/2/users/{id}/bookmarks -h`
   - `x-openapi-cli post:/2/tweets -h`

3. Execute reads before writes:
   - `x-openapi-cli get:/2/users/me`
   - `x-openapi-cli get:/2/users/{id}/bookmarks id=<user_id> max_results=20`
   - `x-openapi-cli get:/2/users/{id}/timelines/reverse_chronological id=<user_id> max_results=20`

4. Execute writes with explicit user confirmation:
   - key/value:
     `x-openapi-cli post:/2/tweets text='hello from uxc'`
   - positional JSON:
     `x-openapi-cli post:/2/tweets '{"text":"hello from uxc"}'`

## Subscribe / Stream Status

X stream endpoints are HTTP streaming routes and can be used with `uxc subscribe` raw HTTP stream mode.

Example:

```bash
uxc subscribe start https://api.x.com/2/tweets/search/stream \
  --auth x-api-app \
  --sink file:$HOME/.uxc/subscriptions/x-search-stream.ndjson
```

Use the relevant stream rule-management endpoints before starting stream consumers.

## Guardrails

- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `x-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.x.com --schema-url https://api.x.com/2/openapi.json <operation> ...`.
- OAuth2 PKCE is the default path for user-context operations.
- Do not commit or hardcode bearer tokens, client secrets, or refresh tokens.
- Treat `post:/2/tweets`, DM endpoints, bookmark write endpoints, and stream rule mutation endpoints as write/high-risk operations; require explicit user confirmation before execution.

## References

- Usage patterns: `references/usage-patterns.md`
- X API OpenAPI schema: `https://api.x.com/2/openapi.json`
- X OAuth2 authorization code with PKCE: `https://docs.x.com/fundamentals/authentication/oauth-2-0/authorization-code`
- X OAuth2 app-only bearer auth: `https://docs.x.com/fundamentals/authentication/oauth-2-0/application-only`
