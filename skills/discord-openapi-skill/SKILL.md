---
name: discord-openapi-skill
description: Operate Discord HTTP API through UXC with Discord OpenAPI schema mapping (`--schema-url`) and bot-token authentication. Use when tasks need read/write Discord REST operations such as guild/channel lookup and message creation.
---

# Discord API Skill

Use this skill to run Discord REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://discord.com/api/v10`.
- Access to Discord OpenAPI spec URL:
  - `https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json`
- Discord bot token (for most read/write operations).

## Authentication

1. Configure bot credential:

```bash
uxc auth credential set discord-openapi \
  --auth-type api_key \
  --header "Authorization:Bot {{secret}}" \
  --secret-env DISCORD_BOT_TOKEN
```

2. Bind credential to Discord API endpoint:

```bash
uxc auth binding add \
  --id discord-openapi \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-openapi \
  --priority 100
```

3. Confirm binding:

```bash
uxc auth binding match https://discord.com/api/v10
```

## Core Workflow

1. Use fixed link command by default:
   - `command -v discord-openapi-cli`
   - If missing, create it: `uxc link discord-openapi-cli https://discord.com/api/v10`
   - `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json -h`

2. Discover operations with schema mapping:
   - `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json -h`

3. Inspect operation schema first:
   - `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json get:/users/@me -h`
   - `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json post:/channels/{channel_id}/messages -h`

4. Execute operation:
   - connectivity check (no auth): `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json get:/gateway`
   - key/value: `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json get:/guilds/{guild_id}/channels guild_id=GUILD_ID`
   - positional JSON: `discord-openapi-cli --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json post:/channels/{channel_id}/messages '{"channel_id":"CHANNEL_ID","content":"Hello from uxc"}'`

## Guardrails

- Discord OpenAPI spec is currently consumed via `--schema-url`; do not omit it in this skill workflow.
- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Prefer positional JSON for non-string objects instead of `--input-json`.
- `discord-openapi-cli <operation> ...` is equivalent to `uxc https://discord.com/api/v10 <operation> ...` with same `--schema-url`.
- Treat `post:/channels/{channel_id}/messages`, delete/update endpoints, and moderation endpoints as write/high-risk operations; require explicit user confirmation before execution.

## References

- Usage patterns: `references/usage-patterns.md`
- Discord API docs: https://discord.com/developers/docs
- Discord API OpenAPI spec: https://github.com/discord/discord-api-spec
