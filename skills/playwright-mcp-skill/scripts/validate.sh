#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/playwright-mcp-skill"
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

if ! rg -q '^name:\s*playwright-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: playwright-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'npx -y @playwright/mcp@latest --headless --isolated' "${SKILL_FILE}"; then
  fail "SKILL.md must document fixed Playwright MCP stdio endpoint"
fi

if ! rg -q 'command -v playwright-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link playwright-mcp-cli "npx -y @playwright/mcp@latest --headless --isolated"' "${SKILL_FILE}"; then
  fail "SKILL.md must include fixed link creation command"
fi

if ! rg -q 'playwright-mcp-cli -h' "${SKILL_FILE}"; then
  fail "SKILL.md must use playwright-mcp-cli help-first discovery"
fi

if ! rg -q 'playwright-mcp-cli browser_navigate -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include operation-level help via playwright-mcp-cli <operation> -h"
fi

if ! rg -q 'browser_navigate url=' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include key=value example"
fi

if ! rg -q "browser_navigate .*'\\{.*\\}'" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include bare JSON positional example"
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

if rg -q -- 'playwright-mcp-cli (list|describe|call)|--input-json|--args .*\{' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must not use legacy list/describe/call/--input-json/--args JSON forms"
fi

if rg -qi 'retry with .*suffix|append.*suffix|dynamic rename|auto-rename' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must not include dynamic command renaming guidance"
fi

if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc "npx -y @playwright/mcp@latest --headless --isolated"' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"Playwright MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$playwright-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $playwright-mcp-skill'
fi

echo "skills/playwright-mcp-skill validation passed"
