#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/feishu-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"
SCHEMA_FILE="${SKILL_DIR}/references/feishu-im.openapi.json"
OVERLAY_FILE="${SKILL_DIR}/references/feishu-openapi.overlay.json"
GENERATOR_FILE="${SKILL_DIR}/scripts/generate_openapi.py"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg
need_cmd jq

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}" "${SCHEMA_FILE}" "${OVERLAY_FILE}" "${GENERATOR_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

jq -e '.openapi and .paths' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'invalid OpenAPI schema JSON or missing .openapi/.paths'
jq -e '.security[]? | has("FeishuBearerAuth")' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing FeishuBearerAuth security requirement'
jq -e '.paths["/bot/v3/info"].get' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /bot/v3/info path'
jq -e '.paths["/im/v1/chats"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /im/v1/chats path'
jq -e '.paths["/im/v1/images"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /im/v1/images path'
jq -e '.paths["/im/v1/files"]' "${SCHEMA_FILE}" >/dev/null 2>&1 || fail 'OpenAPI schema missing /im/v1/files path'
jq -e '.paths["/bot/v3/info"].get and .components.schemas.BotInfo' "${OVERLAY_FILE}" >/dev/null 2>&1 || fail 'overlay missing bot identity operation'

rg -q '^name:\s*feishu-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v feishu-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link feishu-openapi-cli https://open.feishu.cn/open-apis --schema-url ' "${SKILL_FILE}" || fail 'missing fixed link create command with schema-url'
rg -q 'feishu-openapi-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'feishu-openapi-cli get:/bot/v3/info -h' "${SKILL_FILE}" || fail 'missing bot identity help example'
rg -q 'feishu-openapi-cli get:/im/v1/chats -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'feishu-openapi-cli post:/im/v1/images -h' "${SKILL_FILE}" || fail 'missing image upload help example'
rg -q 'feishu-openapi-cli post:/im/v1/files -h' "${SKILL_FILE}" || fail 'missing file upload help example'
rg -q -- '--auth-type bearer' "${SKILL_FILE}" || fail 'missing bearer auth setup'
rg -q 'tenant_access_token' "${SKILL_FILE}" || fail 'missing tenant access token guidance'
rg -q 'uxc auth binding match https://open.feishu.cn/open-apis' "${SKILL_FILE}" || fail 'missing binding match check'
rg -q 'receive_id_type' "${SKILL_FILE}" || fail 'missing receive_id_type guardrail'
rg -q 'content` field is a JSON-encoded string' "${SKILL_FILE}" || fail 'missing content-string guardrail'
rg -q 'multipart/form-data' "${SKILL_FILE}" || fail 'missing multipart upload guardrail'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"Feishu / Lark IM"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$feishu-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $feishu-openapi-skill'

echo "skills/feishu-openapi-skill validation passed"
