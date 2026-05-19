# Feishu / Lark OpenAPI Schema Maintenance

This note tracks how UXC should maintain the Feishu/Lark OpenAPI JSON used by
`skills/feishu-openapi-skill`.

## Problem

The current skill schema is a small curated OpenAPI document:

- `skills/feishu-openapi-skill/references/feishu-im.openapi.json`
- current focus: IM messages, chats, uploads, and basic contact lookup

This is useful for stable agent workflows, but it is not a complete Feishu/Lark
Open Platform surface. Issue #407 exposed the gap for bot self-identity:

- required endpoint: `GET /open-apis/bot/v3/info`
- use case: AgentInbox resolves the configured app bot `open_id` without
  shipping its own tiny provider schema

## Source Of Truth

Feishu/Lark does not currently expose a public standard OpenAPI 3 JSON document
through the same endpoint used by the official CLI.

The official `larksuite/cli` registry fetches a custom metadata protocol:

```text
https://open.feishu.cn/api/tools/open/api_definition?protocol=meta&client_version=<version>
https://open.larksuite.com/api/tools/open/api_definition?protocol=meta&client_version=<version>
```

The same endpoint rejects `protocol=openapi`, `protocol=swagger`, and
`protocol=json`, so UXC should treat `protocol=meta` as an upstream metadata
source, not as an OpenAPI document.

The metadata still carries enough structure to generate OpenAPI operations:

- `servicePath`
- `resources.<resource>.methods.<method>.httpMethod`
- `path`
- `parameters`
- `requestBody`
- `responseBody`
- `scopes`
- `accessTokens`
- `docUrl`

However, this metadata is not full coverage by itself. Some common IM endpoints
are implemented by `larksuite/cli` as shortcuts rather than registry methods,
including send/list/reply message workflows.

## Maintenance Strategy

Maintain the Feishu/Lark schema through three layers, in this order.

### 1. Generate From Official Metadata

Build a converter that fetches `api_definition?protocol=meta` and writes an
OpenAPI 3 document.

Current generator:

```bash
python3 skills/feishu-openapi-skill/scripts/generate_openapi.py
```

The generator writes:

- `skills/feishu-openapi-skill/references/feishu-im.openapi.json`

It applies this curated overlay after metadata conversion:

- `skills/feishu-openapi-skill/references/feishu-openapi.overlay.json`

The converter should:

- support Feishu and Lark hosts
- cache the fetched upstream metadata with its `version`
- map `servicePath + method.path` into OpenAPI paths
- preserve method identity in `operationId`
- map `parameters` into path/query/header parameters
- map `requestBody` and `responseBody` into JSON schemas
- map file fields into `multipart/form-data`
- preserve Feishu/Lark-specific metadata with extensions such as
  `x-feishu-scopes`, `x-feishu-access-tokens`, `x-feishu-doc-url`, and
  `x-feishu-source`
- emit deterministic JSON so diffs are reviewable

Initial generator scope is the skill's current surface plus the bot identity
service needed by #407. After the converter is stable, broaden the selected
services incrementally.

### 2. Patch From CLI And Skill Knowledge

Use `larksuite/cli` as an implementation reference for endpoints missing from
the registry metadata.

Useful local sources:

- `scripts/fetch_meta.py`: official metadata fetch shape
- `internal/registry/remote.go`: Feishu/Lark metadata endpoint and cache rules
- `cmd/schema/schema.go`: method metadata interpretation
- `shortcuts/im/*`: hand-written IM endpoints and guardrails
- `skills/lark-openapi-explorer`: documented fallback to Feishu/Lark docs

Manual patches should be explicit, small, and source-attributed. Prefer a
structured overlay file over ad hoc edits to generated JSON.

### 3. Fill Gaps From Official `llms.txt`

For endpoints that are neither in `protocol=meta` nor clearly represented in
CLI shortcuts, use the official documentation index:

```text
https://open.feishu.cn/llms.txt
https://open.larksuite.com/llms.txt
```

The extraction flow should be:

1. find the relevant module from `llms.txt`
2. load the module `llms-*.txt`
3. load the API markdown document
4. extract method, path, parameters, request body, response body, permissions,
   and notes
5. add the endpoint through the same structured overlay path used for CLI gaps

## Periodic Update Task

Run this maintenance task on a regular cadence, and also whenever AgentInbox or
another provider integration reports a missing Feishu/Lark endpoint.

Recommended cadence: weekly while Feishu integration is active, otherwise before
each UXC release.

Checklist:

1. Fetch fresh Feishu and Lark `protocol=meta` payloads.
2. Regenerate the OpenAPI JSON.
3. Apply CLI/skill/`llms.txt` overlays.
4. Compare generated output with the committed schema.
5. Review added/removed operations and permission changes.
6. Run `bash skills/feishu-openapi-skill/scripts/validate.sh`.
7. Smoke at least one read operation and any newly added operation that can be
   safely called with test credentials.
8. Update this note or the overlay comments when upstream behavior changes.

## First Closure Target: Issue #407

The first implementation pass should close #407 by making this UXC operation
available through the official Feishu/Lark schema path:

```text
GET /open-apis/bot/v3/info
```

Acceptance for that pass:

- UXC can resolve the configured bot identity with existing bootstrap-managed
  app credentials.
- The schema exposes at least `bot.open_id` and `bot.app_name`.
- `skills/feishu-openapi-skill/SKILL.md` documents the operation.
- AgentInbox can remove its temporary mini schema for bot identity discovery.
- A small validation or smoke test covers the endpoint shape.
