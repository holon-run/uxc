#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/whatsapp-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/whatsapp-cloud.openapi.json"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg
need_cmd jq

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}" "${SCHEMA_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

jq -e '.openapi and .paths' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'invalid OpenAPI schema JSON or missing .openapi/.paths'
jq -e '.paths["/{phone_number_id}/messages"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /{phone_number_id}/messages path'

rg -q '^name:\s*whatsapp-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v whatsapp-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link whatsapp-openapi-cli https://graph.facebook.com/v25.0 --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'whatsapp-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -F -q 'whatsapp-openapi-cli post:/{phone_number_id}/messages -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--auth-type bearer' "${SKILL_FILE}" || fail 'missing bearer auth setup'
rg -q 'uxc auth binding match https://graph.facebook.com/v25.0' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'v25.0' "${SKILL_FILE}" || fail 'missing API version pin guidance'
rg -q 'high-risk write' "${SKILL_FILE}" || fail 'missing write guardrail'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"WhatsApp Cloud API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$whatsapp-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $whatsapp-openapi-skill'

echo "skills/whatsapp-openapi-skill validation passed"
