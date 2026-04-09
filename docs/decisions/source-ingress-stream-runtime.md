# Managed Source Streams

This note records the proposed design direction behind issue #360 and the
broader evolution of `uxc`'s daemon/runtime surface.

## Summary

`uxc` should evolve from a daemon that exposes durable subscription jobs into a
managed source streams service.

The new responsibility boundary for `uxc` is:

- authenticate against external providers
- host source ingress runtimes
- manage source-side checkpoints
- wrap external events in a stable stream envelope
- persist those events into durable append-only streams
- expose stream-oriented read and runtime-management APIs

`uxc` should not become the product-layer consumer broker. It should not own:

- agent-specific subscriptions
- filter evaluation
- consumer offsets or lag
- inbox materialization
- activation or delivery semantics

Those remain higher-level concerns for whatever product or application consumes
`uxc` streams.

## Problem

The current daemon subscription model is too low-level for source hosting.

Today a durable subscription is identified primarily by `job_id`. The daemon
stores:

- `SubscribeStartRequest`
- `SubscriptionJobView`

Implementation reference:

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L180)
- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L267)
- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L667)

This model is adequate for:

- direct CLI usage
- debugging a durable background subscription
- a single caller that persists `job_id`

It is not adequate for shared source hosting because:

- there is no stable external source management identity
- there is no stream abstraction separate from the runtime job
- sink choice is coupled to runtime identity
- consumer progress is externalized as `after_seq`
- durable jobs can become orphaned when an upstream system loses `job_id`

## Goals

- make source ingress a first-class daemon/runtime concept
- separate source-side checkpointing from consumer-side progress
- persist ingress events durably so consumers can replay or lag independently
- preserve provider-native payloads while exposing a stable event envelope
- support deterministic runtime reuse and replacement by stable management
  identity
- keep the runtime contract local-first and daemon-friendly

## Non-Goals

- do not build a general-purpose message broker in `uxc`
- do not move higher-level subscription filtering into `uxc`
- do not expose SQL or database-product semantics in the public API
- do not require every caller to understand provider-native checkpoints
- do not remove the current `subscribe` surface immediately

## Design Principles

### 1. Separate ingress from consumption

Source ingress and source consumption are different layers.

Ingress answers:

- how do we connect to the provider?
- what is the provider-side checkpoint?
- what events have been ingested into the local stream?

Consumption answers:

- which consumer wants which events?
- where is each consumer offset?
- how are events filtered, replayed, or routed?

`uxc` should own ingress. Product systems should own consumption.

### 2. Streams are the durable boundary

`job_id` should no longer be the main integration surface. The main boundary
should be:

- source management identity
- source run identity
- stream identity
- append-only event offsets

Runtime jobs remain an implementation detail that powers a stream.

### 3. Standardize thinly, preserve fully

The v1 stream event model should preserve complete provider-native payloads and
avoid source-specific extraction rules in the core runtime contract.

`uxc` should not overfit event shapes to one product's workflow semantics, and
it should not require per-source extractor logic just to make the stream usable.

### 4. Source management identity must be explicit

Durable source runtimes need a stable management identity so the daemon can:

- reuse an existing runtime
- replace an outdated runtime
- stop the correct runtime
- avoid orphaned duplicates

This identity is not a permission boundary. It is a caller-provided coordination
key for idempotent runtime management.

This identity is also not the same thing as the provider-side subscription key
or the normalized ingress spec fingerprint.

## Proposed Model

The v1 managed source streams model introduces three main persistent concepts.

### 1. Source Runtime

A `SourceRuntime` is the live or resumable ingress worker for one managed source
definition.

Suggested fields:

- `run_id`
- `namespace`
- `source_key`
- `spec_key`
- `status`
- `stream_id`
- `checkpoint_json`
- `created_at_unix`
- `started_at_unix`
- `last_event_at_unix`
- `last_error`
- `restart_count`
- `last_resume_at_unix`

Semantics:

- `namespace + source_key` define the stable external management identity
- `run_id` identifies one concrete runtime execution generation
- `spec_key` defines the normalized ingress definition
- `stream_id` points at the durable local stream that receives events

### 2. Event Stream

