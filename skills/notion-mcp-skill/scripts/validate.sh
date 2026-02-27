#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/notion-mcp-skill"
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
  "${SKILL_DIR}/references/oauth-and-binding.md"
  "${SKILL_DIR}/references/error-handling.md"
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

if ! rg -q '^name:\s*notion-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: notion-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'https://mcp.notion.com/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must document Notion MCP endpoint"
fi

if ! rg -q 'notion-search' "${SKILL_FILE}"; then
  fail "SKILL.md must mention notion-search"
fi

if ! rg -q 'notion-fetch' "${SKILL_FILE}"; then
  fail "SKILL.md must mention notion-fetch"
fi

if ! rg -q 'notion-update-page' "${SKILL_FILE}"; then
  fail "SKILL.md must mention notion-update-page"
fi

for rel in \
  "references/usage-patterns.md" \
  "references/oauth-and-binding.md" \
  "references/error-handling.md"; do
  if ! rg -q "${rel}" "${SKILL_FILE}"; then
    fail "SKILL.md must reference ${rel}"
  fi
done

if ! rg -q '\$uxc' "${SKILL_FILE}"; then
  fail "SKILL.md must explicitly reference the $uxc skill for shared OAuth/error guidance"
fi

if ! rg -q 'canonical OAuth and binding workflow, use `\$uxc` skill' "${SKILL_DIR}/references/oauth-and-binding.md"; then
  fail "oauth-and-binding.md must be a thin wrapper pointing to $uxc guidance"
fi

if ! rg -q 'canonical error taxonomy and OAuth recovery playbooks, use `\$uxc` skill' "${SKILL_DIR}/references/error-handling.md"; then
  fail "error-handling.md must be a thin wrapper pointing to $uxc guidance"
fi

if ! rg -q '^\s*display_name:\s*"Notion MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$notion-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $notion-mcp-skill'
fi

echo "skills/notion-mcp-skill validation passed"
