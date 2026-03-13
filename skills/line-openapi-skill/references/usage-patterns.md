# LINE Messaging API Skill - Usage Patterns

## Link Setup

```bash
command -v line-openapi-cli
uxc link line-openapi-cli https://api.line.me \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/line-openapi-skill/references/line-messaging.openapi.json
line-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set line-channel \
  --auth-type bearer \
  --secret-env LINE_CHANNEL_ACCESS_TOKEN

uxc auth binding add \
  --id line-channel \
  --host api.line.me \
  --scheme https \
  --credential line-channel \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://api.line.me
```

## Read Examples

```bash
# Confirm bot identity
line-openapi-cli get:/v2/bot/info

# Look up one user profile
line-openapi-cli get:/v2/bot/profile/{userId} userId=U1234567890abcdef

# Inspect monthly message quota
line-openapi-cli get:/v2/bot/message/quota

# Inspect current month quota consumption
line-openapi-cli get:/v2/bot/message/quota/consumption

# Read current webhook endpoint configuration
line-openapi-cli get:/v2/bot/channel/webhook/endpoint
```

## Write Examples (Confirm Intent First)

```bash
# Push a text message
line-openapi-cli post:/v2/bot/message/push '{"to":"U1234567890abcdef","messages":[{"type":"text","text":"Hello from UXC"}]}'

# Reply to a recent inbound event using its replyToken
line-openapi-cli post:/v2/bot/message/reply '{"replyToken":"REPLY_TOKEN","messages":[{"type":"text","text":"Reply from UXC"}]}'

# Update the webhook delivery URL
line-openapi-cli put:/v2/bot/channel/webhook/endpoint '{"endpoint":"https://example.com/line/webhook"}'

# Trigger LINE's webhook endpoint verification
line-openapi-cli post:/v2/bot/channel/webhook/test '{"endpoint":"https://example.com/line/webhook"}'
```

## Fallback Equivalence

- `line-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.line.me --schema-url <line_openapi_schema> <operation> ...`.