An `EventStream` is the append-only durable log produced by one source runtime.

Suggested fields:

- `stream_id`
- `namespace`
- `source_key`
- `retention_policy`
- `created_at_unix`
- `last_offset`

In v1, streams should not be reused across distinct `(namespace, source_key)`
identities by default.

Reuse rules:

- same `namespace + source_key` always maps to the same `stream_id` in v1
- same `namespace + source_key` + same normalized source spec => reuse runtime
  and stream
- same `namespace + source_key` + changed normalized source spec => replace
  the current runtime generation and keep the same stream
- different `namespace + source_key` => create a different runtime and
  different stream

### 3. Stream Event

A `StreamEvent` is the stable raw-oriented event envelope written into the
stream.

Suggested JSON shape:

```json
{
  "stream_id": "stream_1",
  "offset": 42,
  "source_key": "holon-run/uxc",
  "ingested_at": "2026-04-09T10:00:02Z",
  "raw_payload": {}
}
```

Required properties:

- `stream_id`
- `offset`
- `ingested_at`
- `raw_payload`

Optional-but-strongly-recommended properties:

- `source_key`

`raw_payload` should remain provider-native and complete enough for replay,
debugging, and future remapping.

In v1, payload-derived fields such as:

- source-native event ids
- source-native timestamps
- provider-oriented metadata projections
- provider-oriented variants

are not part of the required core stream contract. Those fields require
source-specific extraction logic, which is intentionally left to higher-level
systems or a future extractor-extension design.

## SQLite Storage Design

The public surface should be stream-oriented, but the local daemon should use
SQLite for durability and read performance.

### Tables

Suggested v1 schema:

#### `source_runtimes`

- `run_id text primary key`
- `namespace text not null`
- `source_key text not null`
- `spec_key text not null`
- `stream_id text not null`
- `status text not null`
- `checkpoint_json text`
- `created_at_unix integer not null`
- `started_at_unix integer`
- `last_event_at_unix integer`
- `last_error text`
- `restart_count integer not null default 0`
- `last_resume_at_unix integer`

Indexes:

- unique `(namespace, source_key)`
- index on `(spec_key)`

#### `event_streams`

- `stream_id text primary key`
- `namespace text not null`
- `source_key text not null`
- `retention_policy_json text`
- `created_at_unix integer not null`
- `last_offset integer not null default 0`

Indexes:

- unique `(namespace, source_key)`

#### `stream_events`

- `stream_id text not null`
- `offset integer not null`
- `ingested_at text not null`
- `raw_payload_json text not null`
- primary key `(stream_id, offset)`

Indexes:

- no additional v1 event indexes are required beyond the primary key

### Retention

Retention should be configured per stream, not per consumer.

Suggested v1 retention options:

- max event count
- max age seconds
- manual trim

Retention is a managed-source concern. Consumer offsets belong elsewhere.

## API Design

The new API surface should be runtime-oriented and stream-oriented.

### 1. `source.ensure`

Input:

- `namespace`
- `source_key`
- `spec`
- optional retention policy

Behavior:

1. normalize `spec`
2. compute `spec_key`
3. if `(namespace, source_key)` exists with same `spec_key`, reuse it
4. if `(namespace, source_key)` exists with different `spec_key`,
   replace the current runtime generation while keeping the same `stream_id`
5. return runtime and stream metadata

Output:

- `run_id`
- `stream_id`
- `status`
- `reused`
- `replaced_previous`

### 2. `source.status`

Lookup by:

- `run_id`
- or `namespace + source_key`

Output:

- runtime state
- stream id
- checkpoint summary
- last event timestamp
- last error

### 3. `source.stop`

Stop by:

- `run_id`
- or `namespace + source_key`

Semantics:

- stop ingress worker
- persist final runtime state
- keep the managed source binding and stream intact

### 4. `source.delete`

Delete by:

- `run_id`
- or `namespace + source_key`

Semantics:

- remove the managed source binding and runtime state
- stop any active ingress worker for that managed source
- do not imply immediate stream deletion in v1
- leave stream lifecycle to retention or future explicit stream-deletion policy

### 5. `stream.read`

Input:

- `stream_id`
- `after_offset`
- `limit`

Output:

