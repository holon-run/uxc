use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use serde_json::{json, Value};
use serial_test::serial;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uxc::mcp::{
    ListEventsParams, McpClient, McpConnectOptions, McpEndpoint, McpError, McpHttpAuth,
    PollEventsParams,
};

const SECRET: &str = "sensitive-fixture-value";
const SESSION: &str = "fixture-session";
const EXTENSION: &str = "io.modelcontextprotocol/events";

struct Fixture {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy, Default)]
struct Behavior {
    modern: bool,
    events: bool,
    authenticated: bool,
    non_mcp: bool,
    connect_delay: bool,
    expired: bool,
    timeout_catalog: bool,
    delete_status: Option<u16>,
    delete_delay: bool,
    reject_poll: bool,
}

impl Fixture {
    fn start(behavior: Behavior) -> Self {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let state = seen.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "http://{}/mcp?token={SECRET}",
            listener.local_addr().unwrap()
        );
        let server = Server::from_tcp(listener)
            .unwrap()
            .serve(make_service_fn(move |_| {
                let state = state.clone();
                async move {
                    Ok::<_, Infallible>(service_fn(move |request| {
                        let state = state.clone();
                        async move { Ok::<_, Infallible>(reply(request, state, behavior).await) }
                    }))
                }
            }));
        let task = tokio::spawn(async move { server.await.unwrap() });
        Self { url, seen, task }
    }

    fn options(&self, auth: McpHttpAuth) -> McpConnectOptions {
        McpConnectOptions::new(McpEndpoint::StreamableHttp {
            url: self.url.clone(),
            auth,
        })
    }

    fn methods(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

fn response(status: u16, body: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(body.into())
        .unwrap()
}

async fn reply(
    request: Request<Body>,
    seen: Arc<Mutex<Vec<String>>>,
    behavior: Behavior,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let body = hyper::body::to_bytes(body).await.unwrap();
    let rpc: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let method = rpc["method"].as_str().unwrap_or(parts.method.as_str());
    seen.lock().unwrap().push(method.to_owned());

    let authorized = parts
        .headers
        .get("authorization")
        .and_then(|h| h.to_str().ok());
    if behavior.authenticated && authorized != Some("Bearer sensitive-fixture-value") {
        return Response::builder()
            .status(401)
            .header(
                "www-authenticate",
                format!("Bearer resource_metadata=\"https://example.invalid/{SECRET}\""),
            )
            .body(Body::from(SECRET))
            .unwrap();
    }
    if behavior.reject_poll && method == "events/poll" {
        return response(401, SECRET);
    }
    if behavior.non_mcp {
        return response(200, format!("<html>{SECRET}</html>"));
    }
    if behavior.connect_delay && method == "server/discover" {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    if !behavior.modern && !matches!(method, "server/discover" | "initialize") {
        if parts
            .headers
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            != Some(SESSION)
            || parts
                .headers
                .get("mcp-protocol-version")
                .and_then(|v| v.to_str().ok())
                != Some("2025-11-25")
        {
            return response(400, "missing session or version");
        }
    }
    if method == "DELETE" {
        if behavior.delete_delay {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        return response(behavior.delete_status.unwrap_or(204), "");
    }
    if method == "notifications/initialized" {
        return response(202, "");
    }
    if behavior.expired && method == "tools/list" {
        return response(404, SECRET);
    }
    let mut capabilities = json!({"tools": {}});
    if behavior.events {
        capabilities["extensions"] = json!({(EXTENSION): {}});
    }
    let result = match method {
        "server/discover" if behavior.modern => json!({
            "supportedVersions": ["2026-07-28"],
            "capabilities": capabilities,
            "serverInfo": {"name": "embedded-http", "version": "1"}
        }),
        "server/discover" => {
            return response(
                200,
                json!({
                    "jsonrpc": "2.0", "id": rpc["id"],
                    "error": {"code": -32601, "message": SECRET}
                })
                .to_string(),
            );
        }
        "initialize" => json!({
            "protocolVersion": "2025-11-25",
            "capabilities": capabilities,
            "serverInfo": {"name": "embedded-http", "version": "1"}
        }),
        "tools/list" if behavior.timeout_catalog => {
            tokio::time::sleep(Duration::from_millis(250)).await;
            json!({"tools": [], "nextCursor": "next"})
        }
        "tools/list" if rpc["params"]["cursor"] == "next" => json!({"tools": [{"name": "two"}]}),
        "tools/list" => json!({"tools": [{"name": "one"}], "nextCursor": "next"}),
        "events/list" if rpc["params"]["cursor"] == "page+/=" => {
            json!({"events": [{"name": "reading", "delivery": ["poll"]}]})
        }
        "events/list" => json!({"events": [], "nextCursor": "page+/="}),
        "events/poll" if rpc["params"]["cursor"] == "opaque+/=" => json!({
            "events": [{"eventId": "reading-1", "name": "reading", "data": {"value": 4}}],
            "cursor": "next+/=", "hasMore": true, "nextPollMs": 123,
        }),
        "events/poll" => {
            return response(
                200,
                json!({
                    "jsonrpc": "2.0", "id": rpc["id"],
                    "error": {"code": -32026, "message": SECRET, "data": {"secret": SECRET}}
                })
                .to_string(),
            )
        }
        _ => return response(500, "unexpected request"),
    };
    let reply = json!({"jsonrpc": "2.0", "id": rpc["id"], "result": result}).to_string();
    if method == "tools/list" {
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!("data: {reply}\n\n")))
            .unwrap();
    }
    if method == "initialize" {
        return Response::builder()
            .header("mcp-session-id", SESSION)
            .body(Body::from(reply))
            .unwrap();
    }
    response(200, reply)
}

async fn connect(fixture: &Fixture) -> McpClient {
    McpClient::connect(fixture.options(McpHttpAuth::Anonymous))
        .await
        .unwrap()
}

async fn connect_error(options: McpConnectOptions) -> McpError {
    match McpClient::connect(options).await {
        Ok(_) => panic!("unexpected successful connection"),
        Err(err) => err,
    }
}

fn verify_redacted(err: &McpError) {
    assert!(!format!("{err} {err:?}").contains(SECRET));
    assert!(std::error::Error::source(err).is_none());
}

#[tokio::test]
async fn reuses_session_and_does_not_send_unsupported_events() {
    let fixture = Fixture::start(Behavior::default());
    let mut client = connect(&fixture).await;
    let discovery = client.discover().await.unwrap();
    assert_eq!(discovery.protocol_version, "2025-11-25");
    assert_eq!(discovery.server_info.unwrap().name, "embedded-http");
    assert!(discovery.capabilities.tools);
    assert!(!discovery.capabilities.events);
    assert!(matches!(
        client.list_events_page(ListEventsParams::default()).await,
        Err(McpError::UnsupportedCapability)
    ));
    assert!(matches!(
        client
            .poll_events_once(PollEventsParams::new("reading"))
            .await,
        Err(McpError::UnsupportedCapability)
    ));
    let tools = client.list_tools().await.unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    client.close().await.unwrap();
    assert_eq!(
        fixture.methods(),
        [
            "server/discover",
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/list",
            "DELETE",
        ]
    );
}

#[tokio::test]
async fn stateless_events_use_opaque_cursors_and_poll_once() {
    let fixture = Fixture::start(Behavior {
        modern: true,
        events: true,
        ..Behavior::default()
    });
    let mut client = connect(&fixture).await;
    assert!(client.discover().await.unwrap().capabilities.events);
    let first = client
        .list_events_page(ListEventsParams::default())
        .await
        .unwrap();
    assert_eq!(first.next_cursor.as_deref(), Some("page+/="));
    assert_eq!(fixture.methods(), ["server/discover", "events/list"]);
    let next = client
        .list_events_page(ListEventsParams {
            cursor: first.next_cursor,
        })
        .await
        .unwrap();
    assert_eq!(next.events[0].name, "reading");
    let mut poll = PollEventsParams::new("reading");
    poll.cursor = Some("opaque+/=".into());
    let result = client.poll_events_once(poll).await.unwrap();
    assert_eq!(result.events[0].data, json!({"value": 4}));
    assert_eq!(result.cursor, "next+/=");
    assert!(result.has_more);
    assert_eq!(result.next_poll_ms, Some(123));
    let err = client
        .poll_events_once(PollEventsParams::new("reading"))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        McpError::Protocol {
            json_rpc_code: Some(-32026)
        }
    );
    verify_redacted(&err);
    client.close().await.unwrap();
    assert_eq!(
        fixture.methods(),
        [
            "server/discover",
            "events/list",
            "events/list",
            "events/poll",
            "events/poll"
        ]
    );
}

