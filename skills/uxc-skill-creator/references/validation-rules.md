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
6. Link naming convention is explicit:
   - use `<provider>-<protocol>-cli`
   - keep the name fixed for the skill
7. Discovery workflow is explicit:
   - start from user-provided host
   - include search + `uxc <endpoint> -h` probe before finalizing protocol/path
8. Auth detection workflow is explicit:
   - include how auth requirement is determined from probe/first call errors
   - include binding verification when OAuth/binding is used

## Banned defaults

Reject these in default examples and workflow text:

- `list`/`describe`/`call` old command framing
- removed input flags or deprecated invocation forms
- raw JSON passed through `--args`
- dynamic runtime command renaming for link conflicts
- protocol/path/auth assumptions made without verification

## OAuth and error handling boundary

For wrapper skills, keep provider-specific notes minimal and reuse `skills/uxc` for canonical OAuth and error playbooks.
