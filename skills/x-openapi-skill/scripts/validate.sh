#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/x-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

rg -q '^name:\s*x-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v x-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link x-openapi-cli https://api.x.com --schema-url https://api.x.com/2/openapi.json' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'x-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'x-openapi-cli get:/2/users/me -h' "${SKILL_FILE}" || fail 'missing operation-level help example'

rg -q 'uxc auth oauth start x-api-user' "${SKILL_FILE}" || fail 'missing oauth start flow'
rg -q -- '--client-id ' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing oauth client-id guidance'
rg -q 'http://127.0.0.1:11111/callback' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing redirect-uri guidance'
rg -q 'uxc auth binding match https://api.x.com/2' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing binding match check'
rg -q 'Do not embed bearer tokens' "${SKILL_FILE}" || fail 'missing no-embedded-token guardrail'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"X API v2"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$x-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $x-openapi-skill'

echo "skills/x-openapi-skill validation passed"
