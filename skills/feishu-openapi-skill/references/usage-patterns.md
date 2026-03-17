# Feishu / Lark IM Skill - Usage Patterns

## Link Setup

```bash
command -v feishu-openapi-cli
uxc link feishu-openapi-cli https://open.feishu.cn/open-apis \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/feishu-openapi-skill/references/feishu-im.openapi.json
feishu-openapi-cli -h
```

For international Lark tenants, use the same schema against `https://open.larksuite.com/open-apis`.

## Token Bootstrap

Preferred path: let `uxc` manage tenant-token bootstrap and refresh from app credentials.

```bash
uxc auth credential set feishu-tenant \
  --auth-type bearer \
  --field app_id=env:FEISHU_APP_ID \
  --field app_secret=env:FEISHU_APP_SECRET

uxc auth bootstrap set feishu-tenant \
  --token-endpoint https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal \
  --header 'Content-Type=application/json; charset=utf-8' \
  --request-json '{"app_id":"{{field:app_id}}","app_secret":"{{field:app_secret}}"}' \
  --access-token-pointer /tenant_access_token \
  --expires-in-pointer /expire \
  --success-code-pointer /code \
  --success-code-value 0

uxc auth bootstrap info feishu-tenant
```

For long-connection subscribe, keep this same credential because the transport needs the stored `app_id` and `app_secret` fields to open a fresh temporary WebSocket URL.

Manual fallback:

For Feishu tenants:

```bash
curl -sS https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal \
  -H 'Content-Type: application/json; charset=utf-8' \
  -d '{"app_id":"cli_xxx","app_secret":"xxxx"}'
```

For Lark tenants, call the same path on the Lark host:

```bash
curl -sS https://open.larksuite.com/open-apis/auth/v3/tenant_access_token/internal \
  -H 'Content-Type: application/json; charset=utf-8' \
  -d '{"app_id":"cli_xxx","app_secret":"xxxx"}'
```

Store the resulting `tenant_access_token` in an environment variable before binding it into `uxc auth` if you are using the manual fallback.

## Auth Setup

```bash
uxc auth credential set feishu-tenant \
  --auth-type bearer \
  --secret-env FEISHU_TENANT_ACCESS_TOKEN

uxc auth binding add \
  --id feishu-tenant \
  --host open.feishu.cn \
  --path-prefix /open-apis \
  --scheme https \
  --credential feishu-tenant \
  --priority 100
```

## Read Examples

```bash
# List chats visible to the app
feishu-openapi-cli get:/im/v1/chats page_size=20

# Inspect one chat
feishu-openapi-cli get:/im/v1/chats/{chat_id} chat_id=oc_xxx

# List chat members
feishu-openapi-cli get:/im/v1/chats/{chat_id}/members chat_id=oc_xxx page_size=50

# Read recent messages from one chat
feishu-openapi-cli get:/im/v1/messages container_id_type=chat container_id=oc_xxx page_size=20

# Read one message by id
feishu-openapi-cli get:/im/v1/messages/{message_id} message_id=om_xxx

# Look up a user profile
feishu-openapi-cli get:/contact/v3/users/{user_id} user_id=ou_xxx user_id_type=open_id
```

## Write Examples (Confirm Intent First)

```bash
# Upload an image and capture the returned image_key
feishu-openapi-cli post:/im/v1/images image_type=message image=/tmp/example.png

# Upload a file and capture the returned file_key
feishu-openapi-cli post:/im/v1/files file_type=stream file_name=report.txt file=/tmp/report.txt

# Send a text message to a chat
feishu-openapi-cli post:/im/v1/messages receive_id_type=chat_id '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"Hello from UXC\"}"}'

# Send an image message using a previously uploaded image_key
feishu-openapi-cli post:/im/v1/messages receive_id_type=chat_id '{"receive_id":"oc_xxx","msg_type":"image","content":"{\"image_key\":\"img_xxx\"}"}'

# Send a file message using a previously uploaded file_key
feishu-openapi-cli post:/im/v1/messages receive_id_type=chat_id '{"receive_id":"oc_xxx","msg_type":"file","content":"{\"file_key\":\"file_xxx\"}"}'

# Send a text message to a user by open_id
feishu-openapi-cli post:/im/v1/messages receive_id_type=open_id '{"receive_id":"ou_xxx","msg_type":"text","content":"{\"text\":\"Hello from UXC\"}"}'

# Reply to a message
feishu-openapi-cli post:/im/v1/messages/{message_id}/reply '{"message_id":"om_xxx","content":"{\"text\":\"Reply from UXC\"}","msg_type":"text"}'
```

## Long-Connection Subscribe

Use `uxc subscribe` directly for inbound Feishu/Lark message intake:

```bash
uxc subscribe start https://open.feishu.cn/open-apis \
  --transport feishu-long-connection \
  --auth feishu-tenant \
  --sink file:$HOME/.uxc/subscriptions/feishu.ndjson
```

What to expect:

- the sink starts with an `open` event containing the temporary Feishu WebSocket URL
- inbound bot-visible messages arrive as `data` events
- real Feishu IM events use `data.header.event_type`, for example:
  - `im.message.receive_v1`

Important notes:

- this transport is provider-aware; it is not a generic raw WebSocket JSON stream
- the runtime handles temporary URL bootstrap, protobuf binary frames, event acknowledgements, and ping control frames automatically
- use a credential that still has `app_id` and `app_secret` fields, even if request/response calls are already using bootstrap-managed bearer auth

## Fallback Equivalence

- `feishu-openapi-cli <operation> ...` is equivalent to
  `uxc https://open.feishu.cn/open-apis --schema-url <feishu_openapi_schema> <operation> ...`.
