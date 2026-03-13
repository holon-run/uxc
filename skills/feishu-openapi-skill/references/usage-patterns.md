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

```bash
curl -sS https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal \
  -H 'Content-Type: application/json; charset=utf-8' \
  -d '{"app_id":"cli_xxx","app_secret":"xxxx"}'
```

Store the resulting `tenant_access_token` in an environment variable before binding it into `uxc auth`.

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
# Send a text message to a chat
feishu-openapi-cli post:/im/v1/messages receive_id_type=chat_id '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"Hello from UXC\"}"}'

# Send a text message to a user by open_id
feishu-openapi-cli post:/im/v1/messages receive_id_type=open_id '{"receive_id":"ou_xxx","msg_type":"text","content":"{\"text\":\"Hello from UXC\"}"}'

# Reply to a message
feishu-openapi-cli post:/im/v1/messages/{message_id}/reply '{"message_id":"om_xxx","content":"{\"text\":\"Reply from UXC\"}","msg_type":"text"}'
```

## Fallback Equivalence

- `feishu-openapi-cli <operation> ...` is equivalent to
  `uxc https://open.feishu.cn/open-apis --schema-url <feishu_openapi_schema> <operation> ...`.
