---
name: discord-openapi-skill
description: Operate Discord HTTP API through UXC with Discord OpenAPI schema. Supports both bot token and OAuth2 user authentication. Use for guild/channel lookup, user info, messages, and Discord REST operations.
---

# Discord API Skill

Use this skill to run Discord REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://discord.com/api/v10`.
- Access to Discord OpenAPI spec URL:
  - `https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json`
- Discord credentials (bot token or OAuth2 user authentication).

## Authentication

### Option 1: Bot Token (Recommended for bot operations)

1. Configure bot credential:

```bash
uxc auth credential set discord-bot \
  --auth-type api_key \
  --header "Authorization:Bot {{secret}}" \
  --secret-env DISCORD_BOT_TOKEN
```

2. Bind credential to Discord API endpoint:

```bash
uxc auth binding add \
  --id discord-bot \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-bot \
  --priority 100
```

### Option 2: OAuth2 User Authentication (For user-specific operations)

**Configuration:**
- Client ID: `1479302369723285736`
- Redirect URI: `http://127.0.0.1:11111/callback`

**OAuth2 Scopes:**

Discord user OAuth2 supports **read-only operations**. It cannot send messages or manage servers as a user (use Bot Token for those operations).

**Recommended Scopes (Full Functionality):**
```bash
--scope "identify email connections guilds guilds.join guilds.members.read messages.read openid"
```

**Minimal Read-Only Scopes:**
```bash
--scope "identify email connections guilds guilds.members.read"
```

**Scope Reference:**

| Scope | Description | Write Operation |
|-------|-------------|-----------------|
| `identify` | Basic user info (username, avatar, etc.) | ❌ Read |
| `email` | User's email address | ❌ Read |
| `connections` | Linked third-party accounts (Twitch, YouTube, etc.) | ❌ Read |
| `guilds` | User's server list | ❌ Read |
| `guilds.join` | Join user to servers | ✅ **Write** |
| `guilds.members.read` | User's member info in servers | ❌ Read |
| `messages.read` | Read messages (local RPC only) | ❌ Read |
| `openid` | OpenID Connect support | ❌ Read |

**Note:** User OAuth2 **cannot** send messages or manage servers as the user. Use Bot Token for write operations. See [Discord OAuth2 documentation](https://docs.discord.com/developers/topics/oauth2) for complete scope list.

**Two-Stage OAuth Flow (Agent-Friendly):**

1. Start OAuth flow with desired scopes:
```bash
uxc auth oauth start discord-user \
  --endpoint https://discord.com/api/v10/oauth2/token \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify email connections guilds guilds.join guilds.members.read messages.read openid"
```

2. Open the displayed authorization URL in browser, complete authorization, then copy the callback URL from browser address bar.

3. Complete OAuth flow:
```bash
uxc auth oauth complete discord-user \
  --session-id <session_id_from_step_1> \
  --authorization-response "<callback_url_from_browser>"
```

4. Bind credential:
```bash
uxc auth binding add \
  --id discord-user \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-user \
  --priority 100
```

**Interactive Alternative (Local Terminal Only):**

```bash
uxc auth oauth login discord-user \
  --endpoint https://discord.com/api/v10/oauth2/token \
  --flow authorization_code \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify email connections guilds guilds.join guilds.members.read messages.read openid"
```

Then paste the callback URL when prompted.

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

## Authentication Methods Comparison

| Feature | User OAuth2 | Bot Token |
|---------|-------------|-----------|
| **Read user info** | ✅ As the user | ❌ Not available |
| **List user's servers** | ✅ User's servers | ✅ Servers bot is in |
| **Send messages** | ❌ Not supported | ✅ As the bot |
| **Manage channels/roles** | ❌ Not supported | ✅ Bot permissions |
| **Moderation actions** | ❌ Not supported | ✅ Bot permissions |
| **Message appearance** | N/A | Bot badge "BOT" |

**Key Limitation:** User OAuth2 **cannot** send messages or manage servers as the user. Discord intentionally restricts user OAuth2 to read-only operations for security. To perform write operations, you must use a Bot Token (which will display messages as coming from a bot).

**Recommendation:**
- Use **User OAuth2** for reading user data and identity verification
- Use **Bot Token** for automated tasks, message sending, and server management

## Guardrails

- **OAuth2 Scope Limitation:** User OAuth2 tokens cannot send messages or manage servers. These operations require Bot Token authentication.
- Discord OpenAPI spec is persisted in the generated link via `uxc link --schema-url ...`; pass `--schema-url <other-url>` only when you need to override it temporarily.
- Keep automation on JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- Prefer positional JSON for non-string objects instead of `--input-json`.
- `discord-openapi-cli <operation> ...` is equivalent to `uxc https://discord.com/api/v10 --schema-url <discord_openapi_spec> <operation> ...`.
- Treat `post:/channels/{channel_id}/messages`, delete/update endpoints, and moderation endpoints as write/high-risk operations; require explicit user confirmation before execution.

## References

- Usage patterns: `references/usage-patterns.md`
- Discord API docs: https://discord.com/developers/docs
- Discord API OpenAPI spec: https://github.com/discord/discord-api-spec
