# UXC And AgentInbox Source Integration

This note records the proposed integration boundary and migration plan for
`uxc` and `AgentInbox`.

## Summary

`uxc` and `AgentInbox` should not both implement full source-runtime
responsibilities.

The recommended split is:

- `uxc` owns source ingress runtime
- `AgentInbox` owns product-layer source sharing, subscriptions, and delivery

More concretely:

- `uxc` reads from external providers, checkpoints progress, writes raw-oriented
  event envelopes, and persists them into durable streams
- `AgentInbox` consumes those streams, applies agent-specific subscription
  filters, manages consumer offsets, materializes inbox items, and triggers
  activations

This keeps Rust focused on stateful protocol execution and keeps
`AgentInbox` focused on product semantics.

## Problem

The current integration uses daemon subscription jobs as the bridge between the
systems.

Examples:

- GitHub source runtime stores `uxcJobId` and `afterSeq` in source checkpoint
- Feishu source runtime stores `uxcJobId` and `afterSeq` in source checkpoint

Implementation references:

- [../agentinbox/src/sources/github.ts](/Users/jolestar/opensource/src/github.com/holon-run/agentinbox/src/sources/github.ts)
- [../agentinbox/src/sources/feishu.ts](/Users/jolestar/opensource/src/github.com/holon-run/agentinbox/src/sources/feishu.ts)

This creates a fragile contract:

- source identity in `AgentInbox` is stable
- runtime identity in `uxc` is `job_id`
- consumer progress and source ingress progress are mixed together
- rebuilding `AgentInbox` state can orphan durable `uxc` jobs

`AgentInbox`'s own event bus design already points at a better split:

- source ingress checkpoint should remain source-level
- subscription consumption progress should be subscription-level

Reference:

- [../agentinbox/docs/site/concepts/eventbus-backend.md](/Users/jolestar/opensource/src/github.com/holon-run/agentinbox/docs/site/concepts/eventbus-backend.md)

## Target Boundary

### `uxc` Responsibilities

- auth, OAuth, credentials, bindings
- source-runtime connection management
- polling, websocket, long-connection, and protocol-specific ingress handling
- source-side checkpoints
- stable raw stream envelopes
- durable stream persistence
- runtime health, reconnect, resume, and status

### `AgentInbox` Responsibilities

- source registry and shared-source ownership
- source schemas presented to users and agents
- subscription definitions and filter evaluation
- consumer offsets, lag, reset, replay policy
- inbox item materialization
- activation and outbound delivery

### Rule Of Thumb

If the question is:

- "how do we talk to the provider reliably?" => `uxc`
- "which agent wants which events and what do we do with them?" => `AgentInbox`

## Why Not Move All Subscriptions Into AgentInbox

That would make `AgentInbox` responsible for:

- provider auth reuse
- poll checkpoint logic
- reconnect and resume behavior
- protocol-specific runtime details
- durable background worker lifecycle

That would duplicate a growing portion of `uxc` in TypeScript and would blur the
boundary already documented in `AgentInbox`'s architecture note.

Reference:

- [../agentinbox/docs/site/concepts/architecture.md](/Users/jolestar/opensource/src/github.com/holon-run/agentinbox/docs/site/concepts/architecture.md)

## Why Not Make `uxc` The Consumer Broker

That would make `uxc` responsible for:

- per-subscription consumer ids
- agent-specific filters
- consumer lag and reset
- inbox routing
- activation semantics

Those are product-level concerns and belong in `AgentInbox`.

## Integration Contract

The integration should move from `job_id` coupling to `namespace + source_key`
source runtime management.

### Management Identity Mapping

For `AgentInbox`, the recommended mapping is:

- `namespace = "agentinbox"`
- `source_key = "<sourceType>:<sourceKey>"`

Examples:

- `github_repo:holon-run/uxc`
- `github_repo_ci:holon-run/agentinbox`
- `feishu_bot:chat:oc_xxx`

