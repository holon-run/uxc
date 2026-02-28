#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/uxc-skill-creator"
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
  "${SKILL_DIR}/references/workflow.md"
  "${SKILL_DIR}/references/templates.md"
  "${SKILL_DIR}/references/validation-rules.md"
  "${SKILL_DIR}/references/anti-patterns.md"
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

if ! rg -q '^name:\s*uxc-skill-creator\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: uxc-skill-creator"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'link-first' "${SKILL_FILE}"; then
  fail "SKILL.md must document link-first convention"
fi

if ! rg -q 'help-first' "${SKILL_FILE}"; then
  fail "SKILL.md must document help-first convention"
fi

if ! rg -q '<provider>-<protocol>-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must document protocol-aware link naming convention"
fi

if ! rg -q 'Start from user-provided host input' "${SKILL_FILE}"; then
  fail "SKILL.md must document host-driven workflow intake"
fi

if ! rg -q 'search official docs/repo' "${SKILL_FILE}"; then
  fail "SKILL.md must document external search for protocol/path/auth discovery"
fi

if ! rg -q 'probe candidates with `uxc <endpoint> -h`' "${SKILL_FILE}"; then
  fail "SKILL.md must document uxc probe-based endpoint validation"
fi

if ! rg -q 'uxc auth binding match <endpoint>' "${SKILL_FILE}"; then
  fail "SKILL.md must document local binding verification for oauth/binding flows"
fi

if ! rg -q 'references/workflow.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference workflow.md"
fi

if ! rg -q 'references/templates.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference templates.md"
fi

if ! rg -q 'references/validation-rules.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference validation-rules.md"
fi

if ! rg -q 'references/anti-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference anti-patterns.md"
fi

if rg -q -- '(^|[[:space:]])uxc <host> (list|describe|call)([[:space:]]|$)|(^|[[:space:]])<link_name> (list|describe|call)([[:space:]]|$)' "${SKILL_FILE}" "${SKILL_DIR}/references/"*.md; then
  fail "uxc-skill-creator docs must not use deprecated list/describe/call invocation forms"
fi

if ! rg -q '^\s*display_name:\s*"UXC Skill Creator"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$uxc-skill-creator.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $uxc-skill-creator'
fi

echo "skills/uxc-skill-creator validation passed"
