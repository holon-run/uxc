#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/ethereum-jsonrpc-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

rg -q '^name:\s*ethereum-jsonrpc-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v ethereum-jsonrpc-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link ethereum-jsonrpc-cli https://ethereum-rpc.publicnode.com --schema-url https://raw.githubusercontent.com/ethereum/execution-apis/assembled-spec/refs-openrpc.json' "${SKILL_FILE}" || fail 'missing fixed link create command'
rg -q 'ethereum-jsonrpc-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'ethereum-jsonrpc-cli eth_blockNumber -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'uxc subscribe start wss://<verified-ethereum-rpc-host> eth_subscribe' "${SKILL_FILE}" || fail 'missing subscription example'
rg -q 'uxc subscribe stop <job_id>' "${SKILL_FILE}" || fail 'missing subscribe stop example'
rg -q 'fixed `--schema-url` link' "${SKILL_FILE}" || fail 'missing schema-url discovery note'
rg -q 'eth_sendRawTransaction' "${SKILL_FILE}" || fail 'missing write-method guardrail'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'
rg -q 'validated with your provider' "${USAGE_FILE}" || fail 'missing provider-validated websocket note'

if rg -q -- '(^|[[:space:]])uxc <host> (list|describe|call)([[:space:]]|$)|(^|[[:space:]])ethereum-jsonrpc-cli (list|describe|call)([[:space:]]|$)|--args .*\{' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy patterns'
fi

rg -q '^\s*display_name:\s*"Ethereum JSON-RPC"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$ethereum-jsonrpc-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $ethereum-jsonrpc-skill'

echo "skills/ethereum-jsonrpc-skill validation passed"
