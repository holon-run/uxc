# WhatsApp Cloud API Skill - Usage Patterns

## Link Setup

```bash
command -v whatsapp-openapi-cli
uxc link whatsapp-openapi-cli https://graph.facebook.com/v25.0 \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/whatsapp-openapi-skill/references/whatsapp-cloud.openapi.json
whatsapp-openapi-cli -h
```

## Auth Setup

```bash
uxc auth credential set whatsapp-cloud \
  --auth-type bearer \
  --secret-env WHATSAPP_CLOUD_ACCESS_TOKEN

uxc auth binding add \
  --id whatsapp-cloud \
  --host graph.facebook.com \
  --path-prefix /v25.0 \
  --scheme https \
  --credential whatsapp-cloud \
  --priority 100
```

Validate the binding:

```bash
uxc auth binding match https://graph.facebook.com/v25.0
```

## Read Examples

```bash
# List phone numbers under one WhatsApp Business account
whatsapp-openapi-cli get:/{waba_id}/phone_numbers waba_id=123456789012345

# Read one phone number's metadata
whatsapp-openapi-cli get:/{phone_number_id} phone_number_id=123456789012345 fields=display_phone_number,verified_name

# Read the business profile attached to the phone number
whatsapp-openapi-cli get:/{phone_number_id}/whatsapp_business_profile phone_number_id=123456789012345
```

## Write Examples (Confirm Intent First)

```bash
# Send a text message
whatsapp-openapi-cli post:/{phone_number_id}/messages '{"phone_number_id":"123456789012345","messaging_product":"whatsapp","to":"15551234567","type":"text","text":{"body":"Hello from UXC"}}'

# Send a template message
whatsapp-openapi-cli post:/{phone_number_id}/messages '{"phone_number_id":"123456789012345","messaging_product":"whatsapp","to":"15551234567","type":"template","template":{"name":"hello_world","language":{"code":"en_US"}}}'

# Update business profile fields
whatsapp-openapi-cli post:/{phone_number_id}/whatsapp_business_profile '{"phone_number_id":"123456789012345","about":"Support via WhatsApp","description":"Customer support team","email":"support@example.com","websites":["https://example.com"]}'
```

## Webhook Guidance

Webhook delivery is useful for inbound messages and status callbacks, but this skill does not host a receiver runtime. Keep webhook setup and verification outside `uxc`, and use this wrapper for request/response operations only.

## Fallback Equivalence

- `whatsapp-openapi-cli <operation> ...` is equivalent to
  `uxc https://graph.facebook.com/v25.0 --schema-url <whatsapp_openapi_schema> <operation> ...`.
