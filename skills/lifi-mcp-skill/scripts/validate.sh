#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/lifi-mcp-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
ENDPOINT="https://mcp.li.quest/mcp"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg
need_cmd jq
need_cmd uxc

required_files=(
  "${SKILL_FILE}"
  "${OPENAI_FILE}"
  "${SKILL_DIR}/references/usage-patterns.md"
)

for file in "${required_files[@]}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

head -n 1 "${SKILL_FILE}" | rg -q '^---$' || fail 'SKILL.md must include YAML frontmatter'
tail -n +2 "${SKILL_FILE}" | rg -q '^---$' || fail 'SKILL.md must include YAML frontmatter'
rg -q '^name:\s*lifi-mcp-skill\s*$' "${SKILL_FILE}" || fail 'SKILL.md frontmatter must define: name: lifi-mcp-skill'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'SKILL.md frontmatter must define a description'

rg -q 'https://mcp.li.quest/mcp' "${SKILL_FILE}" || fail 'SKILL.md must document MCP endpoint'
rg -q 'command -v lifi-mcp-cli' "${SKILL_FILE}" || fail 'SKILL.md must include link command existence check'
rg -q 'uxc link lifi-mcp-cli https://mcp.li.quest/mcp' "${SKILL_FILE}" || fail 'SKILL.md must include fixed link creation command'
rg -q 'lifi-mcp-cli -h' "${SKILL_FILE}" || fail 'SKILL.md must use lifi-mcp-cli help-first discovery'
rg -q 'get-quote' "${SKILL_FILE}" || fail 'SKILL.md must document get-quote tool'
rg -q 'references/usage-patterns.md' "${SKILL_FILE}" || fail 'SKILL.md must reference usage-patterns.md'
rg -q 'equivalent to `uxc https://mcp.li.quest/mcp <operation> ...`' "${SKILL_FILE}" || fail 'SKILL.md must document fallback equivalence guidance'

if rg -q -- '--input-json|lifi-mcp-cli list|lifi-mcp-cli describe|lifi-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail 'LI.FI docs must not use list/describe/call/--input-json in default examples'
fi

if rg -q -- "--args '\\{" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail 'LI.FI docs must not pass raw JSON via --args'
fi

rg -q '^\s*display_name:\s*"LI\.FI MCP"\s*$' "${OPENAI_FILE}" || fail 'agents/openai.yaml must define interface.display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'agents/openai.yaml must define interface.short_description'
rg -q '^\s*default_prompt:\s*".*\$lifi-mcp-skill.*"\s*$' "${OPENAI_FILE}" || fail 'agents/openai.yaml default_prompt must mention $lifi-mcp-skill'

uxc "$ENDPOINT" -h >/tmp/lifi-mcp-help.json
jq -e '.ok == true and .protocol == "mcp" and (.data.operations | length) > 0' </tmp/lifi-mcp-help.json >/dev/null || fail 'endpoint help probe failed'

echo 'skills/lifi-mcp-skill validation passed'
