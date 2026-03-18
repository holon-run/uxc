#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/near-jsonrpc-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/near-public.openrpc.json"

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

rg -q '^name:\s*near-jsonrpc-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'command -v near-jsonrpc-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link near-jsonrpc-cli https://free.rpc.fastnear.com --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json' "${SKILL_FILE}" || fail 'missing fixed link create command'
rg -q -- '--schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/near-jsonrpc-skill/references/near-public.openrpc.json' "${USAGE_FILE}" || fail 'missing fixed schema-url usage example'
rg -q 'fixed `--schema-url` link' "${SKILL_FILE}" || fail 'missing schema-url discovery note'
rg -q 'near.org' "${SKILL_FILE}" || fail 'missing deprecated endpoint warning'
rg -q 'status' "${SKILL_FILE}" || fail 'missing status method guidance'
rg -q 'gas_price' "${SKILL_FILE}" || fail 'missing gas_price method guidance'
rg -q 'validators' "${SKILL_FILE}" || fail 'missing validators method guidance'
rg -F -q -- "--input-json '{\"block_id\":null}'" "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing gas_price null input guidance'
rg -q 'read-only' "${SKILL_FILE}" || fail 'missing read-only guardrail'

if rg -q -- "--args\\s+'\\{|\\[null\\]|uxc https://free\\.rpc\\.fastnear\\.com <operation>" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

jq -e '.openrpc and (.methods | length >= 6)' "${SCHEMA_FILE}" >/dev/null || fail 'invalid near openrpc schema'
rg -q '"name": "status"' "${SCHEMA_FILE}" || fail 'missing status method in schema'
rg -q '"name": "query"' "${SCHEMA_FILE}" || fail 'missing query method in schema'
rg -q '"name": "gas_price"' "${SCHEMA_FILE}" || fail 'missing gas_price method in schema'

rg -q '^\s*display_name:\s*"NEAR JSON-RPC"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$near-jsonrpc-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $near-jsonrpc-skill'

echo "skills/near-jsonrpc-skill validation passed"
