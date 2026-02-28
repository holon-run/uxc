#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/uxc"
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
  "${SKILL_DIR}/references/protocol-cheatsheet.md"
  "${SKILL_DIR}/references/public-endpoints.md"
  "${SKILL_DIR}/references/oauth-and-binding.md"
  "${SKILL_DIR}/references/error-handling.md"
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

if ! rg -q '^name:\s*uxc\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: uxc"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

# Validate required invocation contract appears in SKILL text.
if ! rg -q 'uxc <host> -h' "${SKILL_FILE}"; then
  fail "SKILL.md must document help-first discovery workflow"
fi

if ! rg -q 'uxc <host> <operation> -h' "${SKILL_FILE}"; then
  fail "SKILL.md must document operation-level help workflow"
fi

if ! rg -q "uxc <host> <operation> key=value" "${SKILL_FILE}"; then
  fail "SKILL.md must document execute workflow"
fi

if ! rg -q "uxc <host> <operation> '<payload-json>'" "${SKILL_FILE}"; then
  fail "SKILL.md must document bare JSON execute workflow"
fi

if rg -q -- 'uxc <host> describe <operation>|uxc <host> call <operation>|--input-json' "${SKILL_FILE}" "${SKILL_DIR}/references/"*.md; then
  fail "uxc skill docs must not use describe/call/--input-json in default examples"
fi

if ! rg -q 'Link-First Workflow For Wrapper Skills' "${SKILL_FILE}"; then
  fail "SKILL.md must include Link-First workflow guidance for wrapper skills"
fi

if ! rg -q 'naming convention: `<provider>-mcp-cli`' "${SKILL_FILE}"; then
  fail "SKILL.md must define fixed wrapper link naming convention"
fi

if ! rg -q 'command -v <link_name>' "${SKILL_FILE}"; then
  fail "SKILL.md must include link existence check pattern"
fi

if ! rg -q '`<link_name> <operation> ...` is equivalent to `uxc <host> <operation> ...`' "${SKILL_FILE}"; then
  fail "SKILL.md must include link/uxc equivalence rule"
fi

if rg -q "execute notion" "${SKILL_FILE}"; then
  fail "SKILL.md must not document execute-form invocations"
fi

if rg -q -- "--args '\\{" "${SKILL_FILE}" "${SKILL_DIR}/references/"*.md; then
  fail "uxc docs must not pass raw JSON via --args"
fi

# Validate references linked from SKILL body.
for rel in \
  "references/usage-patterns.md" \
  "references/protocol-cheatsheet.md" \
  "references/public-endpoints.md" \
  "references/oauth-and-binding.md" \
  "references/error-handling.md"; do
  if ! rg -q "${rel}" "${SKILL_FILE}"; then
    fail "SKILL.md must reference ${rel}"
  fi
done

if ! rg -q -F 'Wrapper Pattern (Link-First)' "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "uxc usage-patterns must include wrapper link-first pattern"
fi

if ! rg -q 'Do not dynamically rename link commands at runtime' "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "uxc usage-patterns must forbid dynamic link renaming at runtime"
fi

# Validate openai.yaml minimum fields.
if ! rg -q '^\s*display_name:\s*"UXC"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$uxc.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $uxc'
fi

echo "skills/uxc validation passed"
