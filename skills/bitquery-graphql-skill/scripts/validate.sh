#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/bitquery-graphql-skill"
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

if ! head -n 1 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include YAML frontmatter"
fi

if ! tail -n +2 "${SKILL_FILE}" | rg -q '^---$'; then
  fail "SKILL.md must include closing YAML frontmatter"
fi

if ! rg -q '^name:\s*bitquery-graphql-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: bitquery-graphql-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'https://streaming.bitquery.io/graphql' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must document the GraphQL endpoint"
fi

if ! rg -q 'https://oauth2.bitquery.io/oauth2/token' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must document the OAuth token endpoint"
fi

if ! rg -q 'client_credentials' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must document client_credentials auth"
fi

if ! rg -q 'command -v bitquery-graphql-cli' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must include link-first command checks"
fi

if ! rg -q 'uxc link bitquery-graphql-cli https://streaming.bitquery.io/graphql' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must include fixed link creation"
fi

if ! rg -q 'bitquery-graphql-cli -h' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must use help-first discovery"
fi

for op in 'query/EVM' 'query/Trading'; do
  if ! rg -q "${op}" "${SKILL_FILE}" "${USAGE_FILE}"; then
    fail "Bitquery docs must reference ${op}"
  fi
done

if ! rg -q '_select' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must explain _select usage"
fi

if ! rg -q "query/EVM '.*_select" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must include a positional JSON query example"
fi

if ! rg -q 'subscription/\*' "${SKILL_FILE}"; then
  fail "SKILL.md must mention subscription caution"
fi

if ! rg -q 'references/usage-patterns.md' "${SKILL_FILE}"; then
  fail "SKILL.md must reference usage-patterns.md"
fi

if ! rg -q 'equivalent to `uxc https://streaming.bitquery.io/graphql' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must include fallback equivalence guidance"
fi

if rg -q -- '--input-json|(^|[[:space:]])bitquery-graphql-cli (list|describe|call)([[:space:]]|$)' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must not use list/describe/call/--input-json in default examples"
fi

if rg -q -- "--args '\\{" "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must not pass raw JSON via --args"
fi

if rg -qi 'retry with .*suffix|append.*suffix|dynamic rename|auto-rename' "${SKILL_FILE}" "${USAGE_FILE}"; then
  fail "Bitquery docs must not include dynamic command renaming guidance"
fi

if ! rg -q '^\s*display_name:\s*"Bitquery GraphQL"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$bitquery-graphql-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $bitquery-graphql-skill'
fi

echo "skills/bitquery-graphql-skill validation passed"
