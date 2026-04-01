---
title: Docs
summary: One CLI for tools across protocols.
---

# UXC Docs

One CLI for tools across protocols.

UXC helps agents and automation discover and invoke APIs and tools across
OpenAPI, MCP, GraphQL, gRPC, and JSON-RPC through one consistent workflow.

## Start Here

Most flows follow the same path:

```bash
uxc <host> -h
uxc <host> <operation_id> -h
uxc <host> <operation_id> key=value
```

Example:

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

## Explore

- [Getting Started](./getting-started/)
- [Protocols](./protocols/)
- [Auth](./auth/)
- [Daemon](./daemon/)
- [Ecosystem](./ecosystem/)
- [Reference](./reference/)
- [Skills Directory](./skills/)

## Why UXC

Remote capabilities are easy to access in isolation, but hard to reuse
consistently across systems.

UXC exists to turn schema-described remote capabilities into one reusable CLI
entrypoint for agents, skills, scripts, and local applications.

## What You Get

- help-first discovery with `<host> -h` and `<host> <operation_id> -h`
- structured invocation with key/value args, path-style nested args, `:=` per-field JSON, or positional JSON payloads
- deterministic JSON output by default, with opt-in text mode
- reusable auth credentials and endpoint bindings
- daemon-backed session reuse and background subscriptions
- a TypeScript daemon client for local integrations

## Current Scope

This site starts as a public documentation shell for UXC.
It already exposes the shared `skills/` directory through the site tree and
will gradually absorb the user-facing content currently living in repository
docs.

Until the migration is complete, the repository README remains the most complete
single-page overview.

<!-- INDEX:START -->

- [Auth](./auth/)
- [Daemon](./daemon/)
- [Ecosystem](./ecosystem/)
- [Getting Started](./getting-started/)
- [Protocols](./protocols/)
- [Reference](./reference/)
- [Skills](./skills/)

<!-- INDEX:END -->
