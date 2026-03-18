#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/mempool-space-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/mempool-space-public.openapi.json"

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
jq -e '.paths["/v1/fees/recommended"] and .paths["/mempool"] and .paths["/blocks/tip/height"] and .paths["/address/{address}"] and .paths["/tx/{txid}/status"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Bitcoin paths'
jq -e '.paths["/v1/lightning/statistics/latest"] and .paths["/v1/lightning/search"] and .paths["/v1/lightning/nodes/{public_key}"] and .paths["/v1/lightning/channels"] and .paths["/v1/lightning/channels/{short_id}"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Lightning paths'

rg -q '^name:\s*mempool-space-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v mempool-space-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link mempool-space-openapi-cli https://mempool.space/api --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -F -q 'mempool-space-openapi-cli get:/v1/fees/recommended -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'Treat this v1 skill as read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'
rg -q 'accepts the channel `id` string even though the route label says `short_id`' "${SKILL_FILE}" || fail 'missing channel id guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"mempool.space Public API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$mempool-space-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $mempool-space-openapi-skill'

echo "skills/mempool-space-openapi-skill validation passed"
