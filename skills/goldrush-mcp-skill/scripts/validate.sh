#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/goldrush-mcp-skill"
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

if ! rg -q '^name:\s*goldrush-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: goldrush-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q '@covalenthq/goldrush-mcp-server@latest' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must reference the GoldRush MCP package endpoint"
fi

if ! rg -q 'GOLDRUSH_API_KEY' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must configure GOLDRUSH_API_KEY injection"
fi

if ! rg -q 'command -v goldrush-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link goldrush-mcp-cli "npx -y @covalenthq/goldrush-mcp-server@latest" --credential goldrush-mcp --inject-env GOLDRUSH_API_KEY=\{\{secret\}\}' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fixed link creation command with env injection"
fi

if ! rg -q 'goldrush-mcp-cli -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must use goldrush-mcp-cli help-first discovery"
fi

for op in getAllChains multichain_balances transactions_for_address historical_portfolio_value; do
  if ! rg -q "${op}" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
    fail "GoldRush docs must include ${op}"
  fi
done

if ! rg -q 'config://supported-chains|status://all-chains' "${SKILL_FILE}"; then
  fail "GoldRush docs must mention MCP resources"
fi

if rg -q -- '--input-json|goldrush-mcp-cli list|goldrush-mcp-cli describe|goldrush-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "GoldRush docs must not use list/describe/call/--input-json in default examples"
fi

if ! rg -q 'equivalent to `uxc --auth goldrush-mcp --inject-env GOLDRUSH_API_KEY=\{\{secret\}\} "npx -y @covalenthq/goldrush-mcp-server@latest"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "GoldRush docs must include single-point fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"GoldRush MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$goldrush-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $goldrush-mcp-skill'
fi

echo "skills/goldrush-mcp-skill validation passed"
