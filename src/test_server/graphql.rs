//! GraphQL test server for E2E testing

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
use std::collections::HashMap;
use tokio::signal::ctrl_c;
use tracing::info;

/// Server state
#[derive(Clone)]
struct ServerState {
    scenario: Scenario,
}

/// GraphQL request
#[derive(Debug, Deserialize)]
struct GraphQLRequest {
    query: String,
    #[serde(default)]
    variables: HashMap<String, serde_json::Value>,
}

/// GraphQL response
#[derive(Debug, Serialize)]
struct GraphQLResponse {
    data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<serde_json::Value>>,
}

fn extract_inline_arg(query: &str, arg_name: &str) -> Option<String> {
    let marker = format!("{arg_name}:");
    let marker_pos = query.find(&marker)?;
    let rest = query[marker_pos + marker.len()..].trim_start();

    if let Some(stripped) = rest.strip_prefix('"') {
        // Find an unescaped closing quote, so strings like
        // "Alice \"Bob\"" are parsed as a single argument value.
        let mut prev_was_backslash = false;
        for (idx, ch) in stripped.char_indices() {
            if ch == '"' && !prev_was_backslash {
                return Some(stripped[..idx].to_string());
            }
            if ch == '\\' {
                prev_was_backslash = !prev_was_backslash;
            } else {
                prev_was_backslash = false;
            }
        }
        return None;
    }

    let end = rest.find([',', ')', ' ', '\n', '\t']).unwrap_or(rest.len());
    let token = rest[..end].trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::extract_inline_arg;

    #[test]
    fn extract_inline_arg_handles_escaped_quotes() {
        let query =
            r#"mutation { createUser(name: "Alice \"Bob\"", email: "alice@example.com") { id } }"#;
        let value = extract_inline_arg(query, "name");
        assert_eq!(value.as_deref(), Some(r#"Alice \"Bob\""#));
    }
}

/// Serve GraphQL introspection schema
fn introspection_schema() -> serde_json::Value {
    json!({
      "data": {
        "__schema": {
          "queryType": {
            "name": "Query",
            "fields": [
              {
                "name": "health",
                "description": "Health check",
                "args": [],
                "type": {"kind": "OBJECT", "name": "HealthResult", "ofType": null}
              },
              {
                "name": "user",
                "description": "Find a user by id",
                "args": [
                  {
                    "name": "id",
                    "description": "User ID",
                    "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "ID", "name": "ID", "ofType": null}},
                    "defaultValue": null
                  }
                ],
                "type": {"kind": "OBJECT", "name": "User", "ofType": null}
              },
              {
                "name": "users",
                "description": "List users",
                "args": [],
                "type": {"kind": "LIST", "name": null, "ofType": {"kind": "OBJECT", "name": "User", "ofType": null}}
              }
            ]
          },
          "mutationType": {
            "name": "Mutation",
            "fields": [
              {
                "name": "createUser",
                "description": "Create user",
                "args": [
                  {
                    "name": "name",
                    "description": "User name",
                    "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "SCALAR", "name": "String", "ofType": null}},
                    "defaultValue": null
                  },
                  {
                    "name": "email",
                    "description": "User email",
                    "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "SCALAR", "name": "String", "ofType": null}},
                    "defaultValue": null
                  }
                ],
                "type": {"kind": "OBJECT", "name": "User", "ofType": null}
              },
              {
                "name": "issueCreate",
                "description": "Create issue",
                "args": [
                  {
                    "name": "input",
                    "description": "Issue creation payload",
                    "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "INPUT_OBJECT", "name": "IssueCreateInput", "ofType": null}},
                    "defaultValue": null
                  }
                ],
                "type": {"kind": "OBJECT", "name": "Issue", "ofType": null}
              }
              ]
          },
          "subscriptionType": {
            "name": "Subscription",
            "fields": [
              {
                "name": "messageAdded",
                "description": "Watch messages in a room",
                "args": [
                  {
                    "name": "roomId",
                    "description": "Room identifier",
                    "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "ID", "name": "ID", "ofType": null}},
                    "defaultValue": null
                  }
                ],
                "type": {"kind": "OBJECT", "name": "Message", "ofType": null}
              }
            ]
          },
          "types": [
            {
              "name": "Query",
              "fields": [
                {
                  "name": "health",
                  "description": "Health check",
                  "args": [],
                  "type": {"kind": "OBJECT", "name": "HealthResult", "ofType": null}
                },
                {
                  "name": "user",
                  "description": "Find a user by id",
                  "args": [
                    {
                      "name": "id",
                      "description": "User ID",
                      "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "ID", "name": "ID", "ofType": null}},
                      "defaultValue": null
                    }
                  ],
                  "type": {"kind": "OBJECT", "name": "User", "ofType": null}
                },
                {
                  "name": "users",
                  "description": "List users",
                  "args": [],
                  "type": {"kind": "LIST", "name": null, "ofType": {"kind": "OBJECT", "name": "User", "ofType": null}}
                }
              ]
            },
            {
              "name": "Mutation",
              "fields": [
                {
                  "name": "createUser",
                  "description": "Create user",
                  "args": [
                    {
                      "name": "name",
                      "description": "User name",
                      "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "SCALAR", "name": "String", "ofType": null}},
                      "defaultValue": null
                    },
                    {
                      "name": "email",
                      "description": "User email",
                      "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "SCALAR", "name": "String", "ofType": null}},
                      "defaultValue": null
                    }
                  ],
                  "type": {"kind": "OBJECT", "name": "User", "ofType": null}
                },
                {
                  "name": "issueCreate",
                  "description": "Create issue",
                  "args": [
                    {
                      "name": "input",
                      "description": "Issue creation payload",
                      "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "INPUT_OBJECT", "name": "IssueCreateInput", "ofType": null}},
                      "defaultValue": null
                    }
                  ],
                  "type": {"kind": "OBJECT", "name": "Issue", "ofType": null}
                }
              ]
            },
            {
              "name": "Subscription",
              "fields": [
                {
                  "name": "messageAdded",
                  "description": "Watch messages in a room",
                  "args": [
                    {
                      "name": "roomId",
                      "description": "Room identifier",
                      "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "ID", "name": "ID", "ofType": null}},
                      "defaultValue": null
                    }
                  ],
                  "type": {"kind": "OBJECT", "name": "Message", "ofType": null}
                }
              ]
            },
            {
              "name": "User",
              "fields": [
                {"name": "id", "type": {"name": "ID", "kind": "SCALAR"}},
                {"name": "name", "type": {"name": "String", "kind": "SCALAR"}},
                {"name": "email", "type": {"name": "String", "kind": "SCALAR"}}
              ]
            },
            {
              "name": "HealthResult",
              "fields": [
                {"name": "status", "type": {"name": "String", "kind": "SCALAR"}}
              ]
            },
            {
              "name": "Issue",
              "fields": [
                {"name": "id", "type": {"name": "ID", "kind": "SCALAR"}},
                {"name": "teamId", "type": {"name": "ID", "kind": "SCALAR"}},
                {"name": "title", "type": {"name": "String", "kind": "SCALAR"}}
              ]
            },
            {
              "name": "Message",
              "fields": [
                {"name": "id", "type": {"name": "ID", "kind": "SCALAR"}},
                {"name": "roomId", "type": {"name": "ID", "kind": "SCALAR"}},
                {"name": "body", "type": {"name": "String", "kind": "SCALAR"}}
              ]
            },
            {
              "name": "IssueCreateInput",
              "kind": "INPUT_OBJECT",
              "inputFields": [
                {
                  "name": "teamId",
                  "description": "Team identifier",
                  "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "SCALAR", "name": "ID", "ofType": null}}
                },
                {
                  "name": "title",
                  "description": "Issue title",
                  "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "SCALAR", "name": "String", "ofType": null}}
                }
              ]
            }
          ]
        }
      }
    })
}

/// Execute GraphQL query
async fn execute_query(
    req: GraphQLRequest,
    state: ServerState,
) -> Result<GraphQLResponse, StatusCode> {
    let query = req.query.trim();

    // Keep introspection available even in auth_required mode so protocol detection can succeed.
    if matches!(state.scenario, Scenario::AuthRequired)
        && !(query.contains("__schema") || query.contains("__type("))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match state.scenario {
        Scenario::Ok
        | Scenario::EmptyObjectRequired
        | Scenario::Legacy
        | Scenario::SuiPubSub
        | Scenario::AuthRequired
        | Scenario::ToolsListFailAfterFirst
        | Scenario::ToolCallTimeout
        | Scenario::StructuredContent
        | Scenario::DynamicToolset => {
            // Introspection query
            if query.contains("__schema") || query.contains("__type(") {
                return Ok(GraphQLResponse {
                    data: Some(introspection_schema()["data"].clone()),
                    errors: None,
                });
            }

            // Health query
            if query.contains("health") {
                return Ok(GraphQLResponse {
                    data: Some(json!({"health": {"status": "ok"}})),
                    errors: None,
                });
            }

            // Users query
            if query.contains("users") {
                return Ok(GraphQLResponse {
                    data: Some(json!({
                        "users": [
                            {"id": "1", "name": "Alice", "email": "alice@example.com"},
                            {"id": "2", "name": "Bob", "email": "bob@example.com"}
                        ]
                    })),
                    errors: None,
                });
            }

            // Create user mutation
            if query.contains("createUser") {
                let name = req
                    .variables
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| extract_inline_arg(query, "name"))
                    .unwrap_or_else(|| "Charlie".to_string());
                let email = req
                    .variables
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| extract_inline_arg(query, "email"))
                    .unwrap_or_else(|| "charlie@example.com".to_string());
                return Ok(GraphQLResponse {
                    data: Some(json!({
                        "createUser": {"id": "3", "name": name, "email": email}
                    })),
                    errors: None,
                });
            }

            // Create issue mutation
            if query.contains("issueCreate") {
                let input = req.variables.get("input").and_then(|v| v.as_object());
                let team_id = input
                    .and_then(|v| v.get("teamId"))
                    .and_then(|v| {
                        v.as_str()
                            .map(ToString::to_string)
                            .or_else(|| v.as_i64().map(|n| n.to_string()))
                    })
                    .unwrap_or_else(|| "TEAM-1".to_string());
                let title = input
                    .and_then(|v| v.get("title"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Untitled issue".to_string());

                return Ok(GraphQLResponse {
                    data: Some(json!({
                        "issueCreate": {"id": "I-1", "teamId": team_id, "title": title}
                    })),
                    errors: None,
                });
            }

            // User query (optionally with ID variable)
            if query.contains("user") && !query.contains("users") {
                // Extract ID from variables, accept both string and number.
                let id = req
                    .variables
                    .get("id")
                    .and_then(|v| {
                        v.as_str()
                            .map(ToString::to_string)
                            .or_else(|| v.as_i64().map(|n| n.to_string()))
                    })
                    .or_else(|| extract_inline_arg(query, "id"))
                    .unwrap_or_else(|| "1".to_string());

                if id == "1" {
                    return Ok(GraphQLResponse {
                        data: Some(json!({
                            "user": {"id": "1", "name": "Alice", "email": "alice@example.com"}
                        })),
                        errors: None,
                    });
                } else if id == "2" {
                    return Ok(GraphQLResponse {
                        data: Some(json!({
                            "user": {"id": "2", "name": "Bob", "email": "bob@example.com"}
                        })),
                        errors: None,
                    });
                } else {
                    return Ok(GraphQLResponse {
                        data: Some(json!({"user": null})),
                        errors: Some(vec![json!({"message": "User not found"})]),
                    });
                }
            }

            // Unknown query
            Ok(GraphQLResponse {
                data: Some(json!(null)),
                errors: Some(vec![json!({"message": "Unknown query"})]),
            })
        }
        Scenario::Malformed => Ok(GraphQLResponse {
            data: Some(json!({"invalid": "<unterminated"})),
            errors: None,
        }),
        Scenario::Timeout => {
            tokio::time::sleep(super::common::timeout_duration()).await;
            Ok(GraphQLResponse {
                data: Some(json!({"health": {"status": "ok"}})),
                errors: None,
            })
        }
    }
}

async fn handle_graphql_websocket(mut socket: WebSocket, state: ServerState) {
    while let Some(message) = socket.recv().await {
        let Ok(message) = message else {
            return;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            let _ = socket
                .send(Message::Text(
                    json!({"type":"error","payload":{"message":"invalid json"}}).to_string(),
                ))
                .await;
            return;
        };
        let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match msg_type {
            "connection_init" => {
                if matches!(state.scenario, Scenario::AuthRequired) {
                    let _ = socket
                        .send(Message::Text(
                            json!({"type":"error","payload":{"message":"unauthorized"}})
                                .to_string(),
                        ))
                        .await;
                    return;
                }
                let _ = socket
                    .send(Message::Text(json!({"type":"connection_ack"}).to_string()))
                    .await;
                if matches!(state.scenario, Scenario::Legacy) {
                    let _ = socket
                        .send(Message::Text(json!({"type":"ka"}).to_string()))
                        .await;
                }
            }
            "subscribe" => {
                if matches!(state.scenario, Scenario::Legacy) {
                    let _ = socket
                        .send(Message::Text(
                            json!({"type":"error","payload":{"message":"legacy server expects start"}})
                                .to_string(),
                        ))
                        .await;
                    return;
                }
                let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("1");
                let room_id = value
                    .get("payload")
                    .and_then(|v| v.get("variables"))
                    .and_then(|v| v.get("roomId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("room-1");

                let _ = socket
                    .send(Message::Text(
                        json!({
                            "id": id,
                            "type":"next",
                            "payload":{
                                "data":{"messageAdded":{"id":"m1","roomId":room_id,"body":"hello"}}
                            }
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "id": id,
                            "type":"next",
                            "payload":{
                                "data":{"messageAdded":{"id":"m2","roomId":room_id,"body":"world"}},
                                "extensions":{"trace":"ok"}
                            }
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket
                    .send(Message::Text(
                        json!({"id":id,"type":"complete"}).to_string(),
                    ))
                    .await;
                let _ = socket.close().await;
                return;
            }
            "start" => {
                if !matches!(state.scenario, Scenario::Legacy) {
                    continue;
                }
                let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("1");
                let room_id = value
                    .get("payload")
                    .and_then(|v| v.get("variables"))
                    .and_then(|v| v.get("roomId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("room-1");

                let _ = socket
                    .send(Message::Text(
                        json!({
                            "id": id,
                            "type":"data",
                            "payload":{
                                "data":{"messageAdded":{"id":"m1","roomId":room_id,"body":"hello"}}
                            }
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "id": id,
                            "type":"data",
                            "payload":{
                                "data":{"messageAdded":{"id":"m2","roomId":room_id,"body":"world"}},
                                "extensions":{"trace":"ok"}
                            }
                        })
                        .to_string(),
                    ))
                    .await;
                let _ = socket
                    .send(Message::Text(
                        json!({"id":id,"type":"complete"}).to_string(),
                    ))
                    .await;
                let _ = socket.close().await;
                return;
            }
            "ping" => {
                let _ = socket
                    .send(Message::Text(
                        json!({"type":"pong","payload":value.get("payload").cloned().unwrap_or(json!(null))}).to_string(),
                    ))
                    .await;
            }
            _ => {}
        }
    }
}

/// Create the GraphQL test router
fn create_router(state: ServerState) -> Router {
    async fn graphql_handler(
        State(state): State<ServerState>,
        Json(req): Json<GraphQLRequest>,
    ) -> Result<Response, StatusCode> {
        let response = execute_query(req, state).await?;

        if let Some(errors) = &response.errors {
            if !errors.is_empty() {
                return Ok((
                    StatusCode::OK,
                    Json(serde_json::to_value(response).unwrap()),
                )
                    .into_response());
            }
        }

        Ok(Json(response).into_response())
    }

    async fn graphql_get(
        ws: Option<WebSocketUpgrade>,
        State(state): State<ServerState>,
    ) -> Response {
        if let Some(ws) = ws {
            if matches!(state.scenario, Scenario::Legacy) {
                return ws
                    .protocols(["graphql-ws"])
                    .on_upgrade(move |socket| handle_graphql_websocket(socket, state))
                    .into_response();
            }
            return ws
                .protocols(["graphql-transport-ws"])
                .on_upgrade(move |socket| handle_graphql_websocket(socket, state))
                .into_response();
        }

        "<!DOCTYPE html><html><head><title>GraphQL Playground</title></head><body><h1>GraphQL Test Server</body></html>"
            .into_response()
    }

    Router::new()
        .route("/", get(graphql_get).post(graphql_handler))
        .with_state(state)
}

/// Run the GraphQL test server
pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let (listener, addr) = bind_available().await?;
    let state = ServerState { scenario };
    let app = create_router(state);

    info!("GraphQL test server listening on http://{}", addr);
    write_addr_file(addr, "graphql")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_rx.await.ok();
        info!("GraphQL test server shutting down");
    });

    tokio::spawn(async move {
        server.await.unwrap();
    });

    Ok(ServerHandle {
        addr,
        shutdown: shutdown_tx,
    })
}

/// Main entry point for the GraphQL test server binary
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
