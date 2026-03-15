#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/okx-exchange-websocket-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

command -v rg >/dev/null 2>&1 || fail "required command not found: rg"

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}" "${SKILL_DIR}/scripts/validate.sh"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

rg -q '^name:\s*okx-exchange-websocket-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'wss://ws\.okx\.com:8443/ws/v5/public' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing OKX public websocket endpoint'
rg -q 'uxc subscribe start' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing subscribe start command'
rg -q -- '--transport websocket' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing raw websocket transport guidance'
rg -q -- '--init-frame' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing init frame guidance'
rg -q '"channel":"tickers"' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing ticker subscribe example'

if rg -q -- '--input-json' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy or misleading command pattern'
fi

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"OKX Exchange WebSocket"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$okx-exchange-websocket-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $okx-exchange-websocket-skill'

echo "skills/okx-exchange-websocket-skill validation passed"
