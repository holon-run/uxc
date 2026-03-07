# Discord API Skill - Usage Patterns

## Link Setup

```bash
command -v discord-openapi-cli
uxc link discord-openapi-cli https://discord.com/api/v10 \
  --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json
discord-openapi-cli -h
```

## Auth Setup (Bot Token)

```bash
uxc auth credential set discord-openapi \
  --auth-type api_key \
  --header "Authorization:Bot {{secret}}" \
  --secret-env DISCORD_BOT_TOKEN

uxc auth binding add \
  --id discord-openapi \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-openapi \
  --priority 100
```

## Read Examples

```bash
# Connectivity check (public endpoint)
discord-openapi-cli get:/gateway

# Get current bot/application user
discord-openapi-cli get:/users/@me

# List channels in a guild
discord-openapi-cli get:/guilds/{guild_id}/channels guild_id=YOUR_GUILD_ID
```

## Write Example (Confirm Intent First)

```bash
# Create a channel message
discord-openapi-cli post:/channels/{channel_id}/messages '{"channel_id":"YOUR_CHANNEL_ID","content":"Hello from UXC"}'
```

## Fallback Equivalence

- `discord-openapi-cli <operation> ...` is equivalent to
  `uxc https://discord.com/api/v10 --schema-url <discord_openapi_spec> <operation> ...`.
