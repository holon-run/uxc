# Slack Web API Skill - Usage Patterns

## Link Setup

```bash
command -v slack-openapi-cli
uxc link slack-openapi-cli https://slack.com/api \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/slack-openapi-skill/references/slack-web.openapi.json
slack-openapi-cli -h
```

## Auth Setup (Bot Token Default)

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

## Auth Setup (User Token Override)

```bash
uxc auth credential set slack-user \
  --auth-type bearer \
  --secret-env SLACK_USER_TOKEN
```

Use `--auth slack-user` for reads that need user-token semantics:

```bash
slack-openapi-cli --auth slack-user get:/conversations.replies channel=C1234567890 ts=1717171717.000100
slack-openapi-cli --auth slack-user get:/conversations.history channel=C1234567890 limit=50
```

## Read Examples

```bash
# Confirm token identity and workspace
slack-openapi-cli get:/auth.test

# List candidate channels
slack-openapi-cli get:/conversations.list limit=20 types=public_channel,private_channel

# Inspect one conversation
slack-openapi-cli get:/conversations.info channel=C1234567890

# Read recent history
slack-openapi-cli get:/conversations.history channel=C1234567890 limit=20

# Read thread replies
slack-openapi-cli --auth slack-user get:/conversations.replies channel=C1234567890 ts=1717171717.000100
```

## Write Examples (Confirm Intent First)

```bash
# Post a channel message
slack-openapi-cli post:/chat.postMessage '{"channel":"C1234567890","text":"Hello from UXC"}'

# Post a thread reply
slack-openapi-cli post:/chat.postMessage '{"channel":"C1234567890","text":"Reply from UXC","thread_ts":"1717171717.000100"}'

# Add a reaction to a message
slack-openapi-cli post:/reactions.add '{"channel":"C1234567890","timestamp":"1717171717.000100","name":"thumbsup"}'
```

## Fallback Equivalence

- `slack-openapi-cli <operation> ...` is equivalent to
  `uxc https://slack.com/api --schema-url <slack_openapi_schema> <operation> ...`.
