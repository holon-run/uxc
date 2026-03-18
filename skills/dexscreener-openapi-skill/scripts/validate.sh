#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/dexscreener-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/dexscreener-public.openapi.json"

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
jq -e '.paths["/token-profiles/latest/v1"] and .paths["/latest/dex/search"] and .paths["/tokens/v1/{chainId}/{tokenAddresses}"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected DexScreener paths'
jq -e '.paths["/latest/dex/pairs/{chainId}/{pairId}"].get and .paths["/token-boosts/top/v1"].get' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema must expose expected GET operations'

rg -q '^name:\s*dexscreener-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v dexscreener-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link dexscreener-openapi-cli https://api.dexscreener.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -F -q 'dexscreener-openapi-cli get:/latest/dex/search -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'no-auth setup' "${OPENAI_FILE}" || fail 'missing no-auth prompt guidance'
rg -q 'read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"DexScreener API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$dexscreener-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $dexscreener-openapi-skill'

echo "skills/dexscreener-openapi-skill validation passed"