- `events`
- `next_after_offset`
- `has_more`
- optional `earliest_offset`
- optional `latest_offset`

This replaces the current idea that `job_id` is the thing consumers read from.

### 6. `stream.info`

Output:

- source identity
- retention policy
- earliest offset
- latest offset
- approximate event count
- last ingested timestamp

### 7. `stream.trim`

Administrative endpoint for:

- manual retention enforcement
- testing
- repair operations

## Identity Rules

### External Management Identity

External management identity answers:

- who is allowed to manage this runtime?
- which runtime should be reused or replaced?

V1 fields:

- `namespace`
- `source_key`

Examples:

- `namespace=sync-service`, `source_key=github_repo:holon-run/uxc`
- `namespace=skill:my-skill`, `source_key=alerts:btc-usd`
- `namespace=manual`, `source_key=feishu_bot:chat:oc_xxx`

Rules:

- `namespace` scopes the caller/system namespace
- `source_key` is a caller-provided stable logical source identifier
- `uxc` treats `(namespace, source_key)` as an idempotent management key
- `uxc` does not interpret this pair as an authorization or ACL boundary

### Spec Key

The spec key answers:

- is this the same ingress definition?

The normalized spec key should include:

- endpoint
- protocol family
- source operation id or transport hint
- relevant request args
- resource uri
- poll config
- auth principal identity fingerprint
- schema-discovery fields that materially affect behavior

It should not include:

- request id
- sink path
- presentation metadata
- access token values
- refresh token values
- secret material or secret-derived hashes that change on credential rotation

This preserves the desired behavior:

- same principal, rotated credential => reuse
- different principal or different data-scope identity => replace

### Spec Change Semantics

When `spec_key` changes for the same `(namespace, source_key)`, v1 treats this
as a managed-source runtime replacement, not as a managed-source identity
change.

Effects:

- `run_id` changes
- `stream_id` stays the same
- the previous runtime generation is stopped and replaced
- consumer integrations keep reading from the same stream

Checkpoint handling in v1 should be conservative:

- a changed `spec_key` invalidates the previous source-side checkpoint by
  default
- the new runtime generation starts from a fresh checkpoint state unless a
  future compatibility rule explicitly says otherwise

This keeps the behavior simple:

- runtime replacement is separate from stream migration
- stream continuity is preserved for consumers
- checkpoint reuse is opt-in in the future rather than implicitly unsafe

### Run Id

The run id answers:

- which concrete execution generation is currently or was previously running?

Rules:

- `run_id` is generated by `uxc`
- `run_id` changes when a source runtime is replaced or restarted as a new
  execution generation
- callers should use `run_id` for observability and debugging, not as the
  stable management key

## Relationship To Current `subscribe` API

The current `subscribe start/status/events/stop` surface should remain as a
compatibility layer during migration.

Migration target:

- `subscribe start` becomes a thin wrapper that creates a managed source stream
- `subscription.events(job_id, after_seq)` becomes a wrapper over `stream.read`
- durable `job_id` remains observable for CLI/debugging, but is no longer the
  primary integration handle for product systems

This lets the codebase evolve without forcing a flag day across existing CLI
users or daemon clients.

## Why This Direction

This split follows mature source-ingestion and stream patterns:

- source runtime state is distinct from consumer state
- source checkpoint is distinct from consumer offset
- streams are durable append-only logs
- consumer semantics belong above the managed source streams layer

For `uxc`, this means:

- stronger runtime identity
- less orphaning
- better replayability
- a clearer local-service contract

For higher-level systems, this means:

- stable stream ids
- independent consumer offsets
- less coupling to daemon `job_id`

The cost of this v1 simplification is intentional:

- payload-derived extraction stays above `uxc`
- `uxc` does not yet try to standardize source-native event identity or
  metadata
- a future extractor-extension model can be added later if the performance and
  operational tradeoff is worth the added complexity

## V1 Follow-Through Decisions

- source-runtime retention uses one shared default policy in v1 rather than
  source-type-specific defaults
- `uxc` does not expose event-id dedupe guarantees within a stream in v1
- the compatibility `subscribe` surface remains in the first version that ships
  `source` and `stream`, and is marked deprecated in the following version
