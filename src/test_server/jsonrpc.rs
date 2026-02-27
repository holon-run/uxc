//! JSON-RPC test server for E2E testing

use super::common::{bind_available, write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::signal::ctrl_c;
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

/// Serve OpenRPC schema
async fn serve_schema() -> Json<serde_json::Value> {
    Json(schema_value())
}

fn schema_value() -> serde_json::Value {
    json!({
      "openrpc": "1.2.6",
      "info": {
        "title": "UXC Test JSON-RPC API",
        "version": "1.0.0"
      },
      "methods": [
        {
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
        },
        {
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
        },
        {
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
        },
        {
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
        }
      ]
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
        Scenario::Ok => {
            let result = match method {
                "rpc.discover" => schema_value(),
                "health" => json!({"status": "ok"}),
                "list_users" => json!([
                    {"id": 1, "name": "Alice", "email": "alice@example.com"},
                    {"id": 2, "name": "Bob", "email": "bob@example.com"}
                ]),
                "get_user" => {
                    // Extract ID from params
                    let user_id = if let Some(arr) = params.as_array() {
                        arr.get(0).and_then(|v| v.as_i64())
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
        .route("/", get(serve_schema).post(jsonrpc_handler))
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
