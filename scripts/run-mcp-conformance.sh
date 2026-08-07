#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

conformance_version="${MCP_CONFORMANCE_VERSION:-0.2.0-alpha.10}"
selection_args=()

for arg in "$@"; do
  case "$arg" in
    --scenario | --scenario=* | --suite | --suite=*)
      selection_args=("$@")
      break
      ;;
  esac
done

if [[ ${#selection_args[@]} -eq 0 ]]; then
  selection_args=(--suite all "$@")
fi

cargo build --locked --bin uxc

UXC_BIN="$repo_root/target/debug/uxc" \
  npx --yes --package "@modelcontextprotocol/conformance@${conformance_version}" -- conformance client \
    --command "$repo_root/tests/conformance/mcp-client.sh" \
    --spec-version 2026-07-28 \
    --expected-failures "$repo_root/tests/conformance/expected-failures.yml" \
    "${selection_args[@]}"
