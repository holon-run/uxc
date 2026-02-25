# Skills In This Repository

## Canonical Skill

UXC exposes one canonical reusable skill:

- `skills/uxc`

This skill is the execution abstraction for remote schema-exposed interfaces. Other skills should reuse it when they need API/tool calls.

## Install For Codex

```bash
python ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo holon-run/uxc \
  --path skills/uxc
```

After installation, restart Codex to load the skill.

## Maintenance Rules

- Keep CLI examples in `skills/uxc` aligned with current UXC syntax.
- If CLI semantics or output envelope changes, update:
  - `skills/uxc/SKILL.md`
  - files in `skills/uxc/references/`
  - `skills/uxc/agents/openai.yaml` (if invocation wording changes)
- Run validation:

```bash
bash skills/uxc/scripts/validate.sh
```
