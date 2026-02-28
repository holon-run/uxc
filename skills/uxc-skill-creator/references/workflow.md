# Workflow

## 1. Define provider contract

Lock these values before writing examples:

- Skill name (folder-safe, stable)
- Provider endpoint host/path
- Fixed link command name (`<provider>-mcp-cli`)
- Auth model and required scopes
- High-risk operations that require explicit user confirmation

## 2. Draft wrapper `SKILL.md`

Keep it thin and operational:

- one-sentence purpose
- prerequisites
- core workflow commands
- guardrails
- references list

Delegate generic auth/error patterns to `skills/uxc` references.

## 3. Draft `references/usage-patterns.md`

Include only real recurring calls:

- bootstrap (`command -v`, `uxc link`, `<link_name> -h`)
- read path examples
- write path examples (if provider supports write)
- output parsing notes based on envelope fields

## 4. Add `agents/openai.yaml`

Provide concise UI metadata:

- `display_name`
- `short_description`
- `default_prompt`

## 5. Implement `scripts/validate.sh`

Codify non-negotiables as checks instead of prose:

- required files exist
- frontmatter has `name` and `description`
- link-first and help-first commands exist
- old patterns are rejected

## 6. Validate and iterate

Run:

```bash
bash skills/<skill-name>/scripts/validate.sh
```

Fix failures until validation is clean.
