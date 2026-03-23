#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/notion-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/notion-public.openapi.json"

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
jq -e '.paths["/blocks/{block_id}/children"] and .paths["/data_sources/{data_source_id}/query"] and .paths["/databases/{database_id}"] and .paths["/pages"].post and .paths["/pages/{page_id}"].patch and .paths["/blocks/{block_id}"].patch and .paths["/blocks/{block_id}"].delete and .paths["/blocks/{block_id}/children"].patch' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected traversal, write, or compatibility paths'
jq -e '.components.parameters == null or .components.parameters.NotionVersion == null' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'Notion-Version should not remain as an OpenAPI operation parameter in this skill'

rg -q '^name:\s*notion-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v notion-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link notion-openapi-cli https://api.notion.com/v1 --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'notion-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'notion-openapi-cli post:/search -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -Fq 'notion-openapi-cli post:/pages -h' "${SKILL_FILE}" || fail 'missing page-create help example'
rg -Fq 'notion-openapi-cli patch:/blocks/{block_id}/children -h' "${SKILL_FILE}" || fail 'missing block-append help example'
rg -Fq 'Notion-Version=2026-03-11' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing required Notion-Version guidance'
rg -Fq 'Authorization=Bearer {{secret}}' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing explicit Authorization header guidance'
rg -q 'notion-mcp' "${SKILL_FILE}" || fail 'missing shared oauth credential guidance'
rg -q 'read-first' "${SKILL_FILE}" || fail 'missing read-first guardrail'
rg -Fq 'delete:/blocks/{block_id}' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing delete block guidance'
rg -Fq 'patch:/pages/{page_id}' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing update page guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Notion Public API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$notion-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $notion-openapi-skill'

echo "skills/notion-openapi-skill validation passed"
