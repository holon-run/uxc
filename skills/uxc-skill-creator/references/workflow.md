# Workflow

## 1. Intake host and normalize candidates

Start from the endpoint/host provided by the user.

Build a candidate list before committing to one:

- raw value from user
- shorthand host/path form
- full URL form (`https://...`)
- common provider path variants (for MCP often `/mcp`)

Keep the candidate list explicit in working notes.

## 2. Discover protocol and path (search + probe)

Use both external signals and runtime probing:

1. Search official docs or repository README for:
   - protocol type (MCP, OpenAPI, GraphQL, gRPC, JSON-RPC)
   - canonical endpoint path
   - authentication requirements
2. Probe each endpoint candidate:
   - `uxc <candidate_endpoint> -h`
3. Pick the first endpoint that gives valid operation help and stable behavior.

Do not finalize wrapper naming or examples until this step is complete.

## 3. Detect auth requirements

Lock these values before writing examples:

- Skill name (folder-safe, stable)
- Provider endpoint host/path (verified)
- Fixed link command name (`<provider>-<protocol>-cli`)
- Auth model and required scopes
- High-risk operations that require explicit user confirmation

Use protocol-aware names for cross-protocol consistency:

- MCP wrapper: `notion-mcp-cli`
- OpenAPI wrapper: `github-openapi-cli`
- GraphQL wrapper: `shopify-graphql-cli`

For OAuth/binding flows, verify local state mapping with:

```bash
uxc auth binding match <endpoint>
```

Use first real read call as runtime auth validation.

## 4. Draft wrapper `SKILL.md`

Keep it thin and operational:

- one-sentence purpose
- prerequisites
- core workflow commands
- guardrails
- references list

Delegate generic auth/error patterns to `skills/uxc` references.

## 5. Draft `references/usage-patterns.md`

Include only real recurring calls:

- bootstrap (`command -v`, `uxc link`, `<link_name> -h`)
- read path examples
- write path examples (if provider supports write)
- output parsing notes based on envelope fields

## 6. Add `agents/openai.yaml`

Provide concise UI metadata:

- `display_name`
- `short_description`
- `default_prompt`

## 7. Implement `scripts/validate.sh`

Codify non-negotiables as checks instead of prose:

- required files exist
- frontmatter has `name` and `description`
- link-first and help-first commands exist
- old patterns are rejected
- host/protocol/auth discovery workflow exists

## 8. Validate and iterate

Run:

```bash
bash skills/<skill-name>/scripts/validate.sh
```

Fix failures until validation is clean.
