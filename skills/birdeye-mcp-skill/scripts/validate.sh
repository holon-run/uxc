#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/birdeye-mcp-skill"
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

if ! rg -q '^name:\s*birdeye-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: birdeye-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'mcp\.birdeye\.so/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must document the Birdeye MCP endpoint"
fi

if ! rg -q 'X-API-KEY' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must configure X-API-KEY auth"
fi

if ! rg -q 'command -v birdeye-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link birdeye-mcp-cli https://mcp\.birdeye\.so/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fixed link creation command"
fi

if ! rg -q 'birdeye-mcp-cli -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must use birdeye-mcp-cli help-first discovery"
fi

if ! rg -q 'token market data|trending|price monitoring|DEX' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must describe Birdeye capability focus"
fi

if rg -q -- '--input-json|birdeye-mcp-cli list|birdeye-mcp-cli describe|birdeye-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "Birdeye docs must not use list/describe/call/--input-json in default examples"
fi

if ! rg -q 'equivalent to `uxc https://mcp\.birdeye\.so/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "Birdeye docs must include single-point fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"Birdeye MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$birdeye-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $birdeye-mcp-skill'
fi

echo "skills/birdeye-mcp-skill validation passed"
