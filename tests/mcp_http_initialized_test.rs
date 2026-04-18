//! Regression test: MCP HTTP transport must send `notifications/initialized`
//! between `initialize` and any subsequent request.
//!
//! MCP lifecycle (2025-03-26) requires the client to ack the initialize
//! response before the server will service further requests on the session.
//! Spec-compliant servers (e.g., rmcp 0.15) otherwise hang on follow-up
//! requests; lenient servers (FastMCP) mask the bug by serving anyway.
//!
//! This test spins up a minimal hyper-based mock MCP HTTP server that:
//!   1. Records the JSON-RPC `method` of each incoming POST in order.
//!   2. Simulates spec-compliant behaviour by rejecting `tools/list` with a
//!      JSON-RPC error if `notifications/initialized` has not been received.
//!
//! It then exercises `McpHttpTransport` through the same sequence the adapter
//! uses (initialize → initialized → list_tools) and asserts:
//!   - `list_tools` succeeds.
//!   - `notifications/initialized` appears between `initialize` and
//!     `tools/list` in the recorded method log.

use std::convert::Infallible;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use serde_json::Value as JsonValue;
use tokio::sync::{oneshot, Mutex};

use uxc::adapters::mcp::http_transport::McpHttpTransport;
use uxc::adapters::mcp::McpAdapter;
use uxc::adapters::Adapter;

#[derive(Default, Clone)]
struct RecordingState {
    methods: Arc<Mutex<Vec<String>>>,
    initialized_acked: Arc<Mutex<bool>>,
}

async fn handle(req: Request<Body>, state: RecordingState) -> Response<Body> {
    if req.method() != Method::POST {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::empty())
            .expect("method not allowed response");
    }

    let body_bytes = hyper::body::to_bytes(req.into_body())
        .await
        .expect("read request body");
    let payload: JsonValue =
        serde_json::from_slice(&body_bytes).expect("mock server got invalid JSON");

    let method = payload
        .get("method")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let id = payload.get("id").cloned();

    state.methods.lock().await.push(method.clone());

    match method.as_str() {
        "initialize" => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .header("mcp-session-id", "test-session-123")
            .body(Body::from(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "mock-mcp", "version": "0.0.0" }
                    }
                })
                .to_string(),
            ))
            .expect("initialize response"),
        "notifications/initialized" => {
            // JSON-RPC notification: no id, no response body. Per MCP spec,
            // servers typically return 202 Accepted here.
            *state.initialized_acked.lock().await = true;
            Response::builder()
                .status(StatusCode::ACCEPTED)
                .body(Body::empty())
                .expect("initialized ack response")
        }
        "tools/list" => {
            // Simulate a spec-compliant server: refuse further requests until
            // the client has acked `initialize` via `notifications/initialized`.
            if !*state.initialized_acked.lock().await {
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32002,
                                "message": "Server not initialized: missing notifications/initialized"
                            }
                        })
                        .to_string(),
                    ))
                    .expect("tools/list pre-ack error response");
            }

            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "tools": [] }
                    })
                    .to_string(),
                ))
                .expect("tools/list response")
        }
        _ => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                })
                .to_string(),
            ))
            .expect("generic response"),
    }
}

struct MockServer {
    base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
    state: RecordingState,
}

impl MockServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock mcp http server");
        let addr = listener.local_addr().expect("mock mcp http addr");
        let state = RecordingState::default();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let state_for_svc = state.clone();
        let make_svc = make_service_fn(move |_| {
            let state = state_for_svc.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let state = state.clone();
                    async move { Ok::<_, Infallible>(handle(req, state).await) }
                }))
            }
        });

        let server = Server::from_tcp(listener)
            .expect("mock mcp http server from tcp")
            .serve(make_svc)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });

        let task = tokio::spawn(async move {
            let _ = server.await;
        });

        MockServer {
            base_url: format!("http://{}", addr),
            shutdown: Some(shutdown_tx),
            task,
            state,
        }
    }

    async fn recorded_methods(&self) -> Vec<String> {
        self.state.methods.lock().await.clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        self.task.abort();
    }
}

#[tokio::test]
async fn http_transport_sends_initialized_notification_between_initialize_and_tools_list() {
    let server = MockServer::spawn().await;

    let transport = McpHttpTransport::with_auth_and_timeout(
        server.base_url.clone(),
        None,
        Duration::from_secs(5),
    )
    .expect("construct http transport");

    transport.initialize().await.expect("initialize succeeds");
    transport
        .initialized()
        .await
        .expect("initialized ack succeeds");
    let tools = transport.list_tools().await.expect(
        "tools/list must succeed after notifications/initialized ack; \
         failure here means the ack was not sent",
    );
    assert!(tools.is_empty(), "mock server returns an empty tools list");

    let methods = server.recorded_methods().await;
    assert!(
        methods.len() >= 3,
        "expected at least 3 recorded methods, got {:?}",
        methods
    );

    // The critical assertion: `notifications/initialized` must appear
    // between `initialize` and `tools/list`. Without it, spec-compliant MCP
    // servers (e.g., rmcp 0.15) hang on subsequent requests.
    let init_idx = methods
        .iter()
        .position(|m| m == "initialize")
        .expect("initialize must be recorded");
    let ack_idx = methods
        .iter()
        .position(|m| m == "notifications/initialized")
        .expect("notifications/initialized must be sent after initialize");
    let list_idx = methods
        .iter()
        .position(|m| m == "tools/list")
        .expect("tools/list must be recorded");

    assert!(
        init_idx < ack_idx && ack_idx < list_idx,
        "notifications/initialized must be sent after initialize and before tools/list; got {:?}",
        methods
    );
}