#[tokio::test]
async fn failed_handshake_timeout_disconnect_and_delete_fallback() {
    let non_mcp = Fixture::start(Behavior {
        non_mcp: true,
        ..Behavior::default()
    });
    let err = connect_error(non_mcp.options(McpHttpAuth::Anonymous)).await;
    assert!(matches!(err, McpError::Protocol { .. }));
    verify_redacted(&err);
    let slow = Fixture::start(Behavior {
        connect_delay: true,
        ..Behavior::default()
    });
    let mut options = slow.options(McpHttpAuth::Anonymous);
    options.timeouts.connect = Duration::from_millis(100);
    assert_eq!(connect_error(options).await, McpError::Timeout);
    let expired = Fixture::start(Behavior {
        expired: true,
        delete_status: Some(405),
        ..Behavior::default()
    });
    let mut client = connect(&expired).await;
    assert_eq!(
        client.list_tools().await.unwrap_err(),
        McpError::Disconnected
    );
    assert_eq!(
        client.list_tools().await.unwrap_err(),
        McpError::Disconnected
    );
    client.close().await.unwrap();
    assert_eq!(
        expired.methods(),
        [
            "server/discover",
            "initialize",
            "notifications/initialized",
            "tools/list",
            "DELETE"
        ]
    );
}

