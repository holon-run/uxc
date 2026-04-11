# Daemon Session Lifecycle Contract

This note records the target daemon session lifecycle contract for issue `#337`.

## Summary

`uxc`'s daemon is the stable execution surface for long-lived runtimes, especially MCP stdio
sessions. The lifecycle contract needs to explain:

- what defines a reusable session
- what defines session ownership
- what metadata may change across reuse
- when a session is considered idle
- when idle reap is allowed
- what lifecycle state a child may report
- what users and generated clients may observe from `uxc daemon sessions`

This note defines the target contract for daemon-backed session lifecycle. It is primarily about
MCP stdio sessions, because that is where reuse, exclusivity, and idle-reap semantics already
exist.

## Scope

This note covers:

- stdio session identity
- reuse rules
- daemon exclusivity
- idle semantics
- idle reap behavior
- child-reported lifecycle state
- observable daemon session state

This note does not cover:

- new protocol adapters
- generated client design
- CLI argument ergonomics
- artifact or large-response handling

## Current Implementation Baseline

The current stdio session key is based on:

- endpoint
- auth fingerprint
- injected env fingerprint

Implementation reference:

- [src/daemon.rs](../../src/daemon.rs)

The current stdio session stores mutable runtime metadata such as:

- `idle_ttl_secs`
- `link_name`
- `endpoint`
- `daemon_exclusive`
- lifecycle declaration
- latest lifecycle snapshot

Implementation reference:

- [src/daemon.rs](../../src/daemon.rs)

The current cleanup path has already moved off the foreground request path and currently skips
intrusive background probes for `daemon_exclusive` stdio sessions.

Implementation reference:

- [src/daemon.rs](../../src/daemon.rs)

The current implementation is being refactored away from live runtime probes and toward static
lifecycle declaration plus pushed lifecycle snapshots.

## Model

The lifecycle model is split into four layers.

### 1. Session Identity

Session identity answers:

- does this request refer to the same runtime session?

For stdio, identity is:

- endpoint
- auth fingerprint
- injected env fingerprint
- transport/runtime family

Identity does not include:

- `link_name`
- `daemon_idle_ttl`
- `daemon_exclusive`
- lifecycle contract fields

Those fields affect how a session is presented, owned, or cleaned up, but they do not define which
underlying runtime process is being reused.

### 2. Ownership Policy

Ownership policy answers:

- may another request share this session?
- may another request reclaim this session?
- under what conditions must a request fail instead of reusing or evicting?

For v1, ownership policy includes:

- daemon exclusive keys
- in-flight request state
- subscription/resource-subscription state

`daemon_exclusive` is an ownership boundary, not part of identity.

### 3. Lifecycle Contract

Lifecycle contract answers:

- how should the daemon decide whether automatic idle reap is allowed for this child kind?
- what dynamic state may the child report without exposing child-specific internals?

The contract is split into:

- a static lifecycle declaration, fetched once at startup
- a dynamic lifecycle snapshot, pushed from the child to the daemon as notifications

The daemon should not rely on live request-time lifecycle probes as the primary cleanup decision
mechanism for stdio sessions.

### 4. Presentation Hints

Presentation hints answer:

- how should this session be shown to users?

For v1, these include:

- `link_name`
- source link metadata such as skill/docs origin
- endpoint presentation metadata

These hints may change when a request reuses a matching session.

## V1 Contract

## Session Identity

Two stdio requests are eligible to reuse the same runtime session only when all identity fields
match:

- same endpoint
- same auth fingerprint
- same injected env fingerprint
- same stdio runtime family

This contract intentionally allows:

- a direct invocation to reuse a link-created session
- a link invocation to reuse a direct-created session

as long as identity matches.

This is desirable because links are a presentation layer over the same runtime surface, not a
separate runtime family.

## Reuse Rules

When a request targets an existing session with matching identity:

1. the daemon may attempt reuse
2. the daemon must first enforce ownership rules
3. if ownership allows reuse, the existing session is reused instead of spawning a new runtime

Reuse must not silently cross identity boundaries.

If identity does not match, the daemon must create or acquire a different session key rather than
mutating the existing session into a different identity.

## Reuse-Time Updates

If reuse succeeds, the daemon may update:

- `link_name`
- `idle_ttl_secs`
- endpoint presentation metadata stored on the session snapshot

If reuse succeeds and the request supplies exclusive keys, the daemon may also update ownership
registration for those keys, but that is an ownership update, not an identity change.

For v1, `idle_ttl_secs` follows a latest-request-wins rule:

- the most recent successful reuse or creation request sets the session cleanup TTL
- this applies equally to link-backed calls, direct calls, and daemon-backed subscription startup
- the daemon does not preserve an earlier TTL just because that TTL came from the original creator

## Exclusivity

`daemon_exclusive` v1 contract:

- it is not part of session identity
- it defines an ownership boundary for reuse and switching
- a conflicting owner may only be evicted when it is not busy and can be safely released
- otherwise the new request must fail with a clear ownership conflict

This aligns with current behavior:

- idle owners may be replaced
- busy owners may not be evicted

Lifecycle policy does not override ownership policy. A child may be stateful for idle cleanup while
still participating in explicit ownership hand-off rules.

## Static Lifecycle Declaration

Each stdio child may declare a static lifecycle contract once at startup.

The static declaration should contain a small, generic cleanup policy surface:

