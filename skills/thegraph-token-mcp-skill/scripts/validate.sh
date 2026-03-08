#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/thegraph-token-mcp-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

required_files=(
  "${SKILL_FILE}"
  "${OPENAI_FILE}"
  "${SKILL_DIR}/references/usage-patterns.md"
  "${SKILL_DIR}/scripts/validate.sh"
)

for file in "${required_files[@]}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! rg -q '^name:\s*thegraph-token-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: thegraph-token-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'token-api\.mcp\.thegraph\.com/' "${SKILL_FILE}"; then
  fail "SKILL.md must document The Graph Token MCP endpoint"
fi

if ! rg -q 'command -v thegraph-token-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link thegraph-token-mcp-cli https://token-api\.mcp\.thegraph\.com/' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fixed link creation command"
fi

if ! rg -q 'uxc auth credential set thegraph-token --secret-env THEGRAPH_TOKEN_API_JWT' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must configure a dedicated The Graph Token API JWT credential"
fi

if ! rg -q 'uxc auth binding add --id thegraph-token-mcp --host token-api\.mcp\.thegraph\.com --scheme https --credential thegraph-token --priority 100' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include auth binding setup for The Graph Token MCP endpoint"
fi

if ! rg -q 'API TOKEN \(JWT\)' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must explicitly require API TOKEN (JWT)"
fi

if ! rg -q 'thegraph\.market/dashboard' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must reference The Graph Market dashboard for Token API key management"
fi

for op in getV1Networks getV1EvmTokens getV1EvmBalances; do
  if ! rg -q "${op}" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
    fail "docs must include ${op}"
  fi
done

if rg -q -- 'thegraph-token-mcp-cli (list|describe|call)|--input-json|--args .*\{' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must not use legacy list/describe/call/--input-json/--args JSON forms"
fi

if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc https://token-api\.mcp\.thegraph\.com/' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"The Graph Token MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$thegraph-token-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $thegraph-token-mcp-skill'
fi

echo "skills/thegraph-token-mcp-skill validation passed"
