#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/defillama-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/defillama-public.openapi.json"

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

jq -e '.openapi and .paths' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'invalid OpenAPI schema JSON or missing .openapi/.paths'
jq -e '.paths["/protocols"] and .paths["/protocol/{protocol}"] and .paths["/v2/chains"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected DefiLlama public paths'

rg -q '^name:\s*defillama-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v defillama-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link defillama-openapi-cli https://api.llama.fi --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'defillama-openapi-cli get:/protocols -h' "${SKILL_FILE}" || fail 'missing protocol list help example'
rg -q 'defillama-openapi-cli get:/v2/chains -h' "${SKILL_FILE}" || fail 'missing chain help example'
rg -q 'does not require auth' "${USAGE_FILE}" || fail 'missing no-auth guidance'
rg -q 'split across multiple hosts' "${SKILL_FILE}" || fail 'missing split-host guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"DefiLlama Public API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$defillama-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $defillama-openapi-skill'

echo "skills/defillama-openapi-skill validation passed"
