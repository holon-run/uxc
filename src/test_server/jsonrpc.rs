//! JSON-RPC test server for E2E testing

use super::common::{bind_available, write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::signal::ctrl_c;
use tokio::time::{sleep, Duration};
use tracing::info;

/// Server state
#[derive(Clone)]
struct ServerState {
    scenario: Scenario,
}

/// JSON-RPC request
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// JSON-RPC response
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

/// JSON-RPC error
#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Clone, Copy)]
enum JsonRpcPubSubProfile {
    DerivedUnsubscribe,
    CloseOnly,
}

fn pubsub_profile(scenario: Scenario) -> Option<JsonRpcPubSubProfile> {
    match scenario {
        Scenario::Ok
        | Scenario::Legacy
        | Scenario::LifecycleStatefulHold
        | Scenario::LifecycleStatefulAllow
        | Scenario::LifecycleStatefulNoSnapshot => Some(JsonRpcPubSubProfile::DerivedUnsubscribe),
        Scenario::SuiPubSub => Some(JsonRpcPubSubProfile::CloseOnly),
        _ => None,
    }
}

fn is_subscription_method(method: &str, profile: JsonRpcPubSubProfile) -> bool {
    match profile {
        JsonRpcPubSubProfile::DerivedUnsubscribe => method.ends_with("_subscribe"),
        JsonRpcPubSubProfile::CloseOnly => {
            method.contains("subscribe") && !method.contains("unsubscribe")
        }
    }
}

fn notification_method(profile: JsonRpcPubSubProfile) -> &'static str {
    match profile {
        JsonRpcPubSubProfile::DerivedUnsubscribe => "eth_subscription",
        JsonRpcPubSubProfile::CloseOnly => "subscription",
    }
}

fn notification_result(profile: JsonRpcPubSubProfile) -> serde_json::Value {
    match profile {
        JsonRpcPubSubProfile::DerivedUnsubscribe => json!({
            "number": "0x1",
            "hash": "0xabc"
        }),
        JsonRpcPubSubProfile::CloseOnly => json!({
            "id": {
                "txDigest": "0xabc",
                "eventSeq": "0"
            },
            "packageId": "0x2"
        }),
    }
}

async fn handle_jsonrpc_websocket(mut socket: WebSocket, state: ServerState) {
    let mut next_subscription_id = 1u64;

    while let Some(Ok(message)) = socket.recv().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let method = value
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);

        if let Some(profile) = pubsub_profile(state.scenario) {
            if is_subscription_method(method, profile) {
                let subscription_id = format!("sub-{}", next_subscription_id);
                next_subscription_id += 1;
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": subscription_id,
                        })
                        .to_string(),
                    ))
                    .await;

                sleep(Duration::from_millis(50)).await;
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "jsonrpc": "2.0",
                            "method": notification_method(profile),
                            "params": {
                                "subscription": subscription_id,
                                "result": notification_result(profile)
                            }
                        })
                        .to_string(),
                    ))
                    .await;
                continue;
            }
        }

        if method.ends_with("_unsubscribe") {
            let _ = socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": true,
                    })
                    .to_string(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        }
    }
}

/// Serve OpenRPC schema
async fn serve_schema(State(state): State<ServerState>) -> Json<serde_json::Value> {
    Json(schema_value(state.scenario))
}

fn schema_value(scenario: Scenario) -> serde_json::Value {
    let mut methods = vec![
        json!({
          "name": "health",
          "summary": "Health check",
          "params": [],
          "result": {
            "name": "result",
            "schema": {
              "type": "object",
              "properties": {
                "status": {"type": "string"}
              }
            }
          }
        }),
        json!({
          "name": "get_user",
          "summary": "Get user by ID",
          "params": [
            {
              "name": "id",
              "schema": {"type": "integer"},
              "required": true
            }
          ],
          "result": {
            "name": "user",
            "schema": {
              "type": "object",
              "properties": {
                "id": {"type": "integer"},
                "name": {"type": "string"},
                "email": {"type": "string"}
              }
            }
          }
        }),
        json!({
          "name": "list_users",
          "summary": "List all users",
          "params": [],
          "result": {
            "name": "users",
            "schema": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "id": {"type": "integer"},
                  "name": {"type": "string"},
                  "email": {"type": "string"}
                }
              }
            }
          }
        }),
        json!({
          "name": "create_user",
          "summary": "Create a new user",
          "params": [
            {
              "name": "name",
              "schema": {"type": "string"},
              "required": true
            },
            {
              "name": "email",
              "schema": {"type": "string"},
              "required": true
            }
          ],
          "result": {
            "name": "user",
            "schema": {
              "type": "object",
              "properties": {
                "id": {"type": "integer"},
                "name": {"type": "string"},
                "email": {"type": "string"}
              }
            }
          }
        }),
    ];

    if matches!(scenario, Scenario::SuiPubSub) {
        methods.push(json!({
          "name": "suix_subscribeEvent",
          "summary": "Subscribe to a stream of Sui event",
          "params": [
            {
              "name": "filter",
              "schema": {"type": "object"},
              "required": true
            }
          ],
          "result": {
            "name": "event",
            "schema": {
              "type": "object",
              "properties": {
                "packageId": {"type": "string"}
              }
            }
          }
        }));
        methods.push(json!({
          "name": "suix_subscribeTransaction",
          "summary": "Subscribe to a stream of Sui transaction effects",
          "params": [
            {
              "name": "filter",
              "schema": {"type": "object"},
              "required": true
            }
          ],
          "result": {
            "name": "effects",
            "schema": {
              "type": "object",
              "properties": {
                "digest": {"type": "string"}
              }
            }
          }
        }));
    }

    json!({
      "openrpc": "1.2.6",
      "info": {
        "title": "UXC Test JSON-RPC API",
        "version": "1.0.0"
      },
      "methods": methods
    })
}

