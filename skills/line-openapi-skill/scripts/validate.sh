#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/line-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/line-messaging.openapi.json"

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
jq -e '.paths["/v2/bot/info"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /v2/bot/info path'

rg -q '^name:\s*line-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v line-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link line-openapi-cli https://api.line.me --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'line-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'line-openapi-cli get:/v2/bot/info -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--auth-type bearer' "${SKILL_FILE}" || fail 'missing bearer auth setup'
rg -q 'uxc auth binding match https://api.line.me' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'replyToken' "${SKILL_FILE}" || fail 'missing reply token guardrail'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"LINE Messaging API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$line-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $line-openapi-skill'

echo "skills/line-openapi-skill validation passed"
