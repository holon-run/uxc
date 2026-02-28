# Validation Rules

## Required checks

1. Frontmatter exists and includes only required routing fields:
   - `name`
   - `description`
2. Required files exist:
   - `SKILL.md`
   - `agents/openai.yaml`
   - `references/usage-patterns.md`
   - `scripts/validate.sh`
3. Link-first pattern is explicit:
   - `command -v <link_name>`
   - `uxc link <link_name> <host>`
4. Help-first pattern is explicit:
   - `<link_name> -h`
   - `<link_name> <operation> -h`
5. Execution input style is explicit:
   - at least one `key=value` example
   - at least one bare JSON positional example

## Banned defaults

Reject these in default examples and workflow text:

- `list`/`describe`/`call` old command framing
- removed input flags or deprecated invocation forms
- raw JSON passed through `--args`
- dynamic runtime command renaming for link conflicts

## OAuth and error handling boundary

For wrapper skills, keep provider-specific notes minimal and reuse `skills/uxc` for canonical OAuth and error playbooks.
