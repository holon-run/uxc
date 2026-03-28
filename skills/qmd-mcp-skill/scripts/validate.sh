#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/qmd-mcp-skill"
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
)

for file in "${required_files[@]}"; do
  if [[ ! -f "${file}" ]]; then
    fail "missing required file: ${file}"
  fi
done

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! rg -q '^name:\s*qmd-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: qmd-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'command -v qmd-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link --daemon-idle-ttl 0 qmd-mcp-cli "qmd mcp"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must include the default daemon-backed link creation command"
fi

if ! rg -q 'qmd-mcp-cli -h' "${SKILL_FILE}"; then
  fail "SKILL.md must use qmd-mcp-cli help-first discovery"
fi

if ! rg -q 'qmd-mcp-cli query -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must document query help inspection"
fi

if ! rg -q 'qmd-mcp-cli get -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must document get help inspection"
fi

if ! rg -q 'qmd-mcp-cli multi_get -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must document multi_get help inspection"
fi

if ! rg -q 'qmd-mcp-cli status -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must document status help inspection"
fi

if ! rg -q 'uxc daemon status' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must document daemon status checks"
fi

if ! rg -q 'uxc daemon sessions' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must document daemon session checks"
fi

if ! rg -q '"type":"lex"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must include a lex query example"
fi

if ! rg -q '"type":"vec"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must include a vec query example"
fi

if ! rg -q '"type":"hyde"' "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must include a hyde query example in usage patterns"
fi

if ! rg -q 'equivalent to `uxc "qmd mcp"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must include fallback equivalence guidance"
fi

if rg -q -- 'qmd query ".*"|qmd search ".*"|qmd-mcp-cli (list|describe|call)\b|--input-json' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "qmd skill must not rely on legacy list/describe/call or direct qmd query/search as the default workflow"
fi

if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q '^\s*display_name:\s*"QMD MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$qmd-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $qmd-mcp-skill'
fi

echo "skills/qmd-mcp-skill validation passed"
