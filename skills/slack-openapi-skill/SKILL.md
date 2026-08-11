---
name: slack-openapi-skill
description: "Send messages, read conversation history, and manage reactions in Slack through UXC with a curated OpenAPI schema and bearer-token auth. Use when the task involves Slack messaging, channel reads, or reaction workflows."
user-invocable: true
triggers:
  - slack
  - slack api
  - send slack message
  - conversation history
  - slack reactions
---

# Slack Web API Skill

Use this skill to run Slack Web API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://slack.com/api`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/slack-openapi-skill/references/slack-web.openapi.json`
- A Slack bot token (`xoxb-...`); optional user token (`xoxp-...`) for user-identity reads.

## Scope

Messaging Core surface: auth validation, channel lookup, conversation history, thread replies, posting messages (including `thread_ts` replies), and adding reactions.

Does **not** cover Slack OAuth app installation, file uploads, or `users.*`/`admin.*`/`usergroups.*` families.

## Subscribe / Socket Mode

`uxc` has a built-in Slack Socket Mode transport. Invoke with:

```bash
uxc subscribe start https://slack.com/api --transport slack-socket-mode --auth slack-app --sink file:...
```

Socket Mode requires an app-level `xapp-...` token with `connections:write` scope. This skill covers Web API request/response calls; Socket Mode event intake is a validated transport but not yet fully workflow-packaged.

## Authentication

Slack Web API uses `Authorization: Bearer <token>`. Token types: `xoxb-...` (bot, recommended default), `xoxp-...` (user, explicit override), `xapp-...` (app-level, Socket Mode only).

### Bot Token (Recommended Default)

```bash
uxc auth credential set slack-bot \
  --auth-type bearer \
  --secret-env SLACK_BOT_TOKEN

uxc auth binding add \
  --id slack-bot \
  --host slack.com \
  --path-prefix /api \
  --scheme https \
  --credential slack-bot \
  --priority 100
```

### User Token (Explicit Override)

Use `xoxp-...` when the method requires user-token semantics (thread/history reads outside bot-accessible conversations):

```bash
uxc auth credential set slack-user \
  --auth-type bearer \
  --secret-env SLACK_USER_TOKEN
```

Do **not** bind `slack-user` by default. Invoke explicitly:

```bash
uxc auth binding match https://slack.com/api
slack-openapi-cli --auth slack-user get:/conversations.replies channel=C1234567890 ts=1717171717.000100
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v slack-openapi-cli`
   - If missing, create it:
     `uxc link slack-openapi-cli https://slack.com/api --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/slack-openapi-skill/references/slack-web.openapi.json`
   - `slack-openapi-cli -h`

2. Inspect operation schema first:
   - `slack-openapi-cli get:/auth.test -h`
   - `slack-openapi-cli get:/conversations.history -h`
   - `slack-openapi-cli post:/chat.postMessage -h`

3. Prefer read validation before writes:
   - `slack-openapi-cli get:/auth.test`
   - `slack-openapi-cli get:/conversations.list limit=20 types=public_channel,private_channel`
   - `slack-openapi-cli get:/conversations.info channel=C1234567890`

4. Execute with key/value or positional JSON:
   - key/value:
     `slack-openapi-cli get:/conversations.history channel=C1234567890 limit=20`
   - positional JSON:
     `slack-openapi-cli post:/chat.postMessage '{"channel":"C1234567890","text":"Hello from UXC"}'`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Bot token (`xoxb-...`) is the recommended default. Use `--auth slack-user` (`xoxp-...`) only when user-identity or user-token-only reads are needed.
- `get:/conversations.replies`: bot token works for IM/MPIM threads; use `--auth slack-user` for public/private channel threads.
- `get:/conversations.history` only returns conversations visible to the token; bot token is limited to joined conversations.
- Slack rate limits for `conversations.history` and `conversations.replies` vary by app distribution. Tighter limits apply to newly created commercially distributed non-Marketplace apps starting on May 29, 2025; do not assume generic Tier 3 behavior.
- Treat `post:/chat.postMessage` and `post:/reactions.add` as write/high-risk operations; require explicit user confirmation before execution.
- `slack-openapi-cli <operation> ...` is equivalent to `uxc https://slack.com/api --schema-url <slack_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/slack-web.openapi.json`
- Slack Web API docs: https://docs.slack.dev/apis/web-api
- `chat.postMessage`: https://docs.slack.dev/reference/methods/chat.postMessage
- `conversations.history`: https://docs.slack.dev/reference/methods/conversations.history
- `conversations.replies`: https://docs.slack.dev/reference/methods/conversations.replies/
