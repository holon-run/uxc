#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/defillama-prices-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/defillama-prices.openapi.json"

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

jq -e '.openapi and .paths["/prices/current/{coins}"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected DefiLlama price path'
rg -q '^name:\s*defillama-prices-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q 'command -v defillama-prices-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link defillama-prices-openapi-cli https://coins.llama.fi --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command'
rg -q 'defillama-prices-openapi-cli get:/prices/current/\{coins\} -h' "${SKILL_FILE}" || fail 'missing operation help example'
rg -q 'does not require auth' "${USAGE_FILE}" || fail 'missing no-auth guidance'
if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi
rg -q '^\s*display_name:\s*"DefiLlama Prices API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*default_prompt:\s*".*\$defillama-prices-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $defillama-prices-openapi-skill'

echo "skills/defillama-prices-openapi-skill validation passed"
