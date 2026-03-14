#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/coinbase-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/coinbase-advanced-trade.openapi.json"

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
jq -e '.paths["/api/v3/brokerage/products"] and .paths["/api/v3/brokerage/orders"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Coinbase paths'

rg -q '^name:\s*coinbase-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v coinbase-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link coinbase-openapi-cli https://api.coinbase.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'coinbase-openapi-cli get:/api/v3/brokerage/products -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'jwt_bearer_v1' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing jwt_bearer_v1 signer guidance'
rg -q 'COINBASE_KEY_ID' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing key id guidance'
rg -q 'COINBASE_PRIVATE_KEY' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing private key guidance'
rg -q 'uxc auth binding match https://api.coinbase.com/api/v3/brokerage/accounts' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'high-risk writes' "${SKILL_FILE}" || fail 'missing high-risk write guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Coinbase Advanced Trade"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$coinbase-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $coinbase-openapi-skill'

echo "skills/coinbase-openapi-skill validation passed"
