#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/thegraph-mcp-skill"
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

if ! rg -q '^name:\s*thegraph-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: thegraph-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'subgraphs\.mcp\.thegraph\.com/sse' "${SKILL_FILE}"; then
  fail "SKILL.md must document The Graph MCP endpoint"
fi

if ! rg -q 'command -v thegraph-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link thegraph-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include fixed link creation command"
fi

if ! rg -q 'uxc auth credential set thegraph --secret-env THEGRAPH_API_KEY' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must register a credential for The Graph auth"
fi

if ! rg -q 'uxc auth binding add --id thegraph-sse --host subgraphs\.mcp\.thegraph\.com --path-prefix /sse --scheme https --credential thegraph --priority 100' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include auth binding setup for native The Graph SSE endpoint"
fi

if rg -q 'mcp-remote|inject-env|\$\{THEGRAPH_API_KEY\}' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must not rely on old mcp-remote/env-injection guidance"
fi

if ! rg -q 'thegraph-mcp-cli -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must use thegraph-mcp-cli help-first discovery"
fi

if ! rg -q 'THEGRAPH_API_KEY' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include THEGRAPH_API_KEY auth guidance"
fi

if ! rg -q 'official source' "${SKILL_FILE}"; then
  fail "SKILL.md must mention official-source discovery step"
fi

if ! rg -q 'probe candidate endpoint' "${SKILL_FILE}"; then
  fail "SKILL.md must include probe step before finalizing endpoint"
fi

if ! rg -q 'requires a The Graph Gateway API key' "${SKILL_FILE}"; then
  fail "SKILL.md must explicitly document auth detection result"
fi

if rg -q -- 'thegraph-mcp-cli (list|describe|call)|--input-json|--args .*\{' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must not use legacy list/describe/call/--input-json/--args JSON forms"
fi

if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc https://subgraphs\.mcp\.thegraph\.com/sse' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"The Graph MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$thegraph-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $thegraph-mcp-skill'
fi

echo "skills/thegraph-mcp-skill validation passed"
