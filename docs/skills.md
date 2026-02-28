# Skills In This Repository

This repository ships one canonical skill and several official scenario wrappers.

## Skill Catalog

- `skills/uxc`
  - Canonical reusable execution layer for remote schema-exposed interfaces.
  - Other skills should call this skill for help-first discovery and operation execution patterns.
- `skills/deepwiki`
  - Wrapper for DeepWiki MCP workflows.
- `skills/context7`
  - Wrapper for Context7 MCP library documentation workflows.
- `skills/notion-mcp-skill`
  - Wrapper for Notion MCP workflows with OAuth and guarded-write guidance.

## Recommended Usage Model

1. Install and rely on `skills/uxc` as the base capability.
2. Add wrapper skills only for repeated service-specific workflows.
3. Keep wrapper logic thin and delegate generic protocol execution to `skills/uxc`.

## Install For Codex

Install canonical `uxc` skill:

```bash
python ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo holon-run/uxc \
  --path skills/uxc
```

Install an official wrapper (example: deepwiki):

```bash
python ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo holon-run/uxc \
  --path skills/deepwiki
```

Replace `skills/deepwiki` with `skills/context7` or `skills/notion-mcp-skill` as needed.

After installation, restart Codex to load new skills.

## Maintenance Rules

- Keep CLI examples in all skill docs aligned with current UXC syntax.
- If CLI semantics or output envelope changes, update:
  - `skills/uxc/SKILL.md`
  - `skills/uxc/references/*`
  - wrapper skill docs that include command snippets
- Validate canonical skill docs:

```bash
bash skills/uxc/scripts/validate.sh
```

- Validate Notion wrapper docs when touched:

```bash
bash skills/notion-mcp-skill/scripts/validate.sh
```
