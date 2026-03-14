use crate::auth::{self, Profile, ResolvedRequestAuth};
use crate::daemon_log::redact_endpoint;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Once;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message;

const WEBSOCKET_CONNECT_TIMEOUT_SECS: u64 = 2;
static RUSTLS_PROVIDER_INIT: Once = Once::new();

#[derive(Debug, Clone)]
pub struct WebSocketRuntimeConfig {
    pub endpoint: String,
    pub auth_profile: Option<Profile>,
    pub subprotocols: Vec<String>,
    pub initial_text_frames: Vec<String>,
    pub first_message_timeout_secs: Option<u64>,
    pub initial_reconnect_delay_secs: u64,
    pub max_reconnect_delay_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum WebSocketHandlerAction {
    Continue,
    Reconnect,
    Stop,
}

#[derive(Debug, Clone)]
pub struct WebSocketHandlerOutput {
    pub action: WebSocketHandlerAction,
    pub data: Option<Value>,
    pub meta: Option<Value>,
    pub outbound_text_frames: Vec<String>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct WebSocketStopOutput {
    pub outbound_text_frames: Vec<String>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebSocketOpenMeta {
    pub redacted_url: String,
    pub subprotocol: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WebSocketCloseMeta {
    pub code: Option<u16>,
    pub reason: Option<String>,
}

#[async_trait]
pub trait WebSocketSessionHandler: Send {
    async fn on_open(&mut self, _meta: &WebSocketOpenMeta) -> Result<WebSocketHandlerAction> {
        Ok(WebSocketHandlerAction::Continue)
    }

    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput>;

    async fn on_binary_frame(&mut self, bytes: Vec<u8>) -> Result<WebSocketHandlerOutput>;

    async fn on_close(&mut self, _meta: WebSocketCloseMeta) -> Result<WebSocketHandlerAction> {
        Ok(WebSocketHandlerAction::Reconnect)
    }

    async fn on_stop_requested(&mut self) -> Result<WebSocketStopOutput> {
        Ok(WebSocketStopOutput::default())
    }
}

#[async_trait]
pub trait WebSocketRuntimeObserver: Send {
    async fn emit(
        &mut self,
        event_kind: &str,
        data: Option<Value>,
        meta: Option<Value>,
    ) -> Result<()>;

    async fn update_status(
        &mut self,
        status: Option<&str>,
        last_error: Option<String>,
        increment_reconnect: bool,
    ) -> Result<()>;
}

#[derive(Default)]
pub struct RawFrameHandler;

#[async_trait]
impl WebSocketSessionHandler for RawFrameHandler {
    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: Some(value),
                meta: Some(json!({"frame_type":"text_json"})),
                outbound_text_frames: Vec::new(),
                stop_reason: None,
            }),
            Err(_) => Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: Some(json!({"frame_type":"text","text":text})),
                outbound_text_frames: Vec::new(),
                stop_reason: None,
            }),
        }
    }

    async fn on_binary_frame(&mut self, bytes: Vec<u8>) -> Result<WebSocketHandlerOutput> {
        use base64::Engine;

        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: Some(json!({
                "frame_type":"binary",
                "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            })),
            outbound_text_frames: Vec::new(),
            stop_reason: None,
        })
    }
}

pub enum WebSocketRunError {
    Retry(anyhow::Error),
    Fatal(anyhow::Error),
}

fn stop_requested(stop_rx: &watch::Receiver<bool>) -> bool {
    *stop_rx.borrow()
}

async fn wait_for_stop_or_timeout(stop_rx: &mut watch::Receiver<bool>, duration: Duration) -> bool {
    if *stop_rx.borrow() {
        return true;
    }
    tokio::select! {
        changed = stop_rx.changed() => matches!(changed, Ok(())) && *stop_rx.borrow(),
        _ = tokio::time::sleep(duration) => false,
    }
}

async fn close_as_stopped<O: WebSocketRuntimeObserver>(
    observer: &mut O,
    reason: &str,
) -> Result<()> {
    observer
        .emit("closed", None, Some(json!({"reason": reason})))
        .await?;
    observer.update_status(Some("stopped"), None, false).await
}

