---
name: discord-openapi-skill
description: "Send messages, manage channels, and read server data through the Discord HTTP API via UXC with the official OpenAPI schema. Use when the task involves Discord bot operations, server management, or user profile reads."
user-invocable: true
triggers:
  - discord
  - discord api
  - discord bot
  - send discord message
  - discord server
---

# Discord API Skill

Use this skill to run Discord REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://discord.com/api/v10`.
- Access to Discord OpenAPI spec URL:
  - `https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json`
- Discord bot token (recommended) or OAuth2 user authentication (profile reads only).

## Authentication

### Bot Token (Recommended)

Full API access: messages, server management, moderation.

```bash
uxc auth credential set discord-bot \
  --auth-type api_key \
  --header "Authorization=Bot {{secret}}" \
  --secret "YOUR_BOT_TOKEN_HERE"

uxc auth binding add \
  --id discord-bot \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-bot \
  --priority 100
```

### OAuth2 User Authentication (Profile Reads Only)

User OAuth2 **cannot** read channel messages via HTTP API, send messages, or manage servers. Use only for user profile, email, connections, and server list reads.

Two-stage flow:

```bash
uxc auth oauth start discord-user \
  --endpoint https://discord.com/api/oauth2/token \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify email connections guilds guilds.members.read messages.read openid"
```

Complete after browser authorization:

```bash
uxc auth oauth complete discord-user \
  --session-id <session_id_from_step_1> \
  --authorization-response "<callback_url_from_browser>"

uxc auth binding add \
  --id discord-user \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-user \
  --priority 100
```

## Core Workflow

1. Use fixed link command by default:
   - `command -v discord-openapi-cli`
   - If missing, create it: `uxc link discord-openapi-cli https://discord.com/api/v10 --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json`
   - `discord-openapi-cli -h`

2. Discover operations with schema mapping:
   - `discord-openapi-cli -h`

3. Inspect operation schema first:
   - `discord-openapi-cli get:/users/@me -h`
   - `discord-openapi-cli post:/channels/{channel_id}/messages -h`

4. Execute operation:
   - connectivity check (no auth): `discord-openapi-cli get:/gateway`
   - key/value: `discord-openapi-cli get:/guilds/{guild_id}/channels guild_id=GUILD_ID`
   - positional JSON: `discord-openapi-cli post:/channels/{channel_id}/messages '{"channel_id":"CHANNEL_ID","content":"Hello from uxc"}'`
   - binding check when auth looks wrong: `uxc auth binding match https://discord.com/api/v10`

## Subscribe / Gateway

Discord inbound events flow through the Gateway WebSocket, not this REST surface. The built-in `discord-gateway` transport handles `IDENTIFY`, heartbeat, sequence tracking, and reconnect:

```bash
uxc subscribe start https://discord.com/api/v10 \
  '{"intents":4609,"device":"uxc-discord"}' \
  --transport discord-gateway \
  --auth discord-bot \
  --sink file:$HOME/.uxc/subscriptions/discord-gateway.ndjson
```

Intent `4609` = `GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES`. Add `32768` (`MESSAGE_CONTENT`) only when the bot has that privileged intent enabled in the developer portal.

## Guardrails

- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- User OAuth2 tokens cannot read channel messages, send messages, or manage servers via HTTP API — use Bot Token for those.
- Prefer positional JSON for non-string objects instead of `--input-json`.
- `discord-openapi-cli <operation> ...` is equivalent to `uxc https://discord.com/api/v10 --schema-url <discord_openapi_spec> <operation> ...`.
- Treat `post:/channels/{channel_id}/messages`, delete/update endpoints, and moderation endpoints as write/high-risk operations; require explicit user confirmation before execution.

## References

- Usage patterns: `references/usage-patterns.md`
- Discord API docs: https://discord.com/developers/docs
- Discord API OpenAPI spec: https://github.com/discord/discord-api-spec
