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

1. Start from user-provided host input:
   - record the raw host the user gives
   - normalize endpoint candidates (scheme/no-scheme, path variants)
2. Discover protocol and valid path before drafting skill text:
   - search official docs/repo to confirm endpoint shape and auth model
   - probe candidates with `uxc <endpoint> -h`
   - confirm one working endpoint + protocol as the wrapper target
3. Detect authentication requirement explicitly:
   - run host help or a minimal read call and inspect envelope/error code
   - if auth-protected, record required model (api key or oauth) and scopes
   - verify local mapping path with `uxc auth binding match <endpoint>` when OAuth/binding is used
4. Fix the wrapper interface:
   - provider endpoint (`<host>`)
   - fixed link command name (`<provider>-<protocol>-cli`)
   - auth mode (none, api key, oauth)
5. Write `SKILL.md` as a thin execution policy:
   - link-first command flow
   - help-first discovery flow
   - JSON envelope parsing and safe-write guardrails
6. Add provider-specific `references/usage-patterns.md`:
   - minimal read and write examples
   - key=value and bare JSON positional input examples
7. Add `scripts/validate.sh` with strict checks:
   - required files
   - frontmatter fields
   - command style constraints
   - banned legacy patterns
8. Add `agents/openai.yaml` for skill UI metadata.
9. Run validation and iterate until clean.

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
- Do not assume protocol/path/auth from host string alone; verify by search + probe.

## References

- Step-by-step implementation flow:
  - `references/workflow.md`
- Copy-ready templates:
  - `references/templates.md`
- Validation checklist and banned patterns:
  - `references/validation-rules.md`
- Observed pitfalls and better defaults:
  - `references/anti-patterns.md`

## See Also

- Base execution and protocol/auth guidance:
  - `skills/uxc/SKILL.md`
