---
name: uxc-skill-creator
description: Create wrapper skills that call remote tools through UXC. Use when defining a new provider skill and you need reusable templates, validation rules, and anti-pattern guidance based on proven UXC skill practices.
---

# UXC Skill Creator

Use this skill to design and standardize provider wrapper skills built on top of `uxc`.

## Prerequisites

- `uxc` skill is available as the base execution contract.
- Target wrapper skill scope is clear (provider endpoint, core operations, auth model, write risk).

## Output Contract

A wrapper skill created with this skill should include:

- `SKILL.md`
- `agents/openai.yaml`
- `references/usage-patterns.md`
- `scripts/validate.sh`

Optional files are allowed only when they add real reusable value.

## Core Workflow

1. Fix the wrapper interface first:
   - provider endpoint (`<host>`)
   - fixed link command name (`<provider>-<protocol>-cli`)
   - auth mode (none, api key, oauth)
2. Write `SKILL.md` as a thin execution policy:
   - link-first command flow
   - help-first discovery flow
   - JSON envelope parsing and safe-write guardrails
3. Add provider-specific `references/usage-patterns.md`:
   - minimal read and write examples
   - key=value and bare JSON positional input examples
4. Add `scripts/validate.sh` with strict checks:
   - required files
   - frontmatter fields
   - command style constraints
   - banned legacy patterns
5. Add `agents/openai.yaml` for skill UI metadata.
6. Run validation and iterate until clean.

## Hard Rules

- Default to link-first (`command -v <link_name>` then `uxc link <link_name> <host>`).
- Default to help-first (`<link_name> -h`, `<link_name> <operation> -h`).
- Use protocol-aware link naming:
  - format: `<provider>-<protocol>-cli`
  - examples: `notion-mcp-cli`, `github-openapi-cli`
- Prefer `key=value`; allow bare JSON positional payload.
- Keep JSON output as automation path; do not rely on `--text`.
- Do not use legacy default examples (`list`/`describe`/`call`/removed flags).
- Do not use dynamic link renaming at runtime.

## References

- Step-by-step implementation flow:
  - `references/workflow.md`
- Copy-ready templates:
  - `references/templates.md`
- Validation checklist and banned patterns:
  - `references/validation-rules.md`
- Observed pitfalls and better defaults:
  - `references/anti-patterns.md`
