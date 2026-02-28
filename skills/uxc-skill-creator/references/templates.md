# Templates

## Minimal Wrapper SKILL.md Template

```markdown
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

1. Use fixed link command by default:
   - `command -v <link_name>`
   - If missing, create it: `uxc link <link_name> <host>`
   - `<link_name> -h`
2. Inspect operation schema:
   - `<link_name> <operation> -h`
3. Execute operation:
   - `<link_name> <operation> field=value`
   - `<link_name> <operation> '{"field":"value"}'`

## Guardrails

- Parse JSON envelope fields (`ok`, `data`, `error`).
- Require explicit user confirmation for destructive writes.
- `<link_name> <operation> ...` is equivalent to `uxc <host> <operation> ...`.

## References

- `references/usage-patterns.md`
```

## Minimal usage-patterns.md Template

```markdown
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
```

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

if rg -q -- 'list|describe|call|--input-json|--args .*\{' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail 'found banned legacy patterns'
fi

echo "skills/<skill-name> validation passed"
```
