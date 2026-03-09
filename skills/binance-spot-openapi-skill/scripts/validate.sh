#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/binance-spot-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
SCHEMA_FILE="${SKILL_DIR}/references/binance-spot.openapi.json"
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

rg -q '^name:\s*binance-spot-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v binance-spot-mainnet-openapi-cli' "${SKILL_FILE}" || fail 'missing mainnet link-first check'
rg -q 'command -v binance-spot-testnet-openapi-cli' "${SKILL_FILE}" || fail 'missing testnet link-first check'
rg -q 'uxc link binance-spot-mainnet-openapi-cli https://api.binance.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed mainnet link command'
rg -q 'uxc link binance-spot-testnet-openapi-cli https://testnet.binance.vision --schema-url ' "${SKILL_FILE}" || fail 'missing fixed testnet link command'
rg -q 'post:/api/v3/order/test -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'Treat all mainnet write operations as high-risk' "${SKILL_FILE}" || fail 'missing mainnet write guardrail'
rg -q 'timestamp` and `signature` are injected by the signer binding' "${SKILL_FILE}" || fail 'missing signer injection note'
rg -q -- '--field api_key=env:BINANCE_MAINNET_API_KEY' "${SKILL_FILE}" || fail 'missing mainnet auth example'
rg -q -- '--signer-json' "${SKILL_FILE}" || fail 'missing signer-json example'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Binance Spot API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$binance-spot-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $binance-spot-openapi-skill'

jq -e '.openapi == "3.1.0"' "${SCHEMA_FILE}" >/dev/null || fail 'schema must be OpenAPI 3.1.0 JSON'
jq -e '.paths["/api/v3/order"].post.operationId == "createOrder"' "${SCHEMA_FILE}" >/dev/null || fail 'missing create order operation'
jq -e '.paths["/api/v3/account"].get.operationId == "getAccount"' "${SCHEMA_FILE}" >/dev/null || fail 'missing account operation'
jq -e '.paths["/api/v3/order/test"].post.operationId == "testOrder"' "${SCHEMA_FILE}" >/dev/null || fail 'missing test order operation'

echo "skills/binance-spot-openapi-skill validation passed"
