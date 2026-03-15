#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/binance-spot-websocket-skill"
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

rg -q '^name:\s*binance-spot-websocket-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'wss://stream\.binance\.com:443' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing Binance stream endpoint'
rg -q 'uxc subscribe start' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing subscribe start command'
rg -q -- '--transport websocket' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing raw websocket transport guidance'
rg -q 'btcusdt@trade' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing validated trade stream example'
rg -q 'lowercase' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing lowercase stream guardrail'

if rg -q -- '--input-json' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy or misleading command pattern'
fi

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Binance Spot WebSocket"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$binance-spot-websocket-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $binance-spot-websocket-skill'

echo "skills/binance-spot-websocket-skill validation passed"
