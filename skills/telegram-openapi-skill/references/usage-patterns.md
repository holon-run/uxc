# Telegram Bot API Skill - Usage Patterns

## Link Setup

```bash
command -v telegram-openapi-cli
uxc link telegram-openapi-cli https://api.telegram.org \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/telegram-openapi-skill/references/telegram-bot.openapi.json
telegram-openapi-cli -h
```

## Auth Setup

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

uxc auth binding match https://api.telegram.org/getMe
```

## Read Examples

```bash
# Confirm the bot identity
telegram-openapi-cli get:/getMe

# Inspect a chat the bot can access
telegram-openapi-cli get:/getChat chat_id=@my_channel

# Check webhook state before deciding between webhook and polling
telegram-openapi-cli get:/getWebhookInfo
```

## Polling Example

```bash
# Remove webhook first if one is configured
telegram-openapi-cli post:/deleteWebhook '{"drop_pending_updates":false}'

# Poll for updates with long-poll timeout
telegram-openapi-cli post:/getUpdates '{"timeout":30,"allowed_updates":["message","callback_query"]}'

# Run background polling through uxc subscribe with offset derived from update_id + 1
# Only one getUpdates consumer can be active for the bot token at a time.
uxc subscribe start https://api.telegram.org post:/getUpdates \
  '{"timeout":5,"allowed_updates":["message","callback_query"]}' \
  --mode poll \
  --poll-config '{"interval_secs":2,"extract_items_pointer":"/result","request_cursor_arg":"offset","cursor_from_item_pointer":"/update_id","cursor_transform":"increment","checkpoint_strategy":{"type":"item_key","item_key_pointer":"/update_id"}}' \
  --sink file:/tmp/telegram-updates.ndjson
```

## Write Examples (Confirm Intent First)

```bash
# Send a text message
telegram-openapi-cli post:/sendMessage '{"chat_id":"CHAT_ID","text":"Hello from UXC"}'

# Send a photo by existing file_id or HTTP URL only
telegram-openapi-cli post:/sendPhoto '{"chat_id":"CHAT_ID","photo":"https://example.com/photo.jpg","caption":"From UXC"}'
```

## Fallback Equivalence

- `telegram-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.telegram.org --schema-url <telegram_openapi_schema> <operation> ...`.
