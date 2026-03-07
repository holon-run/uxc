#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/linear-graphql-skill"
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

rg -q '^name:\s*linear-graphql-skill\s*$' "${SKILL_FILE}" || fail 'invalid skill name'
rg -q '^description:\s*.+' "${SKILL_FILE}" || fail 'missing description'

rg -q 'command -v linear-graphql-cli' "${SKILL_FILE}" || fail 'missing link-first command check'
rg -q 'uxc link linear-graphql-cli https://api.linear.app/graphql' "${SKILL_FILE}" || fail 'missing fixed link create command'
rg -q 'linear-graphql-cli -h' "${SKILL_FILE}" || fail 'missing host help example'
rg -q 'linear-graphql-cli query/issues -h' "${SKILL_FILE}" || fail 'missing operation-level help example'

rg -q 'positional JSON' "${SKILL_FILE}" || fail 'missing positional JSON guidance'
rg -q '_select' "${SKILL_FILE}" || fail 'missing _select guidance'
rg -Fq 'Authorization:{{secret}}' "${SKILL_FILE}" || fail 'missing api_key auth header guidance'

if rg -q -- '--input-json' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail 'found banned --input-json pattern'
fi

if rg -q -- "--args\\s+'\\{" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail 'found banned legacy JSON via --args pattern'
fi

rg -q '^\s*display_name:\s*"?Linear GraphQL Agent"?\s*$' "${OPENAI_FILE}" || fail 'missing display_name'
rg -q '^\s*short_description:\s*.+$' "${OPENAI_FILE}" || fail 'missing short_description'
rg -q '^\s*default_prompt:\s*".*\$linear-graphql-skill.*"\s*$' "${OPENAI_FILE}" || fail 'default_prompt must mention $linear-graphql-skill'

echo "skills/linear-graphql-skill validation passed"
