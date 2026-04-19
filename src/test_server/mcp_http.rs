//! MCP HTTP test server for E2E testing

use super::common::{bind_available, write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::signal::ctrl_c;
use tracing::info;

#[derive(Clone)]
struct ServerState {
    scenario: Scenario,
    tools_list_calls: Arc<AtomicU64>,
    resource_subscribed: Arc<AtomicBool>,
    resource_event_seq: Arc<AtomicU64>,
    resource_read_failed_once: Arc<AtomicBool>,
    next_session_id: Arc<AtomicU64>,
    session_resources: Arc<Mutex<std::collections::HashMap<String, SessionState>>>,
}

#[derive(Debug, Clone, Default)]
struct SessionState {
    initialized_acked: bool,
    subscribed: bool,
    value: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    // `id` is absent for JSON-RPC notifications (e.g. `notifications/initialized`).
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

impl JsonRpcRequest {
    fn id_or_null(&self) -> Value {
        self.id.clone().unwrap_or(Value::Null)
    }
}

async fn mcp_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Result<Response, StatusCode> {
    if req.jsonrpc != "2.0" {
        return Ok(Json(json!({
            "jsonrpc": "2.0",
            "id": req.id_or_null(),
            "error": {"code": -32600, "message": "Invalid Request"}
        }))
        .into_response());
    }

    // JSON-RPC notifications (no id) receive no response body. This matches
    // real MCP servers which typically respond 202 Accepted to
    // `notifications/initialized` and friends.
    let Some(req_id) = req.id.clone() else {
        if req.method == "notifications/initialized"
            && matches!(state.scenario, Scenario::RequiresInitializedAck)
        {
            let session_id = headers
                .get("mcp-session-id")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string())
                .ok_or(StatusCode::BAD_REQUEST)?;
            let mut sessions = state
                .session_resources
                .lock()
                .expect("session resource map");
            let Some(entry) = sessions.get_mut(&session_id) else {
                return Err(StatusCode::BAD_REQUEST);
            };
            entry.initialized_acked = true;
        }
        return Ok(StatusCode::ACCEPTED.into_response());
    };

    if matches!(state.scenario, Scenario::AuthRequired) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    if matches!(state.scenario, Scenario::Timeout) {
        tokio::time::sleep(super::common::timeout_duration()).await;
        return Err(StatusCode::REQUEST_TIMEOUT);
    }

    if matches!(state.scenario, Scenario::Malformed) && req.method == "tools/call" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body("{not-json".into())
            .expect("build malformed response"));
    }

    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());

    // Spec-compliant MCP servers refuse any request other than `initialize`
    // until the client has sent `notifications/initialized` (MCP lifecycle
    // spec 2025-03-26). When this scenario is active, mimic that behaviour so
    // tests can catch a client that forgets the post-init ack.
    if matches!(state.scenario, Scenario::RequiresInitializedAck) && req.method != "initialize" {
        let Some(session_id) = session_id.as_ref() else {
            return Err(StatusCode::BAD_REQUEST);
        };
        let sessions = state
            .session_resources
            .lock()
            .expect("session resource map");
        let Some(entry) = sessions.get(session_id) else {
            return Err(StatusCode::BAD_REQUEST);
        };
        if !entry.initialized_acked {
            return Ok(Json(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32002, "message": "Server not initialized"}
            }))
            .into_response());
        }
    }

    let result = match req.method.as_str() {
        "initialize" => {
            let session_id = if matches!(state.scenario, Scenario::RequiresInitializedAck) {
                Some(ensure_session_id(session_id.clone(), &state))
            } else {
                session_id.clone()
            };
            return with_session_response(
                req_id.clone(),
                session_id,
                &state,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {"listChanged": false},
                        "resources": {"subscribe": true}
                    },
                    "serverInfo": {
                        "name": "uxc-test-mcp-http",
                        "version": "1.0.0"
                    },
                    "instructions": "MCP HTTP test server for local e2e"
                }),
            );
        }
        "tools/list" => {
            if matches!(state.scenario, Scenario::SessionScopedResource) {
                return with_session_response(
                    req_id.clone(),
                    session_id.clone(),
                    &state,
                    json!({
                        "tools": [
                            {
                                "name": "set_resource",
                                "description": "Set the current resource value for this MCP session",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "value": {"type": "integer"}
                                    },
                                    "required": ["value"]
                                }
                            }
                        ]
                    }),
                );
            }
            let calls = state
                .tools_list_calls
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            if matches!(state.scenario, Scenario::ToolsListFailAfterFirst) && calls > 1 {
                return Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": req_id.clone(),
                    "error": {"code": -32002, "message": "tools/list failed after first request"}
                }))
                .into_response());
            }
            json!({
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo text back",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": {"type": "string", "description": "text to echo"}
                            },
                            "required": ["message"]
                        }
                    }
                ]
            })
        }
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let message = req
                .params
                .get("arguments")
                .and_then(|v| v.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("hello");
            if matches!(state.scenario, Scenario::ToolStructuredError) {
                return Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": req_id.clone(),
                    "error": {
                        "code": -32010,
                        "message": "Failed to call tool 'gemini.image.download'",
                        "data": {
                            "code": "IMAGE_DOWNLOAD_CAPTURE_FAILED",
                            "message": "Images are visible but download capture failed",
                            "details": {
                                "conversation_url": "https://gemini.google.com/app/mock-conversation",
                                "visible_image_count": 2,
                                "visible_image_urls": [
                                    "https://example.com/img-1.png",
                                    "https://example.com/img-2.png"
                                ],
                                "current_mode": "images",
                                "download_buttons_present": true,
                                "failure_cause": "download_button_click_did_not_trigger_network"
                            }
                        }
                    }
                }))
                .into_response());
            }
            if matches!(state.scenario, Scenario::SessionScopedResource) {
                if name != "set_resource" {
                    return Ok(Json(json!({
                        "jsonrpc": "2.0",
                        "id": req_id.clone(),
                        "error": {"code": -32601, "message": "Tool not found"}
                    }))
                    .into_response());
                }
                let session_id = ensure_session_id(session_id.clone(), &state);
                let value = req
                    .params
                    .get("arguments")
                    .and_then(|v| v.get("value"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                {
                    let mut sessions = state
                        .session_resources
                        .lock()
                        .expect("session resource map");
                    let entry = sessions.entry(session_id.clone()).or_default();
                    entry.value = value;
                }
                return with_session_response(
                    req_id.clone(),
                    Some(session_id),
                    &state,
                    json!({
                        "content": [
                            {"type": "text", "text": format!("resource={value}")}
                        ],
                        "structuredContent": {
                            "value": value
                        }
                    }),
                );
            }
            if name != "echo" {
                return Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": req_id.clone(),
                    "error": {"code": -32601, "message": "Tool not found"}
                }))
                .into_response());
            }
            let mut result = json!({
                "content": [
                    {"type": "text", "text": message}
                ]
            });
            if matches!(state.scenario, Scenario::StructuredContent) {
                result["structuredContent"] = json!({
                    "message": message,
                    "source": "mcp-http",
                    "length": message.len()
                });
            }
            result
        }
        "resources/subscribe" => {
            if matches!(state.scenario, Scenario::SessionScopedResource) {
                let session_id = ensure_session_id(session_id.clone(), &state);
                {
                    let mut sessions = state
                        .session_resources
                        .lock()
                        .expect("session resource map");
                    sessions.entry(session_id.clone()).or_default().subscribed = true;
                }
                return with_session_response(req_id.clone(), Some(session_id), &state, json!({}));
            }
            state.resource_subscribed.store(true, Ordering::SeqCst);
            state.resource_event_seq.store(0, Ordering::SeqCst);
            json!({})
        }
        "resources/read" => {
            let uri = req
                .params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or("test://resource");
            if matches!(state.scenario, Scenario::ResourceReadFailOnce)
                && !state.resource_read_failed_once.swap(true, Ordering::SeqCst)
            {
                return Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": req_id.clone(),
                    "error": {"code": -32003, "message": "resource read failed once"}
                }))
                .into_response());
            }
            let value = if matches!(state.scenario, Scenario::SessionScopedResource) {
                let session_id = ensure_session_id(session_id.clone(), &state);
                let sessions = state
                    .session_resources
                    .lock()
                    .expect("session resource map");
                sessions
                    .get(&session_id)
                    .map(|entry| entry.value)
                    .unwrap_or_default()
            } else {
                state.resource_event_seq.load(Ordering::SeqCst)
            };
            json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": json!({
                        "uri": uri,
                        "value": value
                    })
                    .to_string()
                }]
            })
        }
        "resources/unsubscribe" => {
            if matches!(state.scenario, Scenario::SessionScopedResource) {
                let session_id = ensure_session_id(session_id.clone(), &state);
                {
                    if let Ok(mut sessions) = state.session_resources.lock() {
                        sessions.entry(session_id.clone()).or_default().subscribed = false;
                    }
                }
                return with_session_response(req_id.clone(), Some(session_id), &state, json!({}));
            }
            state.resource_subscribed.store(false, Ordering::SeqCst);
            state.resource_event_seq.store(0, Ordering::SeqCst);
            json!({})
        }
        _ => {
            return Ok(Json(json!({
                "jsonrpc": "2.0",
                "id": req_id.clone(),
                "error": {"code": -32601, "message": "Method not found"}
            }))
            .into_response())
        }
    };

    with_session_response(req_id, session_id, &state, result)
}

