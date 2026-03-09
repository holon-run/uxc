#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/binance-web3-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
SCHEMA_FILE="${SKILL_DIR}/references/binance-web3.openapi.json"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg
need_cmd jq

required_files=(
  "${SKILL_FILE}"
  "${OPENAI_FILE}"
  "${USAGE_FILE}"
  "${SCHEMA_FILE}"
)

for file in "${required_files[@]}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

rg -q '^name:\s*binance-web3-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v binance-web3-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link binance-web3-openapi-cli https://web3.binance.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'binance-web3-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'binance-web3-openapi-cli get:/bapi/defi/v5/public/wallet-direct/buw/wallet/market/token/search -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'UUID v4 `requestId`' "${SKILL_FILE}" || fail 'missing audit requestId guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Binance Web3 API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$binance-web3-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $binance-web3-openapi-skill'

jq -e '.openapi == "3.1.0"' "${SCHEMA_FILE}" >/dev/null || fail 'schema must be OpenAPI 3.1.0 JSON'
jq -e '.paths["/bapi/defi/v5/public/wallet-direct/buw/wallet/market/token/search"].get' "${SCHEMA_FILE}" >/dev/null || fail 'missing token search path'
jq -e '.paths["/bapi/defi/v1/public/wallet-direct/security/token/audit"].post' "${SCHEMA_FILE}" >/dev/null || fail 'missing token audit path'

echo "skills/binance-web3-openapi-skill validation passed"