async fn handle_stop_requested<H: WebSocketSessionHandler, O: WebSocketRuntimeObserver>(
    handler: &mut H,
    stream: Option<
        &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    observer: &mut O,
    default_reason: &str,
) -> std::result::Result<(), WebSocketRunError> {
    let stop_output = handler
        .on_stop_requested()
        .await
        .map_err(WebSocketRunError::Fatal)?;
    if let Some(stream) = stream {
        for frame in &stop_output.outbound_text_frames {
            if let Err(err) = stream.send(Message::Text(frame.clone())).await {
                tracing::debug!("websocket cleanup send failed: {}", err);
                break;
            }
        }
    }
    close_as_stopped(
        observer,
        stop_output.stop_reason.as_deref().unwrap_or(default_reason),
    )
    .await
    .map_err(WebSocketRunError::Fatal)
}

fn resolved_request_auth(
    endpoint: &str,
    auth_profile: Option<&Profile>,
) -> Result<ResolvedRequestAuth> {
    match auth_profile {
        Some(profile) => auth::resolve_profile_request_auth(endpoint, profile),
        None => Ok(ResolvedRequestAuth {
            url: endpoint.to_string(),
            headers: Vec::new(),
        }),
    }
}

fn ensure_rustls_crypto_provider() {
    RUSTLS_PROVIDER_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn connect_once(
    config: &WebSocketRuntimeConfig,
    stop_rx: &mut watch::Receiver<bool>,
) -> std::result::Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WebSocketOpenMeta,
    ),
    WebSocketRunError,
> {
    ensure_rustls_crypto_provider();
    let resolved = resolved_request_auth(&config.endpoint, config.auth_profile.as_ref())
        .map_err(WebSocketRunError::Fatal)?;
    let mut request = resolved
        .url
        .clone()
        .into_client_request()
        .with_context(|| format!("Invalid WebSocket endpoint: {}", config.endpoint))
        .map_err(WebSocketRunError::Fatal)?;

    for (name, value) in resolved.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|err| WebSocketRunError::Fatal(anyhow!(err)))?;
        let header_value =
            HeaderValue::from_str(&value).map_err(|err| WebSocketRunError::Fatal(anyhow!(err)))?;
        request.headers_mut().insert(header_name, header_value);
    }

    if !config.subprotocols.is_empty() {
        let value = HeaderValue::from_str(&config.subprotocols.join(", "))
            .map_err(|err| WebSocketRunError::Fatal(anyhow!(err)))?;
        request
            .headers_mut()
            .insert(HeaderName::from_static("sec-websocket-protocol"), value);
    }

    let connect_future = connect_async(request);
    let (stream, response) = tokio::select! {
        changed = stop_rx.changed() => {
            if changed.is_ok() && *stop_rx.borrow() {
                return Err(WebSocketRunError::Retry(anyhow!("websocket connect cancelled by stop request")));
            }
            return Err(WebSocketRunError::Retry(anyhow!("subscription stop channel closed unexpectedly")));
        }
        result = tokio::time::timeout(Duration::from_secs(WEBSOCKET_CONNECT_TIMEOUT_SECS), connect_future) => {
            match result {
                Ok(Ok(value)) => value,
                Ok(Err(err)) => return Err(WebSocketRunError::Retry(anyhow!(err).context("websocket connection failed"))),
                Err(_) => return Err(WebSocketRunError::Retry(anyhow!(
                    "websocket connect timed out after {}s",
                    WEBSOCKET_CONNECT_TIMEOUT_SECS
                ))),
            }
        }
    };
    let subprotocol = response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    Ok((
        stream,
        WebSocketOpenMeta {
            redacted_url: redact_endpoint(&resolved.url),
            subprotocol,
        },
    ))
}

