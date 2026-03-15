#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/sui-jsonrpc-skill"
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

rg -q '^name:\s*sui-jsonrpc-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v sui-jsonrpc-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link sui-jsonrpc-cli https://fullnode.mainnet.sui.io' "${SKILL_FILE}" || fail 'missing fixed link create command'
rg -q 'sui-jsonrpc-cli -h' "${SKILL_FILE}" || fail 'missing help-first host discovery example'
rg -q 'sui-jsonrpc-cli sui_getLatestCheckpointSequenceNumber -h' "${SKILL_FILE}" || fail 'missing operation-level help example'
rg -q 'uxc subscribe start wss://<verified-sui-rpc-host> suix_subscribeEvent' "${SKILL_FILE}" || fail 'missing event subscription example'
rg -q 'uxc subscribe stop <job_id>' "${SKILL_FILE}" || fail 'missing subscribe stop example'
rg -q 'OpenRPC or `rpc.discover`' "${SKILL_FILE}" || fail 'missing JSON-RPC discovery note'
rg -q 'unsafe_\*' "${SKILL_FILE}" || fail 'missing unsafe method guardrail'
rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'
rg -q 'Use `uxc subscribe start` for pubsub methods' "${SKILL_FILE}" || fail 'missing pubsub execution guidance'
rg -q 'validated with your Sui provider' "${USAGE_FILE}" || fail 'missing provider-verified websocket endpoint note'

if rg -q -- '(^|[[:space:]])uxc <host> (list|describe|call)([[:space:]]|$)|(^|[[:space:]])sui-jsonrpc-cli (list|describe|call)([[:space:]]|$)|--args .*\{' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy patterns'
fi

rg -q '^\s*display_name:\s*"Sui JSON-RPC"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$sui-jsonrpc-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $sui-jsonrpc-skill'

echo "skills/sui-jsonrpc-skill validation passed"
