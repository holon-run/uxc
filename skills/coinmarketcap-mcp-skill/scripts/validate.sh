#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SKILL_DIR="${ROOT_DIR}/skills/coinmarketcap-mcp-skill"
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
  "${SKILL_DIR}/scripts/validate.sh"
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

if ! rg -q '^name:\s*coinmarketcap-mcp-skill\s*$' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define: name: coinmarketcap-mcp-skill"
fi

if ! rg -q '^description:\s*.+' "${SKILL_FILE}"; then
  fail "SKILL.md frontmatter must define a description"
fi

if ! rg -q 'mcp\.coinmarketcap\.com/mcp' "${SKILL_FILE}"; then
  fail "SKILL.md must document CoinMarketCap MCP endpoint"
fi

if ! rg -q 'X-CMC-MCP-API-KEY' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "CoinMarketCap docs must configure X-CMC-MCP-API-KEY auth"
fi

if ! rg -q 'command -v coinmarketcap-mcp-cli' "${SKILL_FILE}"; then
  fail "SKILL.md must include link command existence check"
fi

if ! rg -q 'uxc link coinmarketcap-mcp-cli https://mcp\.coinmarketcap\.com/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must include fixed link creation command"
fi

if ! rg -q 'coinmarketcap-mcp-cli -h' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "docs must use coinmarketcap-mcp-cli help-first discovery"
fi

for op in get_crypto_quotes_latest get_global_metrics_latest trending_crypto_narratives get_crypto_latest_news; do
  if ! rg -q "${op}" "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
    fail "CoinMarketCap docs must include ${op}"
  fi
done

if ! rg -q 'search_cryptos|search_crypto_info' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "CoinMarketCap docs must include search tool coverage"
fi

if rg -q -- '--input-json|coinmarketcap-mcp-cli list|coinmarketcap-mcp-cli describe|coinmarketcap-mcp-cli call' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "CoinMarketCap docs must not use list/describe/call/--input-json in default examples"
fi

if ! rg -q 'x402' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "CoinMarketCap docs must explain current x402 scope boundary"
fi

if ! rg -q 'equivalent to `uxc https://mcp\.coinmarketcap\.com/mcp' "${SKILL_FILE}" "${SKILL_DIR}/references/usage-patterns.md"; then
  fail "CoinMarketCap docs must include single-point fallback equivalence guidance"
fi

if ! rg -q '^\s*display_name:\s*"CoinMarketCap MCP"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.display_name"
fi

if ! rg -q '^\s*short_description:\s*".+"\s*$' "${OPENAI_FILE}"; then
  fail "agents/openai.yaml must define interface.short_description"
fi

if ! rg -q '^\s*default_prompt:\s*".*\$coinmarketcap-mcp-skill.*"\s*$' "${OPENAI_FILE}"; then
  fail 'agents/openai.yaml default_prompt must mention $coinmarketcap-mcp-skill'
fi

echo "skills/coinmarketcap-mcp-skill validation passed"
