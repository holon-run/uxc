#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/blocknative-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/blocknative-gas.openapi.json"

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
jq -e '.paths["/chains"] and .paths["/gasprices/blockprices"] and .paths["/gasprices/basefee-estimates"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Blocknative paths'
jq -e '.paths["/gasprices/distribution"].get and .paths["/gasprices/blockprices"].get' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema must expose expected GET operations'

rg -q '^name:\s*blocknative-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v blocknative-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link blocknative-openapi-cli https://api.blocknative.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -F -q 'blocknative-openapi-cli get:/gasprices/basefee-estimates -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--api-key-header Authorization' "${SKILL_FILE}" || fail 'missing api key setup'
rg -q 'uxc auth binding match https://api.blocknative.com' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'once per second' "${SKILL_FILE}" || fail 'missing polling guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Blocknative Gas Platform"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$blocknative-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $blocknative-openapi-skill'

echo "skills/blocknative-openapi-skill validation passed"
