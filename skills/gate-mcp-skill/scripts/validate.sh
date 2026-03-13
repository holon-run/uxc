#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/gate-mcp-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}" "${SKILL_DIR}/scripts/validate.sh"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

rg -q '^name:\s*gate-mcp-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'https://api\.gatemcp\.ai/mcp' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing Gate MCP endpoint'
rg -q 'command -v gate-mcp-cli' "${SKILL_FILE}" || fail 'missing link-first check'
rg -q 'uxc link gate-mcp-cli https://api\.gatemcp\.ai/mcp' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing fixed link command'
rg -q 'gate-mcp-cli -h' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing help-first discovery'
rg -q 'references/usage-patterns\.md' "${SKILL_FILE}" || fail 'missing usage-patterns reference'
rg -q 'equivalent to `uxc https://api\.gatemcp\.ai/mcp' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing fallback equivalence guidance'
for op in cex_spot_get_spot_tickers cex_spot_get_spot_order_book cex_fx_get_fx_tickers cex_fx_get_fx_order_book cex_fx_get_fx_funding_rate; do
  rg -q "${op}" "${SKILL_FILE}" "${USAGE_FILE}" || fail "missing ${op}"
done
rg -q 'read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'

if rg -q -- '--input-json|gate-mcp-cli list|gate-mcp-cli describe|gate-mcp-cli call' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy command pattern'
fi

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Gate MCP"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$gate-mcp-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $gate-mcp-skill'

echo "skills/gate-mcp-skill validation passed"
