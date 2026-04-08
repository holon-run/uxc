#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/github-openapi-skill"
SKILL_FILE="${SKILL_DIR}/SKILL.md"
OPENAI_FILE="${SKILL_DIR}/agents/openai.yaml"
USAGE_FILE="${SKILL_DIR}/references/usage-patterns.md"

fail() {
  printf '[validate] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

need_cmd rg

for file in "${SKILL_FILE}" "${OPENAI_FILE}" "${USAGE_FILE}"; do
  [[ -f "${file}" ]] || fail "missing required file: ${file}"
done

rg -q '^name:\s*github-openapi-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'
rg -q 'uxc auth credential import github --from gh' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing gh import guidance'
rg -q 'uxc auth binding match https://api.github.com/repos/holon-run/uxc' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing binding verification'
rg -q 'command -v github-openapi-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link github-openapi-cli https://api.github.com --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json' "${SKILL_FILE}" "${USAGE_FILE}" || fail 'missing fixed link command'
rg -q 'github-openapi-cli get:/user -h' "${SKILL_FILE}" || fail 'missing operation help example'
rg -q 'Do not commit GitHub tokens' "${SKILL_FILE}" || fail 'missing token guardrail'

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail 'found banned legacy JSON argument pattern'
fi

rg -q '^\s*display_name:\s*"GitHub REST API"\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$github-openapi-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $github-openapi-skill'

echo "skills/github-openapi-skill validation passed"
