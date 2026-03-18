#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/helius-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/helius-wallet.openapi.json"

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
jq -e '.paths["/v1/wallet/{wallet}/identity"] and .paths["/v1/wallet/batch-identity"] and .paths["/v1/wallet/{wallet}/balances"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Helius wallet paths'
jq -e '.paths["/v1/wallet/batch-identity"].post and .paths["/v1/wallet/{wallet}/history"].get and .paths["/v1/wallet/{wallet}/funded-by"].get' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema must expose expected GET/POST operations'

rg -q '^name:\s*helius-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v helius-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link helius-openapi-cli https://api.helius.xyz' "${SKILL_FILE}" || fail 'missing fixed link create command'
rg -q -- '--schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/helius-openapi-skill/references/helius-wallet.openapi.json' "${SKILL_FILE}" || fail 'missing schema-url guidance'
rg -F -q 'helius-openapi-cli post:/v1/wallet/batch-identity -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--api-key-header X-Api-Key' "${SKILL_FILE}" || fail 'missing api key setup'
rg -q 'uxc auth binding match https://api.helius.xyz' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'beta' "${SKILL_FILE}" || fail 'missing beta surface guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Helius Wallet API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$helius-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $helius-openapi-skill'

echo "skills/helius-openapi-skill validation passed"
