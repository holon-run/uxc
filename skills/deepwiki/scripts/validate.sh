#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/deepwiki"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

# Check dependencies
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

# Validate SKILL frontmatter minimum fields.
# Require the first line to be '---' and a subsequent closing '---'.
if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! rg -q '^name:\s*deepwiki\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: deepwiki"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

# Validate required invocation contract appears in SKILL text.
if ! rg -q 'mcp.deepwiki.com/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must document MCP endpoint"
fi

if ! rg -q 'ask_question' "${SKILL_FILE}"; then
  fail "SKILL.md must document ask_question tool"
fi

if ! rg -q 'command -v deepwiki-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link deepwiki-mcp-cli mcp.deepwiki.com/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must include fixed link creation command"
fi

if ! rg -q 'deepwiki-mcp-cli list' "${SKILL_FILE}"; then
  fail "SKILL.md must use deepwiki-mcp-cli as default invocation path"
fi

if ! rg -q 'ask_question repoName=' "${SKILL_FILE}"; then
  fail "SKILL.md must prefer key=value examples for ask_question"
fi

if ! rg -q "read_wiki_structure .*'\\{.*\\}'" "${SKILL_FILE}"; then
  fail "SKILL.md must include a bare JSON positional example"
fi

if rg -q -- "--args '\\{" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "deepwiki docs must not pass raw JSON via --args"
fi

# Validate references linked from SKILL body.
if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc mcp.deepwiki.com/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "deepwiki docs must include single-point fallback equivalence guidance"
fi

if rg -qi 'retry with .*suffix|append.*suffix|dynamic rename|auto-rename' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "deepwiki docs must not include dynamic command renaming guidance"
fi

# Validate openai.yaml minimum fields.
if ! rg -q '^\s*display_name:\s*"DeepWiki"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$deepwiki.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $deepwiki'
fi

echo "skills/deepwiki validation passed"
