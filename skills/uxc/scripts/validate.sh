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
  "${SKILL_DIR}/references/error-handling.md"
)

for file in "${required_files[@]}"; do
  if [[ ! -f "${file}" ]]; then
    echo "missing required file: ${file}"
    exit 1
  fi
done

# Validate SKILL frontmatter minimum fields.
if ! rg -q '^---$' "${SKILL_FILE}"; then
  echo "SKILL.md must include YAML frontmatter"
  exit 1
fi

if ! rg -q '^name:\s*uxc\s*$' "${SKILL_FILE}"; then
  echo "SKILL.md frontmatter must define: name: uxc"
  exit 1
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  echo "SKILL.md frontmatter must define a description"
  exit 1
fi

# Validate required invocation contract appears in SKILL text.
if ! rg -q 'uxc <host> list' "${SKILL_FILE}"; then
  echo "SKILL.md must document list workflow"
  exit 1
fi

if ! rg -q 'uxc <host> describe <operation>' "${SKILL_FILE}"; then
  echo "SKILL.md must document describe workflow"
  exit 1
fi

if ! rg -q "uxc <host> <operation> --json '<payload-json>'" "${SKILL_FILE}"; then
  echo "SKILL.md must document execute workflow"
  exit 1
fi

# Validate references linked from SKILL body.
for rel in \
  "references/usage-patterns.md" \
  "references/protocol-cheatsheet.md" \
  "references/public-endpoints.md" \
  "references/error-handling.md"; do
  if ! rg -q "${rel}" "${SKILL_FILE}"; then
    echo "SKILL.md must reference ${rel}"
    exit 1
  fi
done

# Validate openai.yaml minimum fields.
if ! rg -q '^\s*display_name:\s*"UXC"\s*$' "${OPENAI_FILE}"; then
  echo "agents/openai.yaml must define interface.display_name"
  exit 1
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  echo "agents/openai.yaml must define interface.short_description"
  exit 1
fi

if ! rg -q '^\s*default_prompt:\s*".*\$uxc.*"\s*$' "${OPENAI_FILE}"; then
  echo 'agents/openai.yaml default_prompt must mention $uxc'
  exit 1
fi

echo "skills/uxc validation passed"
