#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/context7"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

# Check dependencies
need_cmd rg

required_files=(
  "${SKILL_FILE}"
  "${OPENAI_FILE}"
  "${SKILL_DIR}/references/usage-patterns.md"
)

for file in "${required_files[@]}"; do
  if [[ ! -f "${file}" ]]; then
    fail "missing required file: ${file}"
  fi
done

# Validate SKILL frontmatter minimum fields.
# Require the first line to be '---' and a subsequent closing '---'.
if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! rg -q '^name:\s*context7\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: context7"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

# Validate required invocation contract appears in SKILL text.
if ! rg -q 'mcp.context7.com/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must document MCP endpoint"
fi

if ! rg -q 'resolve-library-id' "${SKILL_FILE}"; then
  fail "SKILL.md must document resolve-library-id tool"
fi

if ! rg -q 'query-docs' "${SKILL_FILE}"; then
  fail "SKILL.md must document query-docs tool"
fi

if ! rg -q 'command -v context7-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link context7-mcp-cli mcp.context7.com/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must include fixed link creation command"
fi

if ! rg -q 'context7-mcp-cli -h' "${SKILL_FILE}"; then
  fail "SKILL.md must use context7-mcp-cli help-first discovery"
fi

# Validate preferred input style appears in SKILL text.
if ! rg -q "resolve-library-id libraryName=" "${SKILL_FILE}"; then
  fail "SKILL.md must prefer key=value examples for resolve-library-id"
fi

if ! rg -q "query-docs .*'\\{.*\\}'" "${SKILL_FILE}"; then
  fail "SKILL.md must include a bare JSON positional example"
fi

if rg -q -- '--input-json|context7-mcp-cli list|context7-mcp-cli describe|context7-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "context7 docs must not use list/describe/call/--input-json in default examples"
fi

if rg -q -- "--args '\\{" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "context7 docs must not pass raw JSON via --args"
fi

# Validate references linked from SKILL body.
if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc mcp.context7.com/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "context7 docs must include single-point fallback equivalence guidance"
fi

if rg -qi 'retry with .*suffix|append.*suffix|dynamic rename|auto-rename' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "context7 docs must not include dynamic command renaming guidance"
fi

# Validate openai.yaml minimum fields.
if ! rg -q '^\s*display_name:\s*"Context7"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$context7.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $context7'
fi

echo "skills/context7 validation passed"
