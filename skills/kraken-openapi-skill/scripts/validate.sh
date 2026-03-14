#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/kraken-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/kraken-public.openapi.json"

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
jq -e '.paths["/0/public/Time"] and .paths["/0/public/Ticker"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing expected Kraken paths'

rg -q '^name:\s*kraken-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q 'command -v kraken-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link kraken-openapi-cli https://api.kraken.com --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'provider-specific header signing' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing private auth boundary note'
rg -q 'read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'
rg -q '^\s*display_name:\s*"Kraken REST"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*default_prompt:\s*".*\$kraken-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $kraken-openapi-skill'

echo "skills/kraken-openapi-skill validation passed"
