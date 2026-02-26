//! GraphQL test server for E2E testing

use super::common::{Scenario, ServerHandle, bind_available, write_addr_file};
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    Json,
    response::{IntoResponse, Response},
    routing::get,
    Router,
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
    #[serde(default)]
    operation_name: Option<String>,
}

/// GraphQL response
#[derive(Debug, Serialize)]
struct GraphQLResponse {
    data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<serde_json::Value>>,
}

/// Serve GraphQL introspection schema
fn introspection_schema() -> serde_json::Value {
    json!({
      "data": {
        "__schema": {
          "queryType": {"name": "Query"},
          "mutationType": {"name": "Mutation"},
          "types": [
            {
              "name": "Query",
              "fields": [
                {"name": "health"},
                {"name": "user"},
                {"name": "users"}
              ]
            },
            {
              "name": "Mutation",
              "fields": [
                {"name": "createUser"}
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
    match state.scenario {
        Scenario::Ok => {
            let query = req.query.trim();

            // Introspection query
            if query.contains("__schema") || query.contains("__type") {
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

            // User query with ID
            if query.contains("user") && query.contains("id:") {
                // Extract ID from variables
                let id = req.variables.get("id").and_then(|v| v.as_str()).unwrap_or("1");

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
        Scenario::AuthRequired => Err(StatusCode::UNAUTHORIZED),
        Scenario::Malformed => Ok(GraphQLResponse {
            data: Some(json!({"invalid": "<unterminated"})),
            errors: None,
        }),
        Scenario::Timeout => {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            Ok(GraphQLResponse {
                data: Some(json!({"health": {"status": "ok"}})),
                errors: None,
            })
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

    async fn graphql_playground() -> &'static str {
        "<!DOCTYPE html><html><head><title>GraphQL Playground</title></head><body><h1>GraphQL Test Server</body></html>"
    }

    Router::new()
        .route("/", get(graphql_playground).post(graphql_handler))
        .with_state(state)
}

/// Run the GraphQL test server
pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let (listener, addr) = bind_available().await?;
    let state = ServerState { scenario };
    let app = create_router(state);

    info!("GraphQL test server listening on http://{}", addr);
    write_addr_file(addr, "graphql")?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

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
