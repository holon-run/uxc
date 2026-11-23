# Embedded Rust MCP client

Rust applications can depend on `uxc` and use `uxc::mcp` in process on a Tokio
runtime. No UXC CLI executable or UXC daemon is needed. `uxc::mcp` is the
supported embedding surface; existing adapter, CLI and daemon exports are not
part of that compatibility promise.

## Connect and discover

Configure `McpConnectOptions` with `McpEndpoint::Stdio` (program, literal
arguments, and optional child environment overrides) or
`McpEndpoint::StreamableHttp` (complete URL and explicit auth choice). Only
launch trusted local programs: stdio inherits the process environment, and UXC
does not sandbox the child.

```rust
use uxc::mcp::{McpClient, McpConnectOptions, McpEndpoint, McpError, McpHttpAuth};

async fn inspect(url: String) -> Result<(), McpError> {
    let mut client = McpClient::connect(McpConnectOptions::new(
        McpEndpoint::StreamableHttp { url, auth: McpHttpAuth::Anonymous },
    )).await?;
    let result = async {
        let discovery = client.discover().await?;
        println!("Negotiated {}", discovery.protocol_version);
        for tool in client.list_tools().await? {
            println!("{}", tool.name);
        }
        Ok::<_, McpError>(())
    }.await;
    let closed = client.close().await;
    result?;
    closed
}
```

From a UXC checkout, run the independent Cargo consumer against the repository's
stdio fixture:

```sh
cargo build --locked --features test-server --bin uxc-test-mcp-stdio-server
CARGO_TARGET_DIR="$PWD/target" cargo run --manifest-path examples/embedded-mcp/Cargo.toml -- \
  "$PWD/target/debug/uxc-test-mcp-stdio-server"
```

## Authentication and session lifetime

- `Anonymous` never reads UXC authentication settings. To authenticate, select
  `McpHttpAuth::Profile("my-profile".into())`. UXC reads only the named credential,
  not the endpoint's host binding. Set up credentials separately. Configured
  refresh can run before requests; the library does not launch interactive OAuth
  login or resend an MCP RPC after HTTP 401.
- Reuse one client serially with `&mut self`. Discovery is a snapshot of the
  established handshake, not a liveness check. The client tries modern
  Streamable HTTP discovery, and can negotiate legacy Streamable HTTP on the
  same URL. It does not follow redirects or switch to the separate legacy SSE
  transport. SSE-framed responses to normal Streamable HTTP POSTs are supported.
- The defaults are 30 seconds to connect, 30 seconds per request, and 5 seconds
  to close. Configure these independently with `McpTimeouts`. The request budget
  includes all automatic `tools/list` pages and any Events routing lookups.
  Error variants are typed (`McpError`); they do not retain raw diagnostics,
  credentials, URLs, or response bodies. Protocol errors can expose the numeric
  JSON-RPC code. After timeout, disconnection, or cancellation of an in-flight
  request, explicitly close and reconnect; there is no automatic replay.
- Await `close` even if a request failed. Stdio closes stdin, then terminates and
  reaps the direct child if needed; descendant processes are not managed.
  Stateful HTTP attempts one authenticated DELETE. A 404 (already gone) or 405
  (termination not supported) is accepted. Local resources are released even
  on close failure; a failed handshake or a dropped/expired close future does
  not guarantee remote termination. `Drop` alone is best effort.

## Events: caller-owned scheduling and cursors

Check `discover().await?.capabilities.events`. `list_events_page` returns a
single catalog page. Pass `next_cursor` into the next listing if you want
another page. `poll_events_once` sends one poll and returns event data, the
returned checkpoint `cursor`, and scheduling hints; it does not poll again.
An absent capability returns `McpError::UnsupportedCapability` without sending
an Events RPC.

For example, Holon can load a stored checkpoint, pass it via `PollEventsParams`,
and persist the returned `cursor` only after recording the events durably and
applying its own deduplication and gap policy. The page cursor and poll checkpoint
are separate opaque values. `has_more` and `next_poll_ms` are hints for Holon's
scheduler, not instructions to UXC. The client does not store cursors,
acknowledge events, subscribe, or call arbitrary tools/resources/prompts.
