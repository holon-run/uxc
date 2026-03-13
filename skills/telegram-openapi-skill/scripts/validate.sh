#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/telegram-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/telegram-bot.openapi.json"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}" "${SCHEMA_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

rg -q '^name:\s*telegram-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v telegram-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link telegram-openapi-cli https://api.telegram.org --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'telegram-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'telegram-openapi-cli get:/getMe -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q -- '--path-prefix-template "/bot\{\{secret\}\}"' "${SKILL_FILE}" || fail 'missing Telegram path auth setup'
rg -q 'uxc auth binding match https://api.telegram.org' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Telegram Bot API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$telegram-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $telegram-openapi-skill'

echo "skills/telegram-openapi-skill validation passed"
