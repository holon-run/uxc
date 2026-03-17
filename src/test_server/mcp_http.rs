//! MCP HTTP test server for E2E testing

use super::common::{bind_available, write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
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
    Arc,
};
use tokio::signal::ctrl_c;
use tracing::info;

#[derive(Clone)]
struct ServerState {
    scenario: Scenario,
    tools_list_calls: Arc<AtomicU64>,
    resource_subscribed: Arc<AtomicBool>,
    resource_event_seq: Arc<AtomicU64>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

async fn mcp_handler(
    State(state): State<ServerState>,
    Json(req): Json<JsonRpcRequest>,
) -> Result<Response, StatusCode> {
    if req.jsonrpc != "2.0" {
        return Ok(Json(json!({
            "jsonrpc": "2.0",
            "id": req.id,
            "error": {"code": -32600, "message": "Invalid Request"}
        }))
        .into_response());
    }

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

    let result = match req.method.as_str() {
        "initialize" => json!({
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
        "tools/list" => {
            let calls = state
                .tools_list_calls
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            if matches!(state.scenario, Scenario::ToolsListFailAfterFirst) && calls > 1 {
                return Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": req.id,
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
            if name != "echo" {
                return Ok(Json(json!({
                    "jsonrpc": "2.0",
                    "id": req.id,
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
            let value = state.resource_event_seq.load(Ordering::SeqCst);
            json!({
                "uri": uri,
                "mimeType": "application/json",
                "text": json!({
                    "uri": uri,
                    "value": value
                })
                .to_string()
            })
        }
        "resources/unsubscribe" => {
            state.resource_subscribed.store(false, Ordering::SeqCst);
            state.resource_event_seq.store(0, Ordering::SeqCst);
            json!({})
        }
        _ => {
            return Ok(Json(json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "error": {"code": -32601, "message": "Method not found"}
            }))
            .into_response())
        }
    };

    Ok(Json(json!({
        "jsonrpc": "2.0",
        "id": req.id,
        "result": result
    }))
    .into_response())
}

async fn mcp_event_stream(
    State(state): State<ServerState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    if matches!(state.scenario, Scenario::AuthRequired) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let resource_subscribed = state.resource_subscribed.clone();
    let resource_event_seq = state.resource_event_seq.clone();
    let stream = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        std::time::Duration::from_millis(200),
    ))
    .filter_map(move |_| {
        let resource_subscribed = resource_subscribed.clone();
        let resource_event_seq = resource_event_seq.clone();
        async move {
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

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
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
