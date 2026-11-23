# MCP

For an in-process Rust integration, see the [embedded MCP client](./embedded-mcp.md).

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
| MCP Events discovery and polling (SEP-3415) | Supported when advertised | Only if advertised by the server |
| Multi-round tool results | Explicit capabilities and continuation input | Results normalized as complete |
| OAuth for MCP HTTP | Supported | Supported |

## Stateless Request Metadata

For `2026-07-28`, UXC sends the negotiated protocol version in both:

- `MCP-Protocol-Version`
- `params._meta["io.modelcontextprotocol/protocolVersion"]`

Each request also includes client information and client capabilities. UXC does
not claim roots, sampling, or elicitation support by default. For Events calls,
it declares the Events extension unless the caller provides `--mcp-capabilities`.

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

### MCP Events (SEP-3415)

When the server advertises `io.modelcontextprotocol/events`, UXC exposes
`events/list` and `events/poll` as endpoint operations. Use the event descriptor
to check that `delivery` includes `poll` and to find the accepted arguments
(`inputSchema`) and payload shape (`payloadSchema`).

```bash
uxc "$MCP_ENDPOINT" events/list
uxc "$MCP_ENDPOINT" events/list "cursor=$NEXT_PAGE_CURSOR"

uxc "$MCP_ENDPOINT" events/poll name=weather.reading arguments.station=sfo cursor:=null
uxc "$MCP_ENDPOINT" events/poll name=weather.reading arguments.station=sfo "cursor=$POLL_CURSOR"
```

- `events/list` returns one page. Pass `data.nextCursor` as the `cursor` for
  the next page if it is present.
- Omit the poll cursor or pass `cursor:=null` to start from the server's
  current position. To resume, pass the previous `data.cursor` unchanged with
  the same event name and arguments, even in a new CLI process. List and poll
  cursors are distinct and opaque.
- Each call sends a single `events/poll` request. UXC does not schedule another
  poll or save a checkpoint. The caller decides what to do with `hasMore` and
  `nextPollMs`. If `truncated` is `true`, the server reports a gap in replay;
  UXC returns the flag and the updated cursor along with the server's event
  payloads and metadata.
- On modern Streamable HTTP, UXC sends `Mcp-Name` and any `Mcp-Param-*` headers
  defined by the event's `x-mcp-header` annotations. UXC may read the event
  catalog before polling to find these annotations.

UXC reports an error if the extension is not advertised. If a server rejects
polling for a webhook-only event, the CLI retains the server's `-32026` JSON-RPC
error details. This support does not include push streams or webhook
subscriptions and is separate from `subscriptions/listen`.

### Other extensions

UXC does not currently implement Tasks, Client ID Metadata Documents, or
automatic roots, sampling, and elicitation providers. Unknown metadata and
extension fields are preserved where the CLI contract permits.