async fn run_session_once<H: WebSocketSessionHandler, O: WebSocketRuntimeObserver>(
    config: &WebSocketRuntimeConfig,
    handler: &mut H,
    observer: &mut O,
    stop_rx: &mut watch::Receiver<bool>,
) -> std::result::Result<(), WebSocketRunError> {
    if stop_requested(stop_rx) {
        handle_stop_requested(handler, None, observer, "stopped").await?;
        return Ok(());
    }

    let (mut stream, open_meta) = connect_once(config, stop_rx).await?;

    observer
        .emit(
            "open",
            None,
            Some(json!({
                "url": open_meta.redacted_url,
                "subprotocol": open_meta.subprotocol,
            })),
        )
        .await
        .map_err(WebSocketRunError::Fatal)?;

    for frame in &config.initial_text_frames {
        stream
            .send(Message::Text(frame.clone()))
            .await
            .map_err(|err| {
                WebSocketRunError::Retry(anyhow!("websocket initial send failed: {}", err))
            })?;
    }

    match handler
        .on_open(&open_meta)
        .await
        .map_err(WebSocketRunError::Fatal)?
    {
        WebSocketHandlerAction::Stop => {
            close_as_stopped(observer, "handler_stop")
                .await
                .map_err(WebSocketRunError::Fatal)?;
            return Ok(());
        }
        WebSocketHandlerAction::Reconnect => {
            return Err(WebSocketRunError::Retry(anyhow!(
                "websocket handler requested reconnect during open"
            )));
        }
        WebSocketHandlerAction::Continue => {}
    }

    observer
        .update_status(Some("running"), None, false)
        .await
        .map_err(WebSocketRunError::Fatal)?;

    let mut first_message_timeout = config.first_message_timeout_secs.map(Duration::from_secs);

    loop {
        if stop_requested(stop_rx) {
            handle_stop_requested(handler, Some(&mut stream), observer, "stopped").await?;
            return Ok(());
        }

        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_ok() && *stop_rx.borrow() {
                    handle_stop_requested(handler, Some(&mut stream), observer, "stopped").await?;
                    return Ok(());
                }
                return Err(WebSocketRunError::Retry(anyhow!("subscription stop channel closed unexpectedly")));
            }
            item = async {
                if let Some(duration) = first_message_timeout {
                    match tokio::time::timeout(duration, stream.next()).await {
                        Ok(item) => item,
                        Err(_) => {
                            Some(Err(tokio_tungstenite::tungstenite::Error::Io(
                                std::io::Error::new(
                                    std::io::ErrorKind::TimedOut,
                                    format!("websocket first message timed out after {}s", duration.as_secs()),
                                ),
                            )))
                        }
                    }
                } else {
                    stream.next().await
                }
            } => {
                match item {
                    Some(Ok(Message::Text(text))) => {
                        first_message_timeout = None;
                        let output = handler
                            .on_text_frame(text.to_string())
                            .await
                            .map_err(WebSocketRunError::Fatal)?;
                        for frame in &output.outbound_text_frames {
                            stream
                                .send(Message::Text(frame.clone()))
                                .await
                                .map_err(|err| WebSocketRunError::Retry(anyhow!("websocket send failed: {}", err)))?;
                        }
                        if output.data.is_some() || output.meta.is_some() {
                            observer
                                .emit("data", output.data, output.meta)
                                .await
                                .map_err(WebSocketRunError::Fatal)?;
                        }
                        match output.action {
                            WebSocketHandlerAction::Continue => {}
                            WebSocketHandlerAction::Stop => {
                                close_as_stopped(
                                    observer,
                                    output.stop_reason.as_deref().unwrap_or("handler_stop"),
                                )
                                    .await
                                    .map_err(WebSocketRunError::Fatal)?;
                                return Ok(());
                            }
                            WebSocketHandlerAction::Reconnect => {
                                return Err(WebSocketRunError::Retry(anyhow!("websocket handler requested reconnect")));
                            }
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        first_message_timeout = None;
                        let output = handler
                            .on_binary_frame(bytes.to_vec())
                            .await
                            .map_err(WebSocketRunError::Fatal)?;
                        for frame in &output.outbound_text_frames {
                            stream
                                .send(Message::Text(frame.clone()))
                                .await
                                .map_err(|err| WebSocketRunError::Retry(anyhow!("websocket send failed: {}", err)))?;
                        }
                        if output.data.is_some() || output.meta.is_some() {
                            observer
                                .emit("data", output.data, output.meta)
                                .await
                                .map_err(WebSocketRunError::Fatal)?;
                        }
                        match output.action {
                            WebSocketHandlerAction::Continue => {}
                            WebSocketHandlerAction::Stop => {
                                close_as_stopped(
                                    observer,
                                    output.stop_reason.as_deref().unwrap_or("handler_stop"),
                                )
                                    .await
                                    .map_err(WebSocketRunError::Fatal)?;
                                return Ok(());
                            }
                            WebSocketHandlerAction::Reconnect => {
                                return Err(WebSocketRunError::Retry(anyhow!("websocket handler requested reconnect")));
                            }
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let close_meta = WebSocketCloseMeta {
                            code: frame.as_ref().map(|value| value.code.into()),
                            reason: frame.as_ref().map(|value| value.reason.to_string()),
                        };
                        match handler.on_close(close_meta.clone()).await.map_err(WebSocketRunError::Fatal)? {
                            WebSocketHandlerAction::Stop => {
                                close_as_stopped(observer, "remote_close")
                                    .await
                                    .map_err(WebSocketRunError::Fatal)?;
                                return Ok(());
                            }
                            WebSocketHandlerAction::Continue | WebSocketHandlerAction::Reconnect => {
                                return Err(WebSocketRunError::Retry(anyhow!(
                                    "websocket closed by remote peer{}",
                                    close_meta
                                        .code
                                        .map(|code| format!(" with code {}", code))
                                        .unwrap_or_default()
                                )));
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        first_message_timeout = None;
                        stream
                            .send(Message::Pong(payload))
                            .await
                            .map_err(|err| WebSocketRunError::Retry(anyhow!("websocket pong send failed: {}", err)))?;
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {
                        first_message_timeout = None;
                    }
                    Some(Err(err)) => return Err(WebSocketRunError::Retry(anyhow!("websocket read failed: {}", err))),
                    None => return Err(WebSocketRunError::Retry(anyhow!("websocket stream ended"))),
                }
            }
        }
    }
}

pub async fn run_websocket_subscription_session_once<
    H: WebSocketSessionHandler,
    O: WebSocketRuntimeObserver,
>(
    config: &WebSocketRuntimeConfig,
    handler: &mut H,
    observer: &mut O,
    stop_rx: &mut watch::Receiver<bool>,
) -> std::result::Result<(), WebSocketRunError> {
    run_session_once(config, handler, observer, stop_rx).await
}

pub async fn run_websocket_subscription_runtime<
    H: WebSocketSessionHandler,
    O: WebSocketRuntimeObserver,
>(
    config: WebSocketRuntimeConfig,
    handler: &mut H,
    observer: &mut O,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let mut delay_secs = config.initial_reconnect_delay_secs;
    loop {
        match run_session_once(&config, handler, observer, stop_rx).await {
            Ok(()) => return Ok(()),
            Err(WebSocketRunError::Fatal(err)) => return Err(err),
            Err(WebSocketRunError::Retry(err)) => {
                let message = err.to_string();
                observer
                    .emit("error", None, Some(json!({ "message": message })))
                    .await?;
                observer
                    .update_status(Some("reconnecting"), Some(message.clone()), true)
                    .await?;
                observer
                    .emit("reconnect", None, Some(json!({ "delay_secs": delay_secs })))
                    .await?;
                if wait_for_stop_or_timeout(stop_rx, Duration::from_secs(delay_secs)).await {
                    close_as_stopped(observer, "stopped").await?;
                    return Ok(());
                }
                delay_secs = (delay_secs.saturating_mul(2)).min(config.max_reconnect_delay_secs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn raw_frame_handler_parses_json_text() {
        let mut handler = RawFrameHandler;
        let output = handler
            .on_text_frame("{\"value\":1}".to_string())
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert_eq!(output.data.unwrap()["value"], 1);
        assert_eq!(output.meta.unwrap()["frame_type"], "text_json");
    }

    #[tokio::test]
    async fn raw_frame_handler_keeps_plain_text_in_meta() {
        let mut handler = RawFrameHandler;
        let output = handler.on_text_frame("tick".to_string()).await.unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert!(output.data.is_none());
        assert_eq!(output.meta.unwrap()["text"], "tick");
    }

    #[tokio::test]
    async fn raw_frame_handler_encodes_binary_as_base64() {
        let mut handler = RawFrameHandler;
        let output = handler.on_binary_frame(vec![1, 2, 3]).await.unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert!(output.data.is_none());
        assert_eq!(output.meta.unwrap()["base64"], "AQID");
    }

    #[test]
    fn ensure_rustls_crypto_provider_installs_default() {
        ensure_rustls_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
