# Generated Runtime Clients

This note records the design direction for issue #334.

## Summary

UXC should support generated clients for the daemon/runtime surface so applications can embed UXC as
a stable typed runtime contract instead of consuming it only through the CLI.

The design target is:

- one runtime surface across protocols
- generated clients against that runtime surface
- language-specific emitters layered on the same exported codegen schema

This note is intentionally not limited to TypeScript, even though TypeScript is the most obvious
first emitter because the repository already has a daemon client package.

## Scope

This note covers:

- what generated clients target
- what schema/codegen input should be exported
- what the generated client surface should look like
- how envelopes, errors, metadata, artifacts, and subscriptions should appear in generated clients
- how this relates to the existing daemon client package

This note does not cover:

- implementing a first emitter
- protocol-native direct clients
- replacing the daemon runtime with per-language bespoke logic
- final naming for every generated method

## Current Implementation Baseline

### 1. The daemon already exposes a stable runtime request/response shape

Current runtime invocation is centered on:

- `RuntimeInvokeRequest`
- `RuntimeInvokeResponse`
- `RuntimeInvokeOptions`

Implementation reference:

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L116)

### 2. The response surface is still envelope-oriented

The daemon currently returns:

- `protocol`
- `endpoint`
- `kind`
- optional `operation`
- `data`
- `duration_ms`
- `meta`

This is a normalized runtime envelope, but it is still dynamically shaped because `data` is `Value`.

Implementation reference:

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L145)

### 3. A handwritten TypeScript daemon client already exists

The current Node package exposes:

- `call(...)`
- `subscribeStart(...)`
- `subscribeEvents(...)`
- `daemonStatus()`
- `daemonSessions()`

But it still returns mostly untyped `data` payloads and does not generate host/operation-specific
typed methods.

Implementation reference:

