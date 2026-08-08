#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <server-url>" >&2
  exit 2
fi

server_url="${!#}"
scenario="${MCP_CONFORMANCE_SCENARIO:-}"
context="${MCP_CONFORMANCE_CONTEXT:-}"
if [[ -z "$context" ]]; then
  context='{}'
fi
uxc_bin="${UXC_BIN:-$PWD/target/debug/uxc}"

run_uxc() {
  "$uxc_bin" --no-cache --timeout-ms 15000 "$server_url" "$@" </dev/null >/dev/null
}

case "$scenario" in
  tools_call)
    run_uxc add_numbers --input-json '{"a":2,"b":3}'
    ;;
  request-metadata | json-schema-ref-no-deref)
    run_uxc -h
    ;;
  http-invalid-tool-headers)
    run_uxc valid_tool --input-json '{}'
    ;;
  http-standard-headers)
    run_uxc test_headers --input-json '{}'
    ;;
  http-custom-headers)
    failed=0
    while IFS=$'\t' read -r name arguments; do
      if ! run_uxc "$name" --input-json "$arguments"; then
        echo "UXC call returned an error for conformance tool: $name" >&2
        failed=1
      fi
    done < <(
      python3 - "$context" <<'PY'
import json
import sys

context = json.loads(sys.argv[1])
for call in context.get("toolCalls", []):
    arguments = {
        key: value
        for key, value in call.get("arguments", {}).items()
        if value is not None
    }
    print(f"{call['name']}\t{json.dumps(arguments, separators=(',', ':'))}")
PY
    )
    exit "$failed"
    ;;
  *)
    echo "UXC conformance harness does not implement scenario: $scenario" >&2
    exit 2
    ;;
esac
