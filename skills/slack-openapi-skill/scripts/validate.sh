#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/slack-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/slack-web.openapi.json"

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

# Sanity-check the Slack Web API OpenAPI schema
jq -e '.openapi and .paths' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'invalid OpenAPI schema JSON or missing .openapi/.paths'
jq -e '.paths["/auth.test"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /auth.test path'

rg -q '^name:\s*slack-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v slack-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link slack-openapi-cli https://slack.com/api --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'slack-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'slack-openapi-cli get:/auth.test -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--auth-type bearer' "${SKILL_FILE}" || fail 'missing bearer auth setup'
rg -q 'uxc auth binding match https://slack.com/api' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'
rg -q 'May 29, 2025' "${SKILL_FILE}" || fail 'missing rate limit guardrail date'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Slack Web API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$slack-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $slack-openapi-skill'

echo "skills/slack-openapi-skill validation passed"