- `reap_policy: "safe_idle_reap" | "stateful"`

The daemon should use `reap_policy` as the authoritative cleanup-class input.

### Reap Policies

`safe_idle_reap`

- generic disposable stdio helper
- daemon-local idle signals are sufficient for automatic reap

`stateful`

- automatic reap depends on child-reported lifecycle state
- the daemon must not infer child state from provider-specific internals
- the daemon must not send live background request probes just to discover current cleanup state

If the child explicitly does not support static lifecycle declaration, the daemon may continue
using current generic stdio behavior as a compatibility fallback until migration is complete.

If lifecycle declaration fetch fails for another reason, the daemon should keep the session rather
than guess that `safe_idle_reap` applies.

## Dynamic Lifecycle Snapshot

For `stateful` workers, the child should push lifecycle state changes to the daemon as
notifications.

The daemon should cache only a compressed, generic lifecycle snapshot. It should not need to
understand child-specific state machines such as browser presentation mode, bootstrap mode, or
auth internals.

Suggested fields are:

- `auto_reap_allowed: boolean`
- `retention_reason: "interactive" | "waiting_for_human" | "external_resource" | "active_runtime" | null`
- `retry_after_secs: number | null`
- `updated_at_unix: number`

The child should send:

- an initial snapshot once the session is initialized
- an updated snapshot whenever the effective auto-reap decision changes

For `stateful` workers, the dynamic lifecycle snapshot is the primary runtime-side input for
automatic reap decisions.

## Idle Semantics

`idle_ttl_secs` is a session-level cleanup policy.

V1 rules:

- the latest request updates session idle TTL on reuse
- `idle_ttl_secs == 0` means disable idle reap for that session
- idle time only begins when the daemon no longer sees the session as busy or subscription-held

This means idle TTL should not be interpreted as:

- a maximum session lifetime
- a time limit that runs while the daemon still sees the session as busy or subscription-held

It is specifically a daemon-side idle cleanup window.

## Idle Reap

Idle reap is allowed only when all of the following are true:

1. the session is not subscription-held from the daemon's point of view
2. the session is not currently busy from the daemon's point of view
3. `idle_ttl_secs != 0`
4. idle time exceeds the configured cutoff
5. the lifecycle policy allows removal

Lifecycle policy is evaluated as follows:

- `safe_idle_reap`
  - daemon-local idle signals are sufficient
- `stateful`
  - the daemon must consult the latest cached lifecycle snapshot
  - automatic reap is allowed only when `auto_reap_allowed == true`
  - if no lifecycle snapshot is available, the daemon should keep the session rather than guess

The daemon must not perform synchronous request-path probing to decide whether an idle stdio session
may be reaped.

## Child Exit And Terminal State

If the underlying runtime has become permanently unusable, the child should not remain indefinitely
alive in a misleading "ready" state.

For `stateful` workers, the child should:

- proactively update lifecycle state when retention conditions change
- proactively exit when the owner session is terminally gone and the process can no longer provide
  meaningful service

The daemon should still treat child process exit as the final authoritative terminal signal.

## Subscriptions

Subscriptions should be treated as a daemon-known retention signal, not just as an incidental side
effect.

V1 contract:

- a daemon-backed subscription keeps its underlying session non-idle while the subscription is alive
- normal calls may reuse the same session only if identity matches and ownership rules permit it
- a subscription-held session is not idle-reap eligible until the subscription stops

This preserves the existing reuse surface while staying close to the current implementation.

## Observable Session State

`uxc daemon sessions` is the user-facing view of this contract.

V1 observable guarantees should include:

- an opaque session id / display key
- endpoint
- protocol / transport family
- `link_name` as last presentation hint
- source link metadata such as `link_skill`, `link_skill_doc`, and `link_skill_path` when present
- `idle_ttl_secs`
- `expires_in_secs`
- `daemon_exclusive`
- `in_flight_requests`
- `reuse_eligible`
- `lifecycle_contract`
- `last_lifecycle_update_at_unix`
- `last_lifecycle_snapshot`

Future lifecycle work may add more explicit retention-oriented fields, for example:

- attachment count
- attachment kinds
- pinned reason

so users and generated clients can distinguish:

- idle but retained
- daemon-retained and therefore not reapable
- busy and therefore not reclaimable

## Compatibility Notes

This contract intentionally keeps current stdio identity stable.

That avoids unnecessary breakage for:

- link-backed MCP stdio reuse
- direct stdio invocation reuse
- current daemon-exclusive workflows
- current `daemon sessions` tooling

The main lifecycle change is:

- move from live probe-driven runtime cleanup hints
- to static lifecycle declaration plus pushed lifecycle state plus local daemon state

This note supersedes the older assumption that live runtime probes should be the default
runtime-side guard for stdio idle cleanup.

## Follow-Up Work

The follow-up issues under `#337` remain useful as implementation slices:

- `#340`: session identity and reuse rules
- `#341`: ownership / idle-reap semantics
- `#342`: observable daemon session state contract

Additional lifecycle follow-up from the browser-worker path:

- `#368`: stateful stdio worker cleanup contract

## V1 Follow-Through Decisions

- `daemon_exclusive` remains an ownership rule, not a lifecycle policy.
- `stateful` workers must proactively report lifecycle changes; the daemon should not poll them on
  the request path.
- `uxc` should rely on lifecycle declaration plus pushed snapshots, not live runtime probes.