#[tokio::test]
async fn http_transport_initialized_notification_carries_session_id() {
    // Explicitly assert that the `notifications/initialized` request carries
    // the `mcp-session-id` header returned by the initialize response.
    //
    // Without this header, a multi-session server would reject the ack.
    use hyper::header::HeaderValue;
    use std::sync::Mutex as StdMutex;

    #[derive(Default, Clone)]
    struct HeaderState {
        captured_session_id: Arc<StdMutex<Option<String>>>,
    }

    async fn header_handle(req: Request<Body>, state: HeaderState) -> Response<Body> {
        let method_header = req
            .headers()
            .get("mcp-session-id")
            .cloned()
            .unwrap_or_else(|| HeaderValue::from_static(""));

        let body_bytes = hyper::body::to_bytes(req.into_body())
            .await
            .expect("read body");
        let payload: JsonValue = serde_json::from_slice(&body_bytes).expect("valid JSON");
        let method = payload
            .get("method")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
        let id = payload.get("id").cloned();

        if method == "notifications/initialized" {
            *state.captured_session_id.lock().unwrap() =
                method_header.to_str().ok().map(|s| s.to_string());
            return Response::builder()
                .status(StatusCode::ACCEPTED)
                .body(Body::empty())
                .expect("accepted");
        }

        // initialize
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .header("mcp-session-id", "session-abc-xyz")
            .body(Body::from(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "serverInfo": { "name": "mock", "version": "0.0.0" }
                    }
                })
                .to_string(),
            ))
            .expect("init response")
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let state = HeaderState::default();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state_for_svc = state.clone();
    let make_svc = make_service_fn(move |_| {
        let state = state_for_svc.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let state = state.clone();
                async move { Ok::<_, Infallible>(header_handle(req, state).await) }
            }))
        }
    });
    let server = Server::from_tcp(listener)
        .unwrap()
        .serve(make_svc)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
    let task = tokio::spawn(async move {
        let _ = server.await;
    });

    let base_url = format!("http://{}", addr);
    let transport =
        McpHttpTransport::with_auth_and_timeout(base_url, None, Duration::from_secs(5)).unwrap();
    transport.initialize().await.unwrap();
    transport.initialized().await.unwrap();

    let captured = state.captured_session_id.lock().unwrap().clone();
    assert_eq!(
        captured.as_deref(),
        Some("session-abc-xyz"),
        "initialized notification must carry the session id from the initialize response"
    );

    let _ = shutdown_tx.send(());
    task.abort();
}

/// End-to-end regression test through the adapter layer.
///
/// This exercises the exact call sequence in
/// `src/adapters/mcp/mod.rs::fetch_schema_internal` for HTTP MCP endpoints
/// (initialize → initialized → tools/list) and uses a spec-compliant mock
/// server that refuses `tools/list` until the ack has been received. If the
/// adapter ever regresses to calling `initialize` without the follow-up ack,
/// the mock server will return a JSON-RPC error and `fetch_schema` will fail.
#[tokio::test]
async fn adapter_fetch_schema_sends_initialized_ack_after_initialize() {
    let server = MockServer::spawn().await;

    let adapter = McpAdapter::new();
    let schema = adapter
        .fetch_schema(&server.base_url)
        .await
        .expect("fetch_schema must succeed against spec-compliant server");

    // Sanity-check the schema payload.
    assert_eq!(
        schema.get("protocol").and_then(JsonValue::as_str),
        Some("MCP")
    );

    // And confirm the ack was observed between initialize and tools/list.
    let methods = server.recorded_methods().await;
    let init_idx = methods
        .iter()
        .position(|m| m == "initialize")
        .expect("initialize must be recorded");
    let ack_idx = methods
        .iter()
        .position(|m| m == "notifications/initialized")
        .expect("notifications/initialized must be sent by the adapter");
    let list_idx = methods
        .iter()
        .position(|m| m == "tools/list")
        .expect("tools/list must be recorded");
    assert!(
        init_idx < ack_idx && ack_idx < list_idx,
        "adapter must send notifications/initialized between initialize and tools/list; got {:?}",
        methods
    );
}