async fn mcp_event_stream(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    if matches!(state.scenario, Scenario::AuthRequired) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let resource_subscribed = state.resource_subscribed.clone();
    let resource_event_seq = state.resource_event_seq.clone();
    let session_resources = state.session_resources.clone();
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let stream = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        std::time::Duration::from_millis(200),
    ))
    .filter_map(move |_| {
        let resource_subscribed = resource_subscribed.clone();
        let resource_event_seq = resource_event_seq.clone();
        let session_resources = session_resources.clone();
        let session_id = session_id.clone();
        let scenario = state.scenario;
        async move {
            if matches!(scenario, Scenario::SessionScopedResource) {
                let session_id = session_id.clone()?;
                let value = {
                    let sessions = session_resources.lock().expect("session resource map");
                    let entry = sessions.get(&session_id)?;
                    if !entry.subscribed {
                        return None;
                    }
                    entry.value
                };
                let payload = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/resources/updated",
                    "params": {
                        "uri": "test://resource",
                        "value": value
                    }
                });
                return Some(Ok(Event::default()
                    .event("message")
                    .data(payload.to_string())));
            }
            if !resource_subscribed.load(Ordering::SeqCst) {
                return None;
            }
            let seq = resource_event_seq
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            let payload = json!({
                "jsonrpc": "2.0",
                "method": "notifications/resources/updated",
                "params": {
                    "uri": "test://resource",
                    "value": seq
                }
            });
            Some(Ok(Event::default()
                .event("message")
                .data(payload.to_string())))
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_millis(250))))
}

