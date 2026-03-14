---
name: telegram-openapi-skill
description: Operate Telegram Bot API through UXC with a curated OpenAPI schema, bot-token path auth, polling-based reads, and webhook management guardrails.
---

# Telegram Bot API Skill

Use this skill to run Telegram Bot API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://api.telegram.org`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/telegram-openapi-skill/references/telegram-bot.openapi.json`
- A Telegram bot token from BotFather.

## Scope

This skill covers a lean bot core surface:

- bot identity and chat lookup
- text sends
- media sends by `file_id` or HTTP URL
- polling via `getUpdates`
- webhook setup/status/delete operations

This skill does **not** cover:

- multipart new-file uploads
- self-signed webhook certificate upload
- generic webhook ingestion/runtime hosting
- the full Telegram Bot API surface

## Authentication

Telegram Bot API requires the bot token in the request path: `https://api.telegram.org/bot<TOKEN>/METHOD_NAME`.

Configure the credential with a request path prefix template:

```bash
uxc auth credential set telegram-bot \
  --auth-type api_key \
  --secret-env TELEGRAM_BOT_TOKEN \
  --path-prefix-template "/bot{{secret}}"

uxc auth binding add \
  --id telegram-bot \
  --host api.telegram.org \
  --scheme https \
  --credential telegram-bot \
  --priority 100
```

Validate the local mapping when auth looks wrong:

```bash
uxc auth binding match https://api.telegram.org/getMe
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v telegram-openapi-cli`
   - If missing, create it:
     `uxc link telegram-openapi-cli https://api.telegram.org --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/telegram-openapi-skill/references/telegram-bot.openapi.json`
   - `telegram-openapi-cli -h`

2. Inspect operation schema first:
   - `telegram-openapi-cli get:/getMe -h`
   - `telegram-openapi-cli post:/sendMessage -h`
   - `telegram-openapi-cli post:/getUpdates -h`

3. Prefer read/setup validation before writes:
   - `telegram-openapi-cli get:/getMe`
   - `telegram-openapi-cli get:/getWebhookInfo`
   - `telegram-openapi-cli get:/getChat chat_id=@channel_or_chat_id`

4. Execute operations with key/value or positional JSON:
   - key/value:
     `telegram-openapi-cli post:/sendMessage chat_id=CHAT_ID text="Hello from uxc"`
   - positional JSON:
     `telegram-openapi-cli post:/sendMessage '{"chat_id":"CHAT_ID","text":"Hello from uxc"}'`
   - daemon-backed polling subscribe:
     `uxc subscribe start https://api.telegram.org post:/getUpdates '{"timeout":5,"allowed_updates":["message","callback_query"]}' --mode poll --poll-config '{"interval_secs":2,"extract_items_pointer":"/result","request_cursor_arg":"offset","cursor_from_item_pointer":"/update_id","cursor_transform":"increment","checkpoint_strategy":{"type":"item_key","item_key_pointer":"/update_id"}}' --sink file:/tmp/telegram-updates.ndjson`

## Runtime Validation

The following Telegram polling flow has been validated against the real Bot API through `uxc`:

- `get:/getMe`
- `get:/getWebhookInfo`
- daemon-backed `uxc subscribe --mode poll` on `post:/getUpdates`
- item-derived offset progression from `update_id + 1`
- dedupe/checkpoint behavior for repeated polls

Observed runtime behavior:

- `data` events are emitted for real Telegram updates
- `poll` events record fetched/emitted/skipped counts
- `checkpoint` events are emitted after new updates are seen
- repeated polls skip already-consumed updates after checkpoint advancement

## Operation Groups

### Read / Lookup

- `get:/getMe`
- `get:/getChat`
- `get:/getChatMember`
- `get:/getWebhookInfo`

### Messaging

- `post:/sendMessage`
- `post:/sendPhoto`
- `post:/sendDocument`
- `post:/sendMediaGroup`

### Update Delivery

- `post:/getUpdates`
- `post:/setWebhook`
- `post:/deleteWebhook`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `getUpdates` and webhook delivery are mutually exclusive:
  - if a webhook is configured, call `post:/deleteWebhook` before polling with `post:/getUpdates`
  - if polling is active, do not treat webhook operations as background subscription support
- Telegram allows only one active `getUpdates` consumer per bot token:
  - if another bot process or script is polling at the same time, Telegram returns HTTP 409
  - stop the other consumer before relying on daemon-backed polling subscribe
- For daemon-backed polling subscribe, prefer item-derived offset progression:
  - `extract_items_pointer` should be `/result`
  - `request_cursor_arg` should be `offset`
  - `cursor_from_item_pointer` should be `/update_id`
  - `cursor_transform` should be `increment`
  - `checkpoint_strategy.type` should usually be `item_key` with `item_key_pointer=/update_id`
- `uxc auth binding match` should be checked against a concrete Telegram method URL such as `https://api.telegram.org/getMe`, because auth is applied through a path-prefix template that expands to `/bot<TOKEN>/...`.
- `sendPhoto`, `sendDocument`, and `sendMediaGroup` in this skill accept existing `file_id` values or HTTP URLs only; they do not upload new local files.
- `setWebhook` in this skill does not support certificate upload for self-signed certs.
- Treat `post:/sendMessage`, all `send*` operations, and webhook-changing operations as write/high-risk actions; require explicit user confirmation before execution.
- `telegram-openapi-cli <operation> ...` is equivalent to `uxc https://api.telegram.org --schema-url <telegram_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/telegram-bot.openapi.json`
- Telegram Bot API docs: https://core.telegram.org/bots/api
- Local Bot API server: https://github.com/tdlib/telegram-bot-api
