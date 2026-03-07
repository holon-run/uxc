# Templates

## Minimal Wrapper SKILL.md Template

````markdown
---
name: <skill-name>
description: <what it does + when to use>
---

# <Skill Title>

Use this skill to run <provider> operations through `uxc`.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `<host>`.

## Core Workflow

1. Confirm endpoint/protocol/auth from user host:
   - search official docs for canonical endpoint and auth requirements
   - probe with `uxc <host> -h` (and endpoint variants if needed)
2. Use fixed link command by default:
   - `command -v <link_name>`
   - If missing, create it: `uxc link <link_name> <host>`
   - `<link_name> -h`
3. Inspect operation schema:
   - `<link_name> <operation> -h`
4. Execute operation:
   - `<link_name> <operation> field=value`
   - `<link_name> <operation> '{"field":"value"}'`

## Guardrails

- Parse JSON envelope fields (`ok`, `data`, `error`).
- Require explicit user confirmation for destructive writes.
- `<link_name> <operation> ...` is equivalent to `uxc <host> <operation> ...`.
- When OAuth/binding is used, include local mapping check:
  - `uxc auth binding match <endpoint>`

## References

- `references/usage-patterns.md`
````

Link naming convention to apply in templates:

- `<provider>-<protocol>-cli`
- keep the name fixed at skill-authoring time
- do not auto-rename at runtime

## Minimal usage-patterns.md Template

````markdown
# Usage Patterns

```bash
command -v <link_name>
uxc link <link_name> <host>
<link_name> -h
```

## Read path

```bash
<link_name> <read_operation> field=value
```

## Bare JSON positional example

```bash
<link_name> <operation> '{"field":"value"}'
```

## Fallback equivalence

- `<link_name> <operation> ...` is equivalent to `uxc <host> <operation> ...`.
````

Replace placeholders before use:

- `<skill-name>` -> actual folder name
- `<link_name>` -> fixed command name
- `<host>` -> verified endpoint
- `<operation>` / `<read_operation>` -> real operation IDs

## Minimal validate.sh Template

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/<skill-name>"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"

fail() { printf '[validate] error: %s\n' "$*" >&2; exit 1; }
need_cmd() { command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"; }

need_cmd rg

for f in "${SKILL_FILE}" "${OPENAI_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; do
  [[ -f "$f" ]] || fail "missing required file: $f"
done

rg -q '^name:\s*<skill-name>\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v <link_name>' "${SKILL_FILE}" || fail 'missing link-first check'
rg -q '<link_name> -h' "${SKILL_FILE}" || fail 'missing help-first usage'

if rg -q -- '(^|[[:space:]])uxc <host> (list|describe|call)([[:space:]]|$)|(^|[[:space:]])<link_name> (list|describe|call)([[:space:]]|$)|--input-json|--args .*\{' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail 'found banned legacy patterns'
fi

echo "skills/<skill-name> validation passed"
```
