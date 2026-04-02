# Artifact Contract

This note records the artifact contract for issue #338.

## Summary

UXC already has partial support for two related behaviors:

- schema-aware local file path inputs for multipart OpenAPI operations
- path-preserving artifact output from MCP tool results

It also has an open design need around large-response compaction and artifact access.

This note defines a v1 artifact contract that keeps those concerns under one model:

- callers may provide local file inputs where the schema says a field is file-like
- outputs may carry artifact references instead of requiring every value to be inlined
- large responses may be compacted into a preview plus artifact metadata

The goal is one envelope-level model for payloads that are too large, too file-oriented, or too
awkward to inline directly.

## Scope

This note covers:

- local file input semantics
- artifact output semantics
- response compaction / externalization semantics
- runtime and generated-client implications

This note does not cover:

- protocol-specific binary streaming redesigns
- remote object storage
- mandatory base64 payload transport
- a final implementation choice between local paths and daemon-managed refs

## Current Implementation Baseline

### 1. OpenAPI multipart input already exposes file-shaped schema hints

When an OpenAPI request body includes multipart file fields, UXC already annotates the discovered
input schema with:

- `x-uxc-file-fields`
- `x-uxc-file-input = local_path_string`

Implementation reference:

- [src/adapters/openapi.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/adapters/openapi.rs#L802)

### 2. Multipart execution already accepts local path strings for file fields

When the request body is prepared for multipart execution, file fields are currently expected as
plain local path strings, not as raw bytes.

Implementation reference:

- [src/adapters/openapi.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/adapters/openapi.rs#L1358)
- [src/adapters/openapi.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/adapters/openapi.rs#L1432)

### 3. MCP output already preserves local artifact paths

When an MCP tool result includes structured artifact metadata with a local `path`, UXC currently
preserves that path in the JSON result instead of stripping it.

Implementation reference:

- [src/adapters/mcp/mod.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/adapters/mcp/mod.rs#L778)

### 4. Large-response compaction is still only a design direction

Issue `#142` already frames the key need:

- keep default responses small
- preserve access to the full payload
- stay protocol-agnostic

But there is not yet a unified envelope contract for preview + artifact access.

## Design Principles

### 1. Schema-aware first

UXC should prefer schema-aware file handling over new ad hoc CLI markers.

If the schema already says a field is file-like, callers should provide a local path string and let
UXC interpret it according to schema.

### 2. Artifact references are envelope-level, not transport-specific

The artifact model should belong to the UXC output envelope and runtime surface, not to one
protocol.

That means:

- OpenAPI, MCP, GraphQL, gRPC, and JSON-RPC may all produce inline values or artifact references
- the caller should not need a different large-payload model per protocol

### 3. Preserve direct usability

For humans and agents, the first useful behavior is:

- pass a local path in
- receive either inline output or a machine-readable artifact reference out

The model should not require a separate binary transport layer for normal v1 use.

### 4. Keep portability explicit

Local filesystem paths are useful, but they are not universally portable. The contract should allow:

- direct local path references when the caller/runtime share a machine
- future daemon-managed refs when the caller should not rely on arbitrary filesystem access

## V1 Contract

## Input Contract

### Local file inputs

For v1, file-like inputs are represented as local path strings when the operation schema says the
field is file-like.

Examples:

- multipart OpenAPI file field: `file=/abs/path/report.pdf`
- nested structured field: `attachments[0].source.path=/abs/path/report.pdf`

UXC should not require a special file marker when the schema already establishes file semantics.

### Input classification

A value counts as a file input only when at least one of the following is true:

- the protocol schema marks the field as file-like
- the adapter-specific input schema explicitly marks the field as a local-path file input

A plain string that merely happens to look like a path is not automatically upgraded into a file
input unless schema/adaptor rules say so.

### Mixed structured arguments

File inputs must continue to work alongside:

- `key=value`
- nested path assignment
- per-field JSON assignment with `:=`
- positional JSON objects

The artifact contract does not introduce a second argument model. It only clarifies what certain
string values mean when the schema says the field is file-like.

## Output Contract

### Artifact references

For v1, UXC should treat an artifact as an output reference to non-inline content, especially:

- local files produced by the underlying runtime
- compacted large payloads written out-of-line

The minimal artifact reference shape is:

```json
{
  "kind": "file",
  "name": "report.csv",
  "path": "/tmp/webmcp-artifacts/report.csv"
}
```

Optional metadata may include:

- `mimeType`
- `bytes`
- `sha256`
- `source`
- `description`

### Artifact placement

For v1, artifact references may appear inside `data`, especially when the underlying tool/runtime
already returns structured artifact metadata.

The envelope-level contract should preserve those references without stripping or rewriting them
unless UXC explicitly externalizes content itself.

## Compaction Contract

### Trigger

Compaction is the act of returning:

- a preview inline
- the full payload out-of-line through artifact metadata

This should be available for:

- large host help output
- large operation results
- large protocol-specific structured payloads

### Result shape

When UXC compacts a response, the envelope should continue to return `data`, but `data` becomes a
preview rather than the full payload.

The envelope `meta` should then include compaction fields such as:

- `artifact_truncated: true`
- `artifact_kind`
- `artifact_bytes`
- `artifact_path` or `artifact_ref`
- optional `artifact_sha256`

This keeps the JSON envelope stable while still signaling that the inline `data` is incomplete by
design.

### Preview contract

The preview should be:

- bounded
- useful for inspection
- insufficiently large to recreate the full payload silently

The exact preview policy can vary by output kind, but the contract should always make truncation
explicit through metadata.

### Current v1 defaults

The current v1 implementation uses:

- automatic compaction above `65536` bytes
- envelope-level metadata fields:
  - `artifact_truncated`
  - `artifact_kind`
  - `artifact_bytes`
  - `artifact_path`
  - `artifact_sha256`
- local path-backed externalization under daemon-managed local artifact directory
- inline preview data for compacted payloads

`codegen_host_schema` is intentionally excluded from compaction in v1.

## Local Path vs Daemon Ref

The current design space has two legitimate output reference forms:

### A. Local path

Pros:

- simple
- immediately useful when caller and runtime share a machine

Cons:

- not portable across hosts
- assumes caller can read the filesystem

### B. Daemon-managed ref

Pros:

- better fit for generated clients and remote callers
- does not require arbitrary filesystem access

Cons:

- needs artifact lifecycle management
- introduces retrieval APIs

### V1 decision

This note does not force a final implementation choice yet.

The v1 contract only requires that:

- output metadata may point to externalized content
- the contract can represent either a local path or a daemon-managed ref
- callers can always distinguish inline preview from externalized full content

## Envelope-Level Metadata

The artifact contract should reserve these envelope-level concepts:

- `artifact_truncated`
- `artifact_kind`
- `artifact_bytes`
- `artifact_path`
- `artifact_ref`
- `artifact_sha256`

Not every response needs every field, but these names should remain stable once implemented.

## Runtime and Client Implications

The daemon/runtime surface should preserve artifact references without collapsing them back into
protocol-specific shapes.

This matters for future generated clients:

- generated clients should know when a response is preview-only
- generated clients should know whether they received a local path or a daemon ref
- generated clients should not need protocol-specific recovery logic for large payloads

## Compatibility Notes

This contract keeps current multipart file input behavior compatible:

- file-shaped OpenAPI inputs continue to accept local path strings

This contract also keeps current MCP artifact-path behavior compatible:

- local artifact paths returned by tools continue to pass through unchanged

The main change is not new transport behavior. The main change is defining one stable envelope-level
model for:

- file-oriented inputs
- artifact-oriented outputs
- compacted large responses

## Follow-Up Work

This contract should drive follow-up implementation under:

- `#318`: file-path inputs and local artifact-path outputs
- `#142`: response compaction and artifact access

Potential next implementation slices:

- preserve/stabilize artifact metadata across adapters
- define compaction thresholds and config knobs
- decide first implementation path: local path, daemon ref, or both
- add runtime/client-facing artifact retrieval APIs if daemon refs are chosen

## Open Questions

- Should v1 compaction default to opt-in or automatic above thresholds?
- Should local artifact paths be preferred when already provided by the underlying runtime, even if
  daemon refs are later added?
- Which output kinds deserve specialized previews versus generic truncation?