fn ensure_session_id(existing: Option<String>, state: &ServerState) -> String {
    let session_id = existing.unwrap_or_else(|| {
        format!(
            "test-session-{}",
            state.next_session_id.fetch_add(1, Ordering::SeqCst)
        )
    });
    if let Ok(mut sessions) = state.session_resources.lock() {
        sessions.entry(session_id.clone()).or_default();
    }
    session_id
}

fn with_session_response(
    id: Value,
    session_id: Option<String>,
    state: &ServerState,
    result: Value,
) -> Result<Response, StatusCode> {
    let session_id = if matches!(
        state.scenario,
        Scenario::SessionScopedResource | Scenario::RequiresInitializedAck
    ) {
        Some(ensure_session_id(session_id, state))
    } else {
        session_id
    };
    let mut response = Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
    .into_response();
    if let Some(session_id) = session_id {
        let header_value =
            HeaderValue::from_str(&session_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        response
            .headers_mut()
            .insert("mcp-session-id", header_value);
    }
    Ok(response)
}

fn create_router(state: ServerState) -> Router {
    async fn not_found() -> StatusCode {
        StatusCode::NOT_FOUND
    }

    Router::new()
        .route("/", get(not_found))
        .route("/mcp", get(mcp_event_stream).post(mcp_handler))
        .route("/.well-known/mcp", post(mcp_handler))
        .with_state(state)
}

pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let (listener, addr) = bind_available().await?;
    let app = create_router(ServerState {
        scenario,
        tools_list_calls: Arc::new(AtomicU64::new(0)),
        resource_subscribed: Arc::new(AtomicBool::new(false)),
        resource_event_seq: Arc::new(AtomicU64::new(0)),
        resource_read_failed_once: Arc::new(AtomicBool::new(false)),
        next_session_id: Arc::new(AtomicU64::new(1)),
        session_resources: Arc::new(Mutex::new(std::collections::HashMap::new())),
    });

    info!("MCP HTTP test server listening on http://{}", addr);
    write_addr_file(addr, "mcp-http")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_rx.await.ok();
        info!("MCP HTTP test server shutting down");
    });

    tokio::spawn(async move {
        if let Err(err) = server.await {
            tracing::error!("MCP HTTP test server failed: {}", err);
        }
    });

    Ok(ServerHandle {
        addr,
        shutdown: shutdown_tx,
    })
}

pub async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = if args.len() > 1 {
        Scenario::from_str(&args[1])?
    } else {
        Scenario::Ok
    };

    tracing_subscriber::fmt()
        .with_env_filter("uxc_test_server=info,axum=info")
        .init();

    let handle = run(scenario).await?;

    ctrl_c().await?;
    let _ = handle.shutdown.send(());

    Ok(())
}