This gives each hosted `AgentInbox` source a stable local management identity in
`uxc` without relying on a transient daemon `job_id`.

`uxc` should treat `(namespace, source_key)` as:

- a caller-provided idempotent management key
- not an authorization boundary
- not the normalized ingress-definition fingerprint

This mapping assumes one local `AgentInbox` daemon instance per user
environment in v1. Under that assumption, `namespace = "agentinbox"` is
sufficient. If multi-instance local deployments become a supported model later,
the namespace strategy can be widened then.

### Runtime Reconciliation

`AgentInbox` should stop asking:

- "does my old `uxcJobId` still exist?"

It should instead ask:

- "ensure the ingress runtime for this managed source exists and matches this
  spec"

That means the primary integration call becomes:

- `uxc source.ensure(namespace, source_key, spec)`

`uxc` then computes an internal `spec_key` from the normalized ingress spec.

Rules:

- same `namespace + source_key` + same `spec_key` => reuse
- same `namespace + source_key` + different `spec_key` => replace the current
  `run_id` while keeping the same `stream_id`
- different `namespace + source_key` => create a different managed source

### Stream Consumption

After `source.ensure`, `AgentInbox` reads from the returned `stream_id`.

`AgentInbox` stores:

- `sourceId -> stream_id`
- subscription consumer offsets in its own backend

`uxc` stores:

- source runtime state
- source-side checkpoint
- stream data

The current `run_id` should be treated as a runtime-generation handle for
observability and debugging, not as the stable integration key.

For v1, `stream_id` remains stable for the same managed source identity even
when the ingress `spec_key` changes.

For v1, `uxc` does not execute source-specific extractor or projection logic
for higher-level products. The stream boundary remains raw-oriented.

## Data Ownership

### Stored In `uxc`

- runtime spec
- normalized spec key
- provider-side checkpoint
- runtime state and health
- stream metadata
- durable stream events

### Stored In `AgentInbox`

- `Source`
- `Subscription`
- `Inbox`
- `Activation`
- delivery handles
- consumer offsets
- lag / reset state

### Not Shared

The systems should not share one mixed checkpoint record that means both:

- provider ingress progress
- consumer read progress

Those should remain explicitly separate.

## Migration Strategy

Migration should be staged.

## Phase 1: Managed Source Runtime Contract In `uxc`

Add to `uxc`:

- `source.ensure`
- `source.status`
- `source.stop`
- `source.delete`
- `stream.read`

Add management identity:

- `namespace`
- `source_key`

Keep existing `subscribe` APIs as compatibility wrappers.

Goal:

- stop relying on orphan-prone bare `job_id`
- let one managed source deterministically reuse or replace its runtime
- let callers intentionally remove a managed source instead of only stopping it

## Phase 2: Migrate `AgentInbox` GitHub Source

Change `github_repo` integration:

- replace `ensureRepoEventsSubscription(...checkpoint.uxcJobId...)`
- call `uxc source.ensure(...)`
- persist `stream_id` in source runtime state instead of `uxcJobId`
- move read path to `stream.read`

The source checkpoint in `AgentInbox` should no longer contain:

- `uxcJobId`
- `afterSeq`

It may continue to carry local source-runtime summary fields temporarily, but
ingress progress should move to `uxc`.

## Phase 3: Migrate `AgentInbox` Feishu Source

Apply the same pattern to `feishu_bot`:

- `source.ensure` with `sourceType=feishu_bot`
- `stream.read` for normalized events
- `uxc` owns long-connection runtime lifecycle

## Phase 4: Move Subscription Consumers Fully Onto AgentInbox Backend

After GitHub and Feishu ingress both read through `stream_id`, complete the
event bus separation:

- one source maps to one ingress stream
- one subscription maps to one independent consumer
- replay/reset use `AgentInbox` backend offsets, not source ingress offsets

This aligns with `AgentInbox`'s event bus design.

## Compatibility Plan

The migration does not require a flag day.

### Short Term

