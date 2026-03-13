#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/coinapi-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/coinapi-market.openapi.json"

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
jq -e '.paths["/v1/exchangerate/{asset_id_base}/{asset_id_quote}"] and .paths["/v1/trades/{symbol_id}/latest"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected CoinAPI paths'

rg -q '^name:\s*coinapi-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v coinapi-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link coinapi-openapi-cli https://rest.coinapi.io --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -F -q 'coinapi-openapi-cli get:/v1/exchangerate/{asset_id_base}/{asset_id_quote} -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--api-key-header X-CoinAPI-Key' "${SKILL_FILE}" || fail 'missing X-CoinAPI-Key auth setup'
rg -q 'uxc auth binding match https://rest.coinapi.io' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"CoinAPI REST"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$coinapi-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $coinapi-openapi-skill'

echo "skills/coinapi-openapi-skill validation passed"
