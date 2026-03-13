# DingTalk Messaging Skill - Usage Patterns

## Link Setup

```bash
command -v dingtalk-openapi-cli
uxc link dingtalk-openapi-cli https://api.dingtalk.com/v1.0 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/dingtalk-openapi-skill/references/dingtalk-messaging.openapi.json
dingtalk-openapi-cli -h
```

## Token Bootstrap

```bash
curl -sS https://api.dingtalk.com/v1.0/oauth2/accessToken \
  -H 'Content-Type: application/json' \
  -d '{"appKey":"dingxxxx","appSecret":"xxxx"}'
```

Store the resulting `accessToken` in an environment variable before binding it into `uxc auth`.

## Auth Setup

```bash
uxc auth credential set dingtalk-app \
  --auth-type bearer \
  --secret-env DINGTALK_ACCESS_TOKEN

uxc auth binding add \
  --id dingtalk-app \
  --host api.dingtalk.com \
  --path-prefix /v1.0 \
  --scheme https \
  --credential dingtalk-app \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://api.dingtalk.com/v1.0
```

## Read Examples

```bash
# Read one user by unionId
dingtalk-openapi-cli get:/contact/users/{unionId} unionId=$DINGTALK_UNION_ID
```

## Write Examples (Confirm Intent First)

```bash
# Send a one-to-one bot message to multiple users
dingtalk-openapi-cli post:/robot/oToMessages/batchSend '{"robotCode":"dingxxxx","userIds":["user001","user002"],"msgKey":"sampleText","msgParam":"{\"content\":\"Hello from UXC\"}"}'

# Send a group message through a bot
dingtalk-openapi-cli post:/robot/groupMessages/send '{"openConversationId":"cidxxxx","robotCode":"dingxxxx","msgKey":"sampleText","msgParam":"{\"content\":\"Hello from UXC\"}"}'

# Send a service group message
dingtalk-openapi-cli post:/serviceGroup/messages/send '{"coolAppCode":"coolappxxxx","openConversationId":"cidxxxx","robotCode":"dingxxxx","msgKey":"sampleText","msgParam":"{\"content\":\"Hello from UXC\"}"}'
```

## Fallback Equivalence

- `dingtalk-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.dingtalk.com/v1.0 --schema-url <dingtalk_openapi_schema> <operation> ...`.
