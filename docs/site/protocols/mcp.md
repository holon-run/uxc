# MCP

UXC supports MCP servers over Streamable HTTP and stdio while preserving
compatibility across the stateful and stateless protocol eras.

## Version Support

| Protocol | HTTP | stdio | Lifecycle |
| --- | --- | --- | --- |
| `2026-07-28` | Supported | Supported | Stateless requests with `server/discover` and per-request metadata |
| `2024-11-05` | Supported | Supported | Legacy `initialize` / `notifications/initialized` session |
| Legacy HTTP+SSE | Supported | Not applicable | Existing SSE endpoint and session behavior |

UXC probes the server and selects the modern or legacy path automatically.
Modern HTTP requests do not send `Mcp-Session-Id` and do not open a separate
GET event stream.

## Feature Matrix

| Capability | `2026-07-28` | Legacy |
| --- | --- | --- |
| Tools list and call | Supported | Supported |
| Resources list and read | Supported | Supported |
| Prompts list and get | Supported | Supported |
| Pagination and cache hints | Supported | Existing UXC cache policy |
| Custom MCP request headers | Supported | Not injected into legacy requests |
| `subscriptions/listen` | Supported through daemon-backed flows | Legacy resource subscriptions retained |
| Multi-round tool results | Explicit capabilities and continuation input | Results normalized as complete |
| OAuth for MCP HTTP | Supported | Supported |

## Stateless Request Metadata

For `2026-07-28`, UXC sends the negotiated protocol version in both:

- `MCP-Protocol-Version`
- `params._meta["io.modelcontextprotocol/protocolVersion"]`

Each request also includes client information and client capabilities. UXC
declares only capabilities supplied by the caller instead of claiming roots,
sampling, or elicitation support by default.

## Multi-Round Tool Results

UXC returns `input_required` results without automatically invoking a model or
prompting a user. Callers can explicitly provide:

```bash
uxc --mcp-capabilities @capabilities.json <endpoint> <tool> ...
uxc --mcp-continuation @continuation.json <endpoint> <tool> ...
```

Continuation state is treated as opaque protocol data.

## Subscriptions

Modern servers use the long-lived `subscriptions/listen` request. UXC keeps
that request open through the daemon, filters notifications by subscription
ID, and reopens it after a transport disconnect. It does not use
`Last-Event-ID` or protocol session resumption for modern subscriptions.

## OAuth Security

Authorization-code callbacks accept the RFC 9207 `iss` parameter. When it is
present, UXC requires an exact match with the discovered issuer; callbacks
without `iss` remain compatible with providers that do not advertise it.

Clients created through Dynamic Client Registration are bound to the issuer
that registered them and are not reused after an issuer change. Client ID
Metadata Documents are not implemented yet; RFC 7591 registration remains the
fallback when no client ID is provided.

## Migration from Legacy MCP

Server operators migrating to `2026-07-28` should expect:

1. `server/discover` instead of `initialize`.
2. Protocol and capability metadata on every request.
3. Independent HTTP POST requests without `Mcp-Session-Id`.
4. Request-scoped SSE only when a request streams.
5. `subscriptions/listen` instead of resource subscribe/unsubscribe methods.

UXC keeps the `2024-11-05` path for existing servers, so users do not need to
change commands when a server upgrades.

## Optional Extensions

UXC does not currently implement Tasks, Client ID Metadata Documents, or
automatic roots, sampling, and elicitation providers. Unknown metadata and
extension fields are preserved where the CLI contract permits.
