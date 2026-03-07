#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/dune-mcp-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

required_files=(
  "${SKILL_FILE}"
  "${OPENAI_FILE}"
  "${SKILL_DIR}/references/usage-patterns.md"
  "${SKILL_DIR}/scripts/validate.sh"
)

for file in "${required_files[@]}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! rg -q '^name:\s*dune-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: dune-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'api\.dune\.com/mcp/v1' "${SKILL_FILE}"; then
  fail "SKILL.md must document Dune MCP endpoint"
fi

if ! rg -q 'command -v dune-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link dune-mcp-cli https://api\.dune\.com/mcp/v1' "${SKILL_FILE}"; then
  fail "SKILL.md must include fixed link creation command"
fi

if ! rg -q 'dune-mcp-cli -h' "${SKILL_FILE}"; then
  fail "SKILL.md must use dune-mcp-cli help-first discovery"
fi

for op in searchTables createDuneQuery executeQueryById getExecutionResults; do
  if ! rg -q "${op}" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
    fail "Dune docs must include ${op}"
  fi
done

if rg -q -- '--input-json|dune-mcp-cli list|dune-mcp-cli describe|dune-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "Dune docs must not use list/describe/call/--input-json in default examples"
fi

if ! rg -q 'references/usage-patterns\.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc https://api\.dune\.com/mcp/v1' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "Dune docs must include single-point fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"Dune MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$dune-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $dune-mcp-skill'
fi

echo "skills/dune-mcp-skill validation passed"
