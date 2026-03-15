---
name: slack-openapi-skill
description: Operate Slack Web API through UXC with a curated OpenAPI schema, bearer-token auth, and messaging-core guardrails.
---

# Slack Web API Skill

Use this skill to run Slack Web API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://slack.com/api`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/slack-openapi-skill/references/slack-web.openapi.json`
- A Slack bot token and, for selected thread/history reads, an optional user token.

## Scope

This skill covers a Messaging Core surface:

- auth validation
- channel lookup and inspection
- conversation history reads
- thread replies reads
- posting messages, including replies via `thread_ts`
- adding reactions

This skill does **not** cover:

- Slack OAuth app installation flow
- file upload flows
- `users.*`, `admin.*`, or `usergroups.*` method families

## Subscribe / Socket Mode Status

Slack inbound events can be delivered through Socket Mode. `uxc` now has a built-in Slack Socket Mode transport, but this skill still treats it as a limited event-ingest path rather than a fully packaged workflow surface.

Current `uxc subscribe` status:

- Slack Web API request/response calls are supported by this skill
- a live Socket Mode smoke test succeeded with the built-in transport:
  - `uxc subscribe start https://slack.com/api --transport slack-socket-mode --auth slack-app --sink file:...`
  - the runtime opened a fresh temporary WebSocket URL automatically
  - the initial Slack `hello` frame was received
- a real inbound message event was validated end-to-end:
  - while the Socket Mode job was running, a live Slack message event was delivered as an `events_api` envelope
  - the sink recorded the message payload and `ack_sent=true`

What the current built-in transport already handles:

- app-level `xapp-...` auth via `--auth`
- automatic `apps.connections.open` before each connect attempt
- raw Socket Mode frame capture
- automatic ack for envelopes that carry `envelope_id`

What is still not packaged:

- event-shape guidance per subscribed Slack event family
- higher-level workflow packaging for common Slack event intake flows

Slack Socket Mode is now a validated IM subscribe provider at the transport/runtime level.

## Authentication

Slack Web API uses `Authorization: Bearer <token>`.

Token types used in practice:

- `xoxb-...`: Bot User OAuth Token. This is the recommended default for this skill.
- `xoxp-...`: User OAuth Token. Use this only when you explicitly want user-token semantics.
- `xapp-...`: App-level token. Use this for Socket Mode subscribe, not for normal Web API methods.

To create an app-level `xapp-...` token for Socket Mode:

1. Open the target Slack app at `https://api.slack.com/apps`
2. Go to `Basic Information`
3. Find `App-Level Tokens`
4. Generate a token with the `connections:write` scope
5. Enable `Socket Mode` in the app configuration before relying on subscribe-based event intake

### Option 1: Bot Token (Recommended Default)

Use the Slack `Bot User OAuth Token` (`xoxb-...`) for the default binding and for most messaging operations:

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

### Option 2: User Token (Explicit Override For Selected Reads)

Use a separate Slack `User OAuth Token` (`xoxp-...`) when the method requires user-token semantics, especially thread/history access outside bot-accessible conversations:

```bash
uxc auth credential set slack-user \
  --auth-type bearer \
  --secret-env SLACK_USER_TOKEN
```

Do **not** bind `slack-user` by default to the same host/path. Invoke it explicitly when needed:

```bash
uxc auth binding match https://slack.com/api
slack-openapi-cli --auth slack-user get:/conversations.replies channel=C1234567890 ts=1717171717.000100
```

If you intentionally want writes to appear as the installing user rather than the bot, you can also invoke write methods with `--auth slack-user`, but treat that as an explicit override rather than the default path.

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

## Operation Groups

### Read / Lookup

- `get:/auth.test`
- `get:/conversations.list`
- `get:/conversations.info`
- `get:/conversations.history`
- `get:/conversations.replies`

### Messaging / Reactions

- `post:/chat.postMessage`
- `post:/reactions.add`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Bot token is the recommended default for send and basic read flows.
- Bot token means Slack `Bot User OAuth Token` (`xoxb-...`); do not confuse it with `xapp-...` app-level tokens.
- User token means Slack `User OAuth Token` (`xoxp-...`); use `--auth slack-user` when you intentionally need user identity or user-token-only reads.
- `get:/conversations.replies` has token-type restrictions:
  - bot token works for IM and MPIM threads the bot can access
  - public/private channel thread reads should use `--auth slack-user`
- `get:/conversations.history` only returns conversations visible to the supplied token; a bot token is limited to joined conversations.
- Slack rate limits for `conversations.history` and `conversations.replies` vary by app distribution. Slack documents a tighter limit for newly created commercially distributed non-Marketplace apps starting on May 29, 2025; do not assume generic Tier 3 behavior.
- Treat `post:/chat.postMessage` and `post:/reactions.add` as write/high-risk operations; require explicit user confirmation before execution.
- `slack-openapi-cli <operation> ...` is equivalent to `uxc https://slack.com/api --schema-url <slack_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/slack-web.openapi.json`
- Slack Web API docs: https://docs.slack.dev/apis/web-api
- `chat.postMessage`: https://docs.slack.dev/reference/methods/chat.postMessage
- `conversations.history`: https://docs.slack.dev/reference/methods/conversations.history
- `conversations.replies`: https://docs.slack.dev/reference/methods/conversations.replies/
