# MCP Conformance Maintenance

UXC runs the official MCP client conformance requirements for protocol
`2026-07-28`.

## Pinned Referee

- Package: `@modelcontextprotocol/conformance`
- Version: `0.2.0-alpha.10`
- Requirements set: `2026-07-28`

The package version is the release anchor recorded by the official frozen
requirements file. Override `MCP_CONFORMANCE_VERSION` only when intentionally
evaluating an upgrade.

## Run Locally

Node.js 22 or newer is required by the pinned conformance harness.

```bash
scripts/run-mcp-conformance.sh --verbose
```

The script builds `target/debug/uxc`, starts the official scenario servers,
and runs `tests/conformance/mcp-client.sh` as the client.

## Expected Failures

`tests/conformance/expected-failures.yml` is a checked-in capability boundary,
not a way to hide regressions. The official runner fails when:

- an unlisted scenario starts failing; or
- a listed expected failure starts passing and the baseline is stale.

Remove entries as soon as the checked-in harness and UXC support the scenario.
OAuth browser/callback automation, Client ID Metadata Documents, and an
interactive MRTR input provider are currently explicit baseline entries.

## Upgrade Procedure

1. Read the official conformance changelog and frozen requirements.
2. Run the new package version locally with the existing baseline.
3. Investigate every new failure or stale baseline entry.
4. Update the pinned version, harness, baseline, and public support matrix in
   one reviewed change.
5. Keep optional extensions such as Tasks separate from required protocol
   conformance.