- [packages/uxc-daemon-client/src/index.ts](/Users/jolestar/opensource/src/github.com/holon-run/uxc/packages/uxc-daemon-client/src/index.ts#L168)

## Design Principles

### 1. Target the runtime surface, not each protocol directly

Generated clients should target the daemon/runtime contract rather than generating a separate direct
client for OpenAPI, GraphQL, gRPC, MCP, and JSON-RPC.

That preserves UXC's core value:

- one execution surface
- one auth story
- one lifecycle/reuse model
- one output contract

### 2. Separate discovery-time schema export from runtime request transport

Code generation needs a schema/codegen input that is richer and more stable than the current
ad-hoc runtime `data: Value` envelope.

The generated client should not scrape `-h` output or infer shapes from arbitrary responses. It
should compile from an explicit exported schema.

### 3. Keep one language-neutral source of truth

There should be one exported codegen schema from which multiple emitters can be generated.

That schema should describe:

- hosts
- operations
- input schemas
- output schemas
- subscription surfaces
- envelope metadata
- runtime options

Language emitters then map that schema into:

- TypeScript types and methods
- Python typed wrappers
- Go client types
- Rust structs/traits

### 4. Preserve the runtime envelope semantics

Generated clients should make the runtime surface easier to consume, not hide the important runtime
facts entirely.

That means generated clients should still preserve:

- protocol
- endpoint
- operation identity
- runtime metadata
- artifact/compaction metadata
- subscription semantics

But they may present those through a typed wrapper instead of raw `unknown` / `Value`.

## Proposed Product Surface

Generated clients should be described as:

- generated runtime clients
- typed clients for named UXC hosts
- language-specific bindings over the daemon/runtime surface

The product story should be:

- CLI for discovery
- daemon for execution
- generated clients for embedding

## V1 Design

## 1. Source of Truth

V1 should introduce an explicit exported codegen schema.

This schema is the canonical input to all emitters.

It should not be:

- the raw daemon JSON-RPC methods
- the CLI `-h` output
- protocol-native schemas directly

It should be a UXC-specific runtime description that sits between protocol discovery and generated
language bindings.

### Proposed schema contents

At minimum, the exported schema should include:

- schema version
- generated-at metadata
- host identifier
- endpoint / link reference
- protocol family
- operation list
- per-operation:
  - operation id
  - display name
  - kind
  - input schema
  - output schema
  - output envelope kind
  - whether it is execute-only, help-only, or subscribable
- shared runtime options contract
- artifact/compaction metadata contract

### Host target

V1 generated clients should target a concrete host surface.

That means generation should work from:

- a named link
- or an explicit endpoint + schema/discovery result

The important thing is that generation happens after UXC has already resolved the host surface into
a stable operation model.

## 2. Generated Client Shape

V1 generated clients should expose:

- a host-scoped client type
- one method per operation
- typed input for each operation
- typed result wrapper for each operation

The key point is that generated clients should not force applications to manually write:

- operation ids as strings
- payload shapes as `Record<string, unknown>`
- response decoding logic for every call

### Example conceptual shape

The conceptual generated client should feel like:

```ts
const client = new PetstoreClient(runtime);

const result = await client.getPet({ petId: 1 });
```

Where `result` is still a typed runtime result, not a bare protocol-native object.

## 3. Result Shape

Generated clients should not collapse everything into the bare `data` payload.

V1 generated methods should return a typed runtime result wrapper with at least:

- `data`
- `meta`
- `protocol`
- `endpoint`
- `operation`
- `kind`

Conceptually:

```ts
type RuntimeResult<TData, TMeta = RuntimeMeta> = {
  protocol: string;
  endpoint: string;
  operation?: string | null;
  kind: string;
  data: TData;
  meta: TMeta;
  duration_ms?: number | null;
};
```

This keeps generated clients aligned with the daemon contract while still making operation payloads
typed.

## 4. Error Model

Generated clients should preserve daemon/runtime failure semantics rather than inventing a separate
error universe.

V1 should distinguish:

- transport / socket / daemon connection errors
- JSON-RPC method errors
- runtime invocation errors

Language emitters may map those into idiomatic language exceptions or result types, but the codegen
schema should keep the underlying daemon error model stable.

## 5. Artifact and Compaction Semantics

Generated clients must align with the artifact contract in `#338`.

That means generated result types should be able to express:

- inline data
- preview-only data
- local artifact paths
- daemon artifact refs

V1 generated clients do not need to solve artifact retrieval yet, but they must type the metadata
correctly so callers can see:

- whether the response was compacted
- whether a local path is available
- whether a daemon ref is available

## 6. Subscription Semantics

Generated clients should eventually cover subscriptions, but v1 should not require full parity with
all runtime subscription modes before the schema is useful.

V1 schema should still include subscription-capable operations so emitters have a stable place to
grow into.

The generated client contract should eventually support:

- starting subscriptions
- typed event envelopes
- polling or streaming consumption
- lifecycle-aware cleanup

But the first emitter may reasonably start with request/response operations and add subscription
helpers later.

## Codegen Schema Structure

V1 should introduce a language-neutral schema roughly shaped like:

```json
{
  "version": "v1",
  "host": {
    "id": "petstore",
    "endpoint": "https://petstore3.swagger.io/api/v3",
    "protocol": "openapi"
  },
  "runtime": {
    "invoke_options_schema": {},
    "result_meta_schema": {},
    "artifact_meta_schema": {}
  },
  "operations": [
    {
      "id": "get:/pet/{petId}",
      "display_name": "GET /pet/{petId}",
      "kind": "execute",
      "input_schema": {},
      "output_schema": {},
      "result_kind": "call_result"
    }
  ]
}
```

This is illustrative, not the final field layout. The important decision is that the codegen input
is:

- UXC-specific
- host-scoped
- operation-aware
- envelope-aware

## Relationship to `uxc-daemon-client`

The existing handwritten daemon client package should not be thrown away.

Instead, it should become one of these:

### Option A

The low-level runtime transport used by generated TypeScript clients.

This means:

- keep `UxcDaemonClient` as the base JSON-RPC transport
- generated TS clients import and use it internally

### Option B

The manually maintained reference implementation of what generated clients should feel like.

Even if TypeScript generation comes later, the current package can guide the runtime transport and
error conventions.

V1 design should assume Option A is the most practical path.

## Compatibility Notes

This design intentionally depends on the lifecycle and artifact contracts:

- `#337` for daemon/runtime lifecycle semantics
- `#338` for artifact and compaction semantics

Generated clients should not freeze assumptions that conflict with those contracts.

That is why this issue is a design note first, not an implementation shortcut.

## Follow-Up Work

This note should drive follow-up implementation issues for:

- exported codegen schema format
- first emitter language
- method naming and namespacing rules
- typed artifact metadata
- typed subscription event support
- compatibility and snapshot testing for generated clients

## Open Questions

- Should generation target links, explicit endpoints, or both from day one?
- Should the exported schema be produced by the CLI, by the daemon, or by both?
- How much of subscription support belongs in the first emitter?
- Should generated clients always return the full runtime result wrapper, or offer both raw and
  unwrapped helper methods?
