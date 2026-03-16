//! Raw WebSocket test server for E2E testing

use super::common::{bind_available, write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::json;
use tokio::signal::ctrl_c;
use tokio::time::{timeout, Duration};
use tracing::info;

#[derive(Clone)]
struct ServerState {
    scenario: Scenario,
}

async fn handle_websocket_socket(kind: String, mut socket: WebSocket) {
    match kind.as_str() {
        "frames" => {
            let _ = socket
                .send(Message::Text(
                    r#"{"price":"123.45","symbol":"BTCUSDT"}"#.into(),
                ))
                .await;
            let _ = socket.send(Message::Text("tick".into())).await;
            let _ = socket.send(Message::Binary(vec![1, 2, 3])).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = socket.close().await;
        }
        "init" => {
            let mut received_frames = Vec::new();
            for _ in 0..2 {
                match timeout(Duration::from_secs(2), socket.recv()).await {
                    Ok(Some(Ok(Message::Text(text)))) => received_frames.push(text),
                    Ok(Some(Ok(_))) => break,
                    Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                }
            }
            let _ = socket
                .send(Message::Text(
                    json!({ "received_frames": received_frames }).to_string(),
                ))
                .await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = socket.close().await;
        }
        "subprotocol" => {
            let _ = socket.send(Message::Text(r#"{"ready":true}"#.into())).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = socket.close().await;
        }
        _ => {
            let _ = socket.close().await;
        }
    }
}

async fn websocket_handler(
    AxumPath(kind): AxumPath<String>,
    State(state): State<ServerState>,
    ws: WebSocketUpgrade,
) -> Response {
    if !matches!(state.scenario, Scenario::Ok) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let upgrade = if kind == "subprotocol" {
        ws.protocols(["market.v1"])
    } else {
        ws
    };

    upgrade
        .on_upgrade(move |socket| handle_websocket_socket(kind, socket))
        .into_response()
}

fn create_router(state: ServerState) -> Router {
    Router::new()
        .route("/:kind", get(websocket_handler))
        .with_state(state)
}

pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let (listener, addr) = bind_available().await?;
    let state = ServerState { scenario };
    let app = create_router(state);

    info!("WebSocket test server listening on ws://{}", addr);
    write_addr_file(addr, "websocket")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_rx.await.ok();
        info!("WebSocket test server shutting down");
    });

    tokio::spawn(async move {
        server.await.unwrap();
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