- existing `subscribe` methods continue to work
- `AgentInbox` can adopt new `source` and `stream` APIs incrementally
- legacy `uxcJobId`-based checkpoints remain readable during migration

### Medium Term

- source adapters prefer `stream_id`
- `uxcJobId` becomes debug metadata only
- `subscription.events(job_id, after_seq)` is used less or only by direct CLI
  workflows

### Long Term

- product integrations should stop depending on durable subscription jobs as the
  primary abstraction
- `source` and `stream` become the stable local-service contract

## First Vertical Slice

The recommended first slice is `github_repo`.

Why:

- it is poll-based and easier to model than long-connection transports
- it already exposes a clear stable `source_key` and normalized product
  semantics
- it is the source type most directly tied to the orphan-job problem

First-slice plan:

1. add `uxc source.ensure/status/stop` and `stream.read`
2. implement GitHub ingress runtime in `uxc` using existing poll/runtime code
3. switch `AgentInbox` GitHub source runtime to `stream_id`
4. validate source rebuild, reset, and replay flows

## Runtime And Schema Mapping

`uxc` should keep a raw-oriented ingress boundary in v1.

`AgentInbox` should keep product-oriented schema mapping.

`AgentInbox` decides:

- which fields are extracted from the raw payload
- which of those extracted fields appear in source schema docs
- which fields are filterable in product APIs
- what the final product `eventVariant` is
- how a matching event becomes an inbox item or delivery handle

## Relationship To Source Profiles

This integration model fits the `remote_source` profile direction proposed in
`AgentInbox` issue #37.

Profile/template responsibilities in `AgentInbox` should be split into two
layers:

### 1. Ingress Spec Passed To `uxc`

This includes provider/runtime details such as:

- endpoint
- operation or transport
- poll vs subscribe mode
- checkpoint strategy
- auth/profile binding

`uxc` receives this ingress spec and computes `spec_key`.

### 2. Product Mapping Spec Kept In `AgentInbox`

This includes product semantics such as:

- item extraction
- metadata mapping
- payload mapping
- final product `eventVariant` mapping
- summary mapping

This stays in `AgentInbox` because it is product-layer schema and routing logic,
not ingress-runtime logic.

V1 intentionally does not introduce a declarative extractor execution engine in
`uxc`. If that becomes desirable later for performance or operational reasons,
it should be designed as a separate extension model after `AgentInbox`'s source
profile system has stabilized.

### Builtin And User-Defined Profiles

This split lets both kinds of `AgentInbox` source profiles share one runtime
contract with `uxc`:

- builtin profiles such as `github_repo`, `github_repo_ci`, and `feishu_bot`
- user-defined profiles layered on `remote_source`

In both cases, `AgentInbox` should continue to define the logical
`source_key`. `uxc` should not generate that key on behalf of the caller.

## Risks

### 1. Overloading `uxc` With Product Logic

Mitigation:

- keep `uxc` event normalization thin
- do not add consumer offsets or delivery semantics to `uxc`

### 2. Keeping Two Runtime Paths Alive Too Long

Mitigation:

- migrate source types one by one
- keep old `subscribe` APIs as wrappers rather than separate implementations

### 3. Stream Retention Surprises

Mitigation:

- make retention explicit per stream
- define a conservative default
- keep replay/reset semantics in `AgentInbox`

## Success Criteria

This integration split is successful when:

- rebuilding `AgentInbox` state does not orphan durable ingress runtimes
- source ingress checkpoint is owned by `uxc`
- subscription consumer offset is owned by `AgentInbox`
- one source can safely feed multiple subscriptions with independent offsets
- provider runtime logic is not duplicated across Rust and TypeScript

## V1 Follow-Through Decisions

- `AgentInbox` does not expose its internal database `sourceId` to `uxc` in v1
- `AgentInbox` relies on `namespace=agentinbox` and
  `source_key=<sourceType>:<sourceKey>` as the stable management identity
- `AgentInbox` does not persistently cache `uxc stream.info` metadata in v1 and
  reads it on demand when needed
