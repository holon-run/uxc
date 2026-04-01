# Daemon Session Lifecycle Contract

This note records the lifecycle contract for issue #337.

## Summary

UXC's daemon is the stable execution surface for long-lived runtimes, especially MCP stdio
sessions. The lifecycle contract needs to explain:

- what defines a reusable session
- what defines session ownership
- what metadata may change across reuse
- when a session is considered idle
- when idle reap is allowed
- what users and generated clients may observe from `uxc daemon sessions`

This note defines a v1 contract for daemon-backed session lifecycle. It is primarily about MCP
stdio sessions, because that is where reuse, exclusivity, and reaping semantics already exist.

## Scope

This note covers:

- stdio session identity
- reuse rules
- daemon exclusivity
- idle semantics
- idle reap behavior
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

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L6183)

The current stdio session stores mutable runtime metadata such as:

- `idle_ttl_secs`
- `link_name`
- `endpoint`
- `daemon_exclusive`
- `can_reap_contract`

Implementation reference:

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L498)
- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L627)

The current cleanup path already treats stdio sessions differently from HTTP sessions and probes
`can_reap` before removing an idle stdio session.

Implementation reference:

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L1077)

MCP stdio subscriptions already reuse stdio sessions through the same `get_or_create_stdio(...)`
path, but the current lifecycle model does not define a separate attachment contract. Session
retention is still expressed through daemon-known busy/subscription state plus runtime-reported
`can_reap`.

Implementation reference:

- [src/daemon.rs](/Users/jolestar/opensource/src/github.com/holon-run/uxc/src/daemon.rs#L4983)

## Model

The lifecycle model is split into three layers.

### 1. Session Identity

Session identity answers:

- does this request refer to the same runtime session?

For stdio, v1 identity is:

- endpoint
- auth fingerprint
- injected env fingerprint
- transport/runtime family

For current implementation, this is equivalent to the current stdio session key contract.

Identity does not include:

- `link_name`
- `daemon_idle_ttl`
- `daemon_exclusive`

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

### 3. Presentation and Cleanup Hints

Presentation and cleanup hints answer:

- how should this session be shown to users?
- how should this session be cleaned up when it becomes idle?

For v1, these include:

- `link_name`
- `idle_ttl_secs`
- future source metadata such as skill/docs origin

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
5. `can_reap` allows removal, or the runtime does not support `can_reap` and the daemon falls back
   to current best-effort cleanup rules

For MCP stdio sessions, `can_reap` is the runtime-side guard after daemon-side idleness. It is not
the primary definition of idleness.

### Deferred Reap

When `can_reap` says the session should be kept alive:

- the daemon must not remove the session
- the daemon should record the observed `can_reap_contract`
- the daemon may retry after the indicated delay or a default retry interval

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
- `idle_ttl_secs`
- `expires_in_secs`
- `daemon_exclusive`
- `in_flight_requests`
- `reuse_eligible`
- `can_reap_contract`

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

The main change is not the identity key itself. The main change is making ownership and idle/reap
semantics explicit instead of leaving them implicit in implementation details.

## Follow-Up Work

The follow-up issues under #337 remain useful as implementation slices:

- `#340`: session identity and reuse rules
- `#341`: ownership / idle-reap semantics
- `#342`: observable daemon session state contract

## Open Questions

- Should `daemon_exclusive` conflicts surface richer observable state in `daemon sessions`?
- Should subscription-held state be exposed directly, or summarized into a pinned-state view?
