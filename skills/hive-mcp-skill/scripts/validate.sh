#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/hive-mcp-skill"
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

rg -q '^name:\s*hive-mcp-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v hive-mcp-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link hive-mcp-cli https://hiveintelligence.xyz/mcp' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing fixed link create command'
rg -q 'get_market_and_price_endpoints' "${SKILL_FILE}" || fail 'missing category discovery guidance'
rg -q 'get_api_endpoint_schema' "${SKILL_FILE}" || fail 'missing schema inspection guidance'
rg -q 'invoke_api_endpoint' "${SKILL_FILE}" || fail 'missing invoke guidance'
rg -q 'convenience aggregation layer' "${SKILL_FILE}" || fail 'missing positioning guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Hive Intelligence MCP"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$hive-mcp-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $hive-mcp-skill'

echo "skills/hive-mcp-skill validation passed"