#[tokio::test]
async fn deadlines_cover_all_catalog_pages_and_http_close() {
    let fixture = Fixture::start(Behavior {
        timeout_catalog: true,
        delete_delay: true,
        ..Behavior::default()
    });
    let mut options = fixture.options(McpHttpAuth::Anonymous);
    options.timeouts.request = Duration::from_millis(450);
    options.timeouts.close = Duration::from_millis(100);
    let mut client = McpClient::connect(options).await.unwrap();
    assert_eq!(client.list_tools().await.unwrap_err(), McpError::Timeout);
    assert_eq!(
        client.list_tools().await.unwrap_err(),
        McpError::Disconnected
    );
    assert_eq!(client.close().await.unwrap_err(), McpError::Timeout);
    assert_eq!(
        fixture.methods(),
        [
            "server/discover",
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/list",
            "DELETE"
        ]
    );
}

fn write_bearer_credentials() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let credentials = json!({
        "version": 1,
        "credentials": {
            "fixture": {
                "auth_type": "bearer",
                "secret_source": {"kind": "literal", "value": SECRET}
            }
        }
    });
    std::fs::write(dir.path().join("credentials.json"), credentials.to_string()).unwrap();
    dir
}

#[tokio::test]
#[serial]
async fn explicit_profile_auth_is_sent_upfront_and_rejections_are_not_retried() {
    let credentials = write_bearer_credentials();
    std::env::set_var(
        "UXC_CREDENTIALS_FILE",
        credentials.path().join("credentials.json"),
    );

    let fixture = Fixture::start(Behavior {
        modern: true,
        events: true,
        authenticated: true,
        ..Behavior::default()
    });
    let mut client = McpClient::connect(fixture.options(McpHttpAuth::Profile("fixture".into())))
        .await
        .unwrap();
    assert_eq!(
        client.discover().await.unwrap().protocol_version,
        "2026-07-28"
    );

    // Rejected credentials surface as an auth error with no refresh or replay.
    let rejecting = Fixture::start(Behavior {
        modern: true,
        events: true,
        reject_poll: true,
        ..Behavior::default()
    });
    let mut rejected =
        McpClient::connect(rejecting.options(McpHttpAuth::Profile("fixture".into())))
            .await
            .unwrap();
    let err = rejected
        .poll_events_once(PollEventsParams::new("reading"))
        .await
        .unwrap_err();
    assert_eq!(err, McpError::Authentication);
    verify_redacted(&err);
    let methods = rejecting.methods();
    assert_eq!(
        methods
            .iter()
            .filter(|method| method.as_str() == "events/poll")
            .count(),
        1
    );

    client.close().await.unwrap();
    rejected.close().await.unwrap();
    std::env::remove_var("UXC_CREDENTIALS_FILE");
}
