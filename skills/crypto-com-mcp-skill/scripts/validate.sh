#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/crypto-com-mcp-skill"
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

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

rg -q '^name:\s*crypto-com-mcp-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v crypto-com-mcp-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'https://mcp\.crypto\.com/market-data/mcp' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing fixed Crypto.com MCP endpoint'
rg -q 'uxc link crypto-com-mcp-cli https://mcp\.crypto\.com/market-data/mcp' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing fixed link create command'
rg -F -q 'https://mcp.crypto.com/docs' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing official docs reference'
for op in get_instruments get_ticker get_book get_candlestick get_trades; do
  rg -q "${op}" "${SKILL_FILE}" "${USAGE_FILE}" || fail "missing ${op}"
done
rg -q 'read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'

if rg -q -- '--input-json|crypto-com-mcp-cli list|crypto-com-mcp-cli describe|crypto-com-mcp-cli call' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy command pattern'
fi

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Crypto.com MCP"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$crypto-com-mcp-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $crypto-com-mcp-skill'

echo "skills/crypto-com-mcp-skill validation passed"
