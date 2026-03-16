#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/chrome-devtools-mcp-skill"
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
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! rg -q '^name:\s*chrome-devtools-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: chrome-devtools-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fixed live Chrome DevTools MCP autoConnect endpoint"
fi

if ! rg -q 'command -v chrome-devtools-mcp-cli' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include default link existence check"
fi

if ! rg -q 'uxc link chrome-devtools-mcp-cli "npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fixed link creation command"
fi

if ! rg -q 'chrome-devtools-mcp-port' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include optional browserUrl link"
fi

if ! rg -q 'chrome-devtools-mcp-isolated' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include optional isolated fallback link"
fi

if ! rg -q -- '--autoConnect' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include autoConnect mode"
fi

if ! rg -q -- '--browserUrl http://127.0.0.1:9222' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include browserUrl attach mode"
fi

if ! rg -q -- '--headless --isolated --no-usage-statistics' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include isolated fallback mode"
fi

if ! rg -q -- '--autoConnect' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include autoConnect mode"
fi

if ! rg -q 'official source' "${SKILL_FILE}"; then
  fail "SKILL.md must mention official-source discovery step"
fi

if ! rg -q 'probe candidate endpoints with' "${SKILL_FILE}"; then
  fail "SKILL.md must include probe step before finalizing endpoint"
fi

if ! rg -q 'no OAuth/API key' "${SKILL_FILE}"; then
  fail "SKILL.md must explicitly document auth detection result"
fi

if ! rg -q 'chrome-devtools-mcp-cli -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include help-first command"
fi

if ! rg -q 'chrome-devtools-mcp-cli new_page -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include operation help for new_page"
fi

if ! rg -q 'new_page url=' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include key=value example"
fi

if ! rg -q "new_page .*'\\{.*\\}'" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include bare JSON positional example"
fi

if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc "npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fallback equivalence guidance"
fi

if rg -q -- 'chrome-devtools-mcp-cli (list|describe|call)(\s|$)|--input-json|--args .*\{' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must not use legacy list/describe/call/--input-json/--args JSON forms"
fi

if ! rg -q '^\s*display_name:\s*"Chrome DevTools MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$chrome-devtools-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $chrome-devtools-mcp-skill'
fi

echo "skills/chrome-devtools-mcp-skill validation passed"
