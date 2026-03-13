---
name: whatsapp-openapi-skill
description: Operate WhatsApp Business Platform Cloud API through UXC with a curated OpenAPI schema, bearer-token auth, and message/profile guardrails.
---

# WhatsApp Cloud API Skill

Use this skill to run WhatsApp Business Platform Cloud API operations through `uxc` + OpenAPI.

Reuse the `uxc` skill for shared execution, auth, and error-handling guidance.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `https://graph.facebook.com/v25.0`.
- Access to the curated OpenAPI schema URL:
  - `https://raw.githubusercontent.com/holon-run/uxc/main/skills/whatsapp-openapi-skill/references/whatsapp-cloud.openapi.json`
- A Meta app and WhatsApp Business account with Cloud API access.
- A valid system-user or app access token that can call the target WhatsApp assets.
- At least one `phone_number_id`, and for phone number listing, the related `waba_id`.

## Scope

This skill covers a compact Cloud API request/response surface:

- phone number listing
- phone number metadata reads
- business profile reads and updates
- outbound message sends

This skill does **not** cover:

- inbound webhook receiver runtime
- template lifecycle management
- embedded signup or broader business onboarding flows
- media upload/download lifecycle
- the full WhatsApp Business Platform surface

## API Version

This skill is pinned to Graph API `v25.0`, based on current Meta developer examples at implementation time. Keep the base URL versioned:

- `https://graph.facebook.com/v25.0`

If Meta deprecates this version later, the wrapper should be revised in a follow-up update rather than assuming unversioned compatibility.

## Authentication

WhatsApp Cloud API uses `Authorization: Bearer <access_token>`.

Configure one bearer credential and bind it to the versioned Graph API base path:

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

Validate the active mapping when auth looks wrong:

```bash
uxc auth binding match https://graph.facebook.com/v25.0
```

## Core Workflow

1. Use the fixed link command by default:
   - `command -v whatsapp-openapi-cli`
   - If missing, create it:
     `uxc link whatsapp-openapi-cli https://graph.facebook.com/v25.0 --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/whatsapp-openapi-skill/references/whatsapp-cloud.openapi.json`
   - `whatsapp-openapi-cli -h`

2. Inspect operation schema first:
   - `whatsapp-openapi-cli get:/{waba_id}/phone_numbers -h`
   - `whatsapp-openapi-cli get:/{phone_number_id}/whatsapp_business_profile -h`
   - `whatsapp-openapi-cli post:/{phone_number_id}/messages -h`

3. Prefer read/setup validation before writes:
   - `whatsapp-openapi-cli get:/{waba_id}/phone_numbers waba_id=123456789012345`
   - `whatsapp-openapi-cli get:/{phone_number_id} phone_number_id=123456789012345`
   - `whatsapp-openapi-cli get:/{phone_number_id}/whatsapp_business_profile phone_number_id=123456789012345`

4. Execute with key/value or positional JSON:
   - key/value:
     `whatsapp-openapi-cli get:/{phone_number_id} phone_number_id=123456789012345 fields=display_phone_number,verified_name`
   - positional JSON:
     `whatsapp-openapi-cli post:/{phone_number_id}/messages '{"phone_number_id":"123456789012345","messaging_product":"whatsapp","to":"15551234567","type":"text","text":{"body":"Hello from UXC"}}'`

## Operation Groups

### Asset Discovery

- `get:/{waba_id}/phone_numbers`
- `get:/{phone_number_id}`

### Business Profile

- `get:/{phone_number_id}/whatsapp_business_profile`
- `post:/{phone_number_id}/whatsapp_business_profile`

### Messaging

- `post:/{phone_number_id}/messages`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable fields first: `ok`, `kind`, `protocol`, `data`, `error`.
- `post:/{phone_number_id}/messages` is a high-risk write. Require explicit user confirmation before execution.
- Message delivery is still constrained by WhatsApp conversation rules, template approval rules, recipient opt-in expectations, and account policy state. Auth success does not imply a send is allowed.
- This v1 skill does not manage media uploads. If you send `image` or `document` content, use already hosted URLs or existing asset references as allowed by the platform.
- Webhook subscription and verification are intentionally documented only. This skill does not configure a receiver runtime.
- Business profile update fields are partial. Only send the fields you intend to change.
- `whatsapp-openapi-cli <operation> ...` is equivalent to `uxc https://graph.facebook.com/v25.0 --schema-url <whatsapp_openapi_schema> <operation> ...`.

## References

- Usage patterns: `references/usage-patterns.md`
- Curated OpenAPI schema: `references/whatsapp-cloud.openapi.json`
- WhatsApp Cloud API docs: https://developers.facebook.com/docs/whatsapp/cloud-api
- Graph API access tokens: https://developers.facebook.com/docs/graph-api/overview/access-tokens/
