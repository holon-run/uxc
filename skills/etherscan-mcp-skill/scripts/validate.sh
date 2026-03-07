#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/etherscan-mcp-skill"
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

if ! rg -q '^name:\s*etherscan-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: etherscan-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'mcp\.etherscan\.io/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must document Etherscan MCP endpoint"
fi

if ! rg -q 'command -v etherscan-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link etherscan-mcp-cli https://mcp\.etherscan\.io/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must include fixed link creation command"
fi

if ! rg -q 'etherscan-mcp-cli -h' "${SKILL_FILE}"; then
  fail "SKILL.md must use etherscan-mcp-cli help-first discovery"
fi

for op in balance tokenTopHolders getContractAbi transaction; do
  if ! rg -q "${op}" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
    fail "Etherscan docs must include ${op}"
  fi
done

if rg -q -- '--input-json|etherscan-mcp-cli list|etherscan-mcp-cli describe|etherscan-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "Etherscan docs must not use list/describe/call/--input-json in default examples"
fi

if ! rg -q 'references/usage-patterns\.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc https://mcp\.etherscan\.io/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "Etherscan docs must include single-point fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"Etherscan MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$etherscan-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $etherscan-mcp-skill'
fi

echo "skills/etherscan-mcp-skill validation passed"
