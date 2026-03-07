---
name: discord-openapi-skill
description: Operate Discord HTTP API through UXC with Discord OpenAPI schema. Bot token recommended for full API access including messages and server management. OAuth2 user authentication available for limited profile operations only.
---

# Discord API Skill

Use this skill to run Discord REST operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://discord.com/api/v10`.
- Access to Discord OpenAPI spec URL:
  - `https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json`
- Discord bot token (recommended) or OAuth2 user authentication (limited functionality).

## Authentication

### Option 1: Bot Token (Recommended)

Bot token provides full access to Discord API including reading messages, managing servers, sending messages, and all administrative operations. This is the recommended method for most use cases.

1. Create a bot application at https://discord.com/developers/applications
2. Generate a bot token from the Bot section
3. Configure bot credential:

```bash
uxc auth credential set discord-bot \
  --auth-type api_key \
  --header "Authorization=Bot {{secret}}" \
  --secret "YOUR_BOT_TOKEN_HERE"
```

4. Bind credential to Discord API endpoint:

```bash
uxc auth binding add \
  --id discord-bot \
  --host discord.com \
  --path-prefix /api/v10 \
  --scheme https \
  --credential discord-bot \
  --priority 100
```

### Option 2: OAuth2 User Authentication (Limited Use Cases)

**Important:** User OAuth2 has significant limitations and is **not recommended** for most operations:
- ❌ Cannot read channel messages via HTTP API (local RPC only)
- ❌ Cannot send messages or manage servers
- ✅ Can read user profile, email, connections
- ✅ Can list user's servers

Only use OAuth2 if you specifically need to access user profile information as the user. For all other operations, use Bot Token.

If you still need OAuth2 for user profile operations:

**Configuration:**
- Client ID: `1479302369723285736`
- Redirect URI: `http://127.0.0.1:11111/callback`

**OAuth2 Scopes:**

Discord user OAuth2 supports **read-only operations**. It cannot send messages or manage servers as a user (use Bot Token for those operations).

**Recommended Scopes (Full Functionality):**
```bash
--scope "identify email connections guilds guilds.members.read messages.read openid"
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
| `guilds.join` | Join user to servers (requires the same application's bot to already be in that guild) | ✅ **Write** |
| `guilds.members.read` | User's member info in servers | ❌ Read |
| `messages.read` | Read messages (local RPC only, **not** HTTP API) | ❌ Read |
| `openid` | OpenID Connect support | ❌ Read |

**Note:** User OAuth2 **cannot** send messages or manage servers as the user. Use Bot Token for write operations. `guilds.join` is a special user OAuth write capability that depends on the same application's bot already being in the target guild, so it is not part of the default read-only flow. See [Discord OAuth2 documentation](https://discord.com/developers/topics/oauth2) for complete scope list.

**Two-Stage OAuth Flow (Agent-Friendly):**

1. Start OAuth flow with desired scopes:
```bash
uxc auth oauth start discord-user \
  --endpoint https://discord.com/api/oauth2/token \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify email connections guilds guilds.members.read messages.read openid"
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
  --endpoint https://discord.com/api/oauth2/token \
  --flow authorization_code \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify email connections guilds guilds.members.read messages.read openid"
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

| Feature | Bot Token | User OAuth2 |
|---------|-----------|-------------|
| **Read channel messages** | ✅ Full access | ❌ Not via HTTP API |
| **Send messages** | ✅ As the bot | ❌ Not supported |
| **Manage channels/roles** | ✅ Bot permissions | ❌ Not supported |
| **Moderation actions** | ✅ Bot permissions | ❌ Not supported |
| **List servers** | ✅ Servers bot is in | ✅ User's servers |
| **Read user info** | ❌ Not available | ✅ As the user |
| **Message appearance** | Bot badge "BOT" | N/A |

**Key Recommendation:** Use **Bot Token** for almost all operations. User OAuth2 is only useful if you need to read user profile information as that specific user. For reading channel messages, managing servers, or sending messages, Bot Token is required.

## Guardrails

- **OAuth2 Scope Limitation:** User OAuth2 tokens cannot read channel messages via HTTP API, send messages, or manage servers. These operations require Bot Token authentication.
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
