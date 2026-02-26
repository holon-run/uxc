//! OpenAPI test server for E2E testing

use super::common::{bind_available, write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use tokio::signal::ctrl_c;
use tracing::info;

/// Server state
#[derive(Clone)]
struct ServerState {
    scenario: Scenario,
}

/// Create the OpenAPI test router
fn create_router(state: ServerState) -> Router {
    // Serve OpenAPI schema
    async fn serve_schema() -> Json<serde_json::Value> {
        Json(json!({
          "openapi": "3.0.0",
          "info": {
            "title": "UXC Test API",
            "version": "1.0.0",
            "description": "Local test server for E2E testing"
          },
          "paths": {
            "/health": {
              "get": {
                "operationId": "get_health",
                "summary": "Health check endpoint",
                "responses": {
                  "200": {
                    "description": "Healthy",
                    "content": {
                      "application/json": {
                        "schema": {
                          "type": "object",
                          "properties": {
                            "status": {"type": "string"}
                          }
                        }
                      }
                    }
                  }
                }
              }
            },
            "/users": {
              "get": {
                "operationId": "list_users",
                "summary": "List all users",
                "responses": {
                  "200": {
                    "description": "List of users",
                    "content": {
                      "application/json": {
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
                    }
                  }
                }
              },
              "post": {
                "operationId": "create_user",
                "summary": "Create a new user",
                "requestBody": {
                  "required": true,
                  "content": {
                    "application/json": {
                      "schema": {
                        "type": "object",
                        "required": ["name", "email"],
                        "properties": {
                          "name": {"type": "string"},
                          "email": {"type": "string"}
                        }
                      }
                    }
                  }
                },
                "responses": {
                  "201": {
                    "description": "User created",
                    "content": {
                      "application/json": {
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
                  }
                }
              }
            },
            "/users/{id}": {
              "get": {
                "operationId": "get_user",
                "summary": "Get user by ID",
                "parameters": [{
                  "name": "id",
                  "in": "path",
                  "required": true,
                  "schema": {"type": "integer"}
                }],
                "responses": {
                  "200": {
                    "description": "User details",
                    "content": {
                      "application/json": {
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
                  }
                }
              }
            }
          }
        }))
    }

    // Health check endpoint
    async fn health_check(State(state): State<ServerState>) -> Result<Response, StatusCode> {
        match state.scenario {
            Scenario::Ok => Ok(Json(json!({"status": "ok"})).into_response()),
            Scenario::AuthRequired => Err(StatusCode::UNAUTHORIZED),
            Scenario::Malformed => {
                // Return invalid JSON
                Ok(Response::builder()
                    .header("content-type", "application/json")
                    .body("{invalid json}".into())
                    .unwrap())
            }
            Scenario::Timeout => {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                Ok(Json(json!({"status": "ok"})).into_response())
            }
        }
    }

    // List users endpoint
    async fn list_users(State(state): State<ServerState>) -> Result<Response, StatusCode> {
        match state.scenario {
            Scenario::Ok => {
                let users = vec![
                    json!({"id": 1, "name": "Alice", "email": "alice@example.com"}),
                    json!({"id": 2, "name": "Bob", "email": "bob@example.com"}),
                ];
                Ok(Json(json!(users)).into_response())
            }
            Scenario::AuthRequired => Err(StatusCode::UNAUTHORIZED),
            Scenario::Malformed => Ok(Response::builder()
                .header("content-type", "application/json")
                .body("[{broken}".into())
                .unwrap()),
            Scenario::Timeout => {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                Ok(Json(json!([])).into_response())
            }
        }
    }

    // Create user endpoint
    async fn create_user(
        State(state): State<ServerState>,
        Json(payload): Json<serde_json::Value>,
    ) -> Result<Response, StatusCode> {
        match state.scenario {
            Scenario::Ok => {
                let name = payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let email = payload
                    .get("email")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown@example.com");
                Ok((
                    StatusCode::CREATED,
                    Json(json!({"id": 3, "name": name, "email": email})),
                )
                    .into_response())
            }
            Scenario::AuthRequired => Err(StatusCode::UNAUTHORIZED),
            Scenario::Malformed => Ok(Response::builder()
                .header("content-type", "application/json")
                .body("{created".into())
                .unwrap()),
            Scenario::Timeout => {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                Err(StatusCode::REQUEST_TIMEOUT)
            }
        }
    }

    // Get user by ID endpoint
    async fn get_user(
        State(state): State<ServerState>,
        AxumPath(id): AxumPath<u32>,
    ) -> Result<Response, StatusCode> {
        match state.scenario {
            Scenario::Ok => {
                if id == 1 {
                    Ok(
                        Json(json!({"id": 1, "name": "Alice", "email": "alice@example.com"}))
                            .into_response(),
                    )
                } else if id == 2 {
                    Ok(
                        Json(json!({"id": 2, "name": "Bob", "email": "bob@example.com"}))
                            .into_response(),
                    )
                } else {
                    Err(StatusCode::NOT_FOUND)
                }
            }
            Scenario::AuthRequired => Err(StatusCode::UNAUTHORIZED),
            Scenario::Malformed => Ok(Response::builder()
                .header("content-type", "application/json")
                .body("{invalid".into())
                .unwrap()),
            Scenario::Timeout => {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                Err(StatusCode::NOT_FOUND)
            }
        }
    }

    Router::new()
        .route("/openapi.json", get(serve_schema))
        .route("/health", get(health_check))
        .route("/users", get(list_users).post(create_user))
        .route("/users/:id", get(get_user))
        .with_state(state)
}

/// Run the OpenAPI test server
pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let (listener, addr) = bind_available().await?;
    let state = ServerState { scenario };
    let app = create_router(state);

    info!("OpenAPI test server listening on http://{}", addr);
    write_addr_file(addr, "openapi")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_rx.await.ok();
        info!("OpenAPI test server shutting down");
    });

    tokio::spawn(async move {
        server.await.unwrap();
    });

    Ok(ServerHandle {
        addr,
        shutdown: shutdown_tx,
    })
}

/// Main entry point for the OpenAPI test server binary
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
