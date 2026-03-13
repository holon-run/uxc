#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/mexc-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/mexc-spot.openapi.json"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd jq
need_cmd rg

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}" "${SCHEMA_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

jq -e '.openapi and .paths and .components' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'invalid OpenAPI schema JSON or missing .openapi/.paths/.components'
jq -e '.paths["/api/v3/ticker/price"] and .paths["/api/v3/account"] and .paths["/api/v3/order"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected MEXC paths'

rg -q '^name:\s*mexc-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q 'command -v mexc-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link mexc-openapi-cli https://api.mexc.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q -- '--signer-json' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing signer-json guidance'
rg -q 'X-MEXC-APIKEY' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing API key header guidance'
rg -q 'high-risk' "${SKILL_FILE}" || fail 'missing write guardrail'
rg -q 'uxc auth binding match https://api.mexc.com/api/v3/account' "${SKILL_FILE}" || fail 'missing binding match example'
rg -q '^\s*display_name:\s*"MEXC Spot"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*default_prompt:\s*".*\$mexc-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $mexc-openapi-skill'

echo "skills/mexc-openapi-skill validation passed"
