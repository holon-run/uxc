# DingTalk Messaging Skill - Usage Patterns

## Link Setup

```bash
command -v dingtalk-openapi-cli
uxc link dingtalk-openapi-cli https://api.dingtalk.com/v1.0 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/dingtalk-openapi-skill/references/dingtalk-messaging.openapi.json
dingtalk-openapi-cli -h
```

## Token Bootstrap

Preferred path: let `uxc` manage app-token bootstrap and refresh from app credentials.

```bash
uxc auth credential set dingtalk-app \
  --auth-type bearer \
  --field app_key=env:DINGTALK_APP_KEY \
  --field app_secret=env:DINGTALK_APP_SECRET

uxc auth bootstrap set dingtalk-app \
  --token-endpoint https://api.dingtalk.com/v1.0/oauth2/accessToken \
  --header 'Content-Type=application/json' \
  --request-json '{"appKey":"{{field:app_key}}","appSecret":"{{field:app_secret}}"}' \
  --access-token-pointer /accessToken \
  --expires-in-pointer /expireIn

uxc auth bootstrap info dingtalk-app
```

Manual fallback:

```bash
curl -sS https://api.dingtalk.com/v1.0/oauth2/accessToken \
  -H 'Content-Type: application/json' \
  -d '{"appKey":"dingxxxx","appSecret":"xxxx"}'
```

Store the resulting `accessToken` in an environment variable before binding it into `uxc auth` if you are using the manual fallback.

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
