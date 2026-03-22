#!/usr/bin/env bash

set -euo pipefail

EXPECTED_TAG=""
ALLOW_DIRTY=false

usage() {
  cat <<'USAGE'
Run pre-release checks.

Usage:
  release-check.sh [vX.Y.Z|refs/tags/vX.Y.Z] [--allow-dirty]
USAGE
}

fail() {
  printf '[release-check] error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

main() {
  for arg in "$@"; do
    case "${arg}" in
      -h|--help)
        usage
        exit 0
        ;;
      --allow-dirty)
        ALLOW_DIRTY=true
        ;;
      -*)
        fail "unknown option: ${arg}"
        ;;
      *)
        if [[ -n "${EXPECTED_TAG}" ]]; then
          fail "unexpected extra positional argument: ${arg}"
        fi
        EXPECTED_TAG="${arg}"
        ;;
    esac
  done

  need_cmd cargo
  need_cmd git
  need_cmd npm

  if ! git diff --quiet || ! git diff --cached --quiet; then
    if [[ "${ALLOW_DIRTY}" == "true" ]]; then
      printf '[release-check] warn: working tree is not clean (allowed by --allow-dirty)\n' >&2
    else
      fail "working tree is not clean"
    fi
  fi

  local version
  version="$(awk -F\" '
    /^\[package\]/ { in_pkg = 1; next }
    /^\[/ && !/^\[package\]/ { if (in_pkg) exit }
    in_pkg && /^version[[:space:]]*=/ { print $2; exit }
  ' Cargo.toml)"
  [[ -n "${version}" ]] || fail "failed to read version from Cargo.toml"

  local npm_version
  npm_version="$(node -p "require('./packages/uxc-daemon-client/package.json').version")"
  [[ "${npm_version}" == "${version}" ]] \
    || fail "npm package version ${npm_version} does not match Cargo.toml version ${version}"

  if [[ -n "${EXPECTED_TAG}" ]]; then
    local normalized
    normalized="${EXPECTED_TAG#refs/tags/}"
    [[ "${normalized}" == "v${version}" ]] || fail "tag ${normalized} does not match Cargo.toml version v${version}"
  fi

  grep -Eq "^## \\[${version}\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md \
    || fail "CHANGELOG.md does not contain dated release section for ${version}"

  cargo clippy -- -D warnings -A non_camel_case_types -A unused_variables -A unused_imports -A dead_code -A clippy::upper_case_acronyms -A clippy::enum_variant_names -A clippy::vec_init_then_push -A clippy::type_complexity
  cargo test --locked -- --test-threads=1
  cargo build --bin uxc
  npm ci
  npm run build:ts
  UXC_BIN="${PWD}/target/debug/uxc" npm run test:ts

  printf '[release-check] OK: version=%s\n' "${version}"
}

main "$@"