/// Execute JSON-RPC method
async fn execute_method(
    method: &str,
    params: &serde_json::Value,
    id: serde_json::Value,
    state: ServerState,
) -> Result<JsonRpcResponse, StatusCode> {
    match state.scenario {
        Scenario::Ok
        | Scenario::EmptyObjectRequired
        | Scenario::Legacy
        | Scenario::SuiPubSub
        | Scenario::ToolsListFailAfterFirst
        | Scenario::ToolCallTimeout
        | Scenario::ToolStructuredError
        | Scenario::StructuredContent
        | Scenario::ResourceReadFailOnce
        | Scenario::SessionScopedResource
        | Scenario::DynamicToolset
        | Scenario::LifecycleStatefulHold
        | Scenario::LifecycleStatefulAllow
        | Scenario::LifecycleStatefulNoSnapshot
        | Scenario::RequiresInitializedAck => {
            let result = match method {
                "rpc.discover" => schema_value(state.scenario),
                "health" => json!({"status": "ok"}),
                "list_users" => json!([
                    {"id": 1, "name": "Alice", "email": "alice@example.com"},
                    {"id": 2, "name": "Bob", "email": "bob@example.com"}
                ]),
                "get_user" => {
                    // Extract ID from params
                    let user_id = if let Some(arr) = params.as_array() {
                        arr.first().and_then(|v| v.as_i64())
                    } else if let Some(obj) = params.as_object() {
                        obj.get("id").and_then(|v| v.as_i64())
                    } else {
                        None
                    };

                    match user_id {
                        Some(1) => json!({"id": 1, "name": "Alice", "email": "alice@example.com"}),
                        Some(2) => json!({"id": 2, "name": "Bob", "email": "bob@example.com"}),
                        _ => json!(null),
                    }
                }
                "create_user" => {
                    let (name, email) = if let Some(arr) = params.as_array() {
                        (
                            arr.first()
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown")
                                .to_string(),
                            arr.get(1)
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown@example.com")
                                .to_string(),
                        )
                    } else if let Some(obj) = params.as_object() {
                        (
                            obj.get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown")
                                .to_string(),
                            obj.get("email")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown@example.com")
                                .to_string(),
                        )
                    } else {
                        ("Unknown".to_string(), "unknown@example.com".to_string())
                    };

                    json!({"id": 3, "name": name, "email": email})
                }
                _ => {
                    return Ok(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32601,
                            message: format!("Method not found: {}", method),
                        }),
                    });
                }
            };

            Ok(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(result),
                error: None,
            })
        }
        Scenario::AuthRequired => Err(StatusCode::UNAUTHORIZED),
        Scenario::Malformed => Ok(JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({"invalid": "malformed"})),
            error: None,
        }),
        Scenario::Timeout => {
            tokio::time::sleep(super::common::timeout_duration()).await;
            Ok(JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({"status": "ok"})),
                error: None,
            })
        }
    }
}

/// Create the JSON-RPC test router
fn create_router(state: ServerState) -> Router {
    async fn jsonrpc_get(
        ws: Option<WebSocketUpgrade>,
        State(state): State<ServerState>,
    ) -> Response {
        if let Some(ws) = ws {
            return ws
                .on_upgrade(move |socket| handle_jsonrpc_websocket(socket, state))
                .into_response();
        }
        serve_schema(State(state)).await.into_response()
    }

    async fn jsonrpc_handler(
        State(state): State<ServerState>,
        Json(req): Json<JsonRpcRequest>,
    ) -> Response {
        if req.jsonrpc != "2.0" {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "error": {"code": -32600, "message": "Invalid Request"}
            }))
            .into_response();
        }

        match execute_method(&req.method, &req.params, req.id, state).await {
            Ok(resp) => Json(resp).into_response(),
            Err(status) => status.into_response(),
        }
    }

    // Handle MCP probe endpoints (return 404)
    async fn not_found() -> StatusCode {
        StatusCode::NOT_FOUND
    }

    Router::new()
        .route("/", get(jsonrpc_get).post(jsonrpc_handler))
        .route("/openrpc.json", get(serve_schema))
        .route("/.well-known/openrpc.json", get(serve_schema))
        .route("/.well-known/mcp", get(not_found).post(not_found))
        .route("/mcp", get(not_found).post(not_found))
        .with_state(state)
}

/// Run the JSON-RPC test server
pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let (listener, addr) = bind_available().await?;
    let state = ServerState { scenario };
    let app = create_router(state);

    info!("JSON-RPC test server listening on http://{}", addr);
    write_addr_file(addr, "jsonrpc")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_rx.await.ok();
        info!("JSON-RPC test server shutting down");
    });

    tokio::spawn(async move {
        server.await.unwrap();
    });

    Ok(ServerHandle {
        addr,
        shutdown: shutdown_tx,
    })
}

/// Main entry point for the JSON-RPC test server binary
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

    let _handle = run(scenario).await?;

    // Wait for ctrl+c
    ctrl_c().await?;
    Ok(())
}
