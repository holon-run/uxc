#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/nodit-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/nodit-web3.openapi.json"

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
jq -e '.paths["/v1/multichain/lookupEntities"] and .paths["/v1/{chain}/{network}/native/getNativeBalanceByAccount"] and .paths["/v1/{chain}/{network}/blockchain/getTransactionsByAccount"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Nodit paths'
jq -e '.paths["/v1/{chain}/{network}/token/getTokenPricesByContracts"].post and .paths["/v1/{chain}/{network}/token/getTokenContractMetadataByContracts"].post' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema must expose expected POST operations'

rg -q '^name:\s*nodit-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v nodit-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link nodit-openapi-cli https://web3.nodit.io --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -F -q 'nodit-openapi-cli post:/v1/multichain/lookupEntities -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--api-key-header X-API-KEY' "${SKILL_FILE}" || fail 'missing api key setup'
rg -q 'overlaps with `Chainbase`, `Alchemy`, and `Moralis`' "${SKILL_FILE}" || fail 'missing overlap guardrail'
rg -q 'HTTP 429 TOO_MANY_REQUESTS' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing rate-limit guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Nodit Web3 Data API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$nodit-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $nodit-openapi-skill'

echo "skills/nodit-openapi-skill validation passed"
