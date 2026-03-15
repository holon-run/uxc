use crate::subscription_websocket::{
    WebSocketHandlerAction, WebSocketHandlerOutput, WebSocketOpenMeta, WebSocketSessionHandler,
    WebSocketStopOutput,
};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub fn derive_socket_mode_open_endpoint(endpoint: &str) -> Result<String> {
    let mut url = url::Url::parse(endpoint).context("invalid Slack Socket Mode endpoint")?;
    match url.scheme() {
        "http" | "https" => {}
        other => bail!(
            "Slack Socket Mode transport requires an http:// or https:// endpoint, got '{}'",
            other
        ),
    }

    let path = url.path().trim_end_matches('/');
    let new_path = if path.is_empty() || path == "/" {
        "/api/apps.connections.open".to_string()
    } else if path.ends_with("/apps.connections.open") {
        path.to_string()
    } else {
        format!("{path}/apps.connections.open")
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

#[derive(Debug, Clone)]
pub struct SlackSocketModeOpenResponse {
    pub websocket_url: String,
}

pub fn parse_socket_mode_open_response(body: &Value) -> Result<SlackSocketModeOpenResponse> {
    if body.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        bail!("Slack Socket Mode open failed: {}", error);
    }
    let websocket_url = body
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Slack Socket Mode open response missing url"))?;
    Ok(SlackSocketModeOpenResponse {
        websocket_url: websocket_url.to_string(),
    })
}

pub struct SlackSocketModeHandler;

impl SlackSocketModeHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SlackSocketModeHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WebSocketSessionHandler for SlackSocketModeHandler {
    async fn on_open(&mut self, _meta: &WebSocketOpenMeta) -> Result<WebSocketHandlerAction> {
        Ok(WebSocketHandlerAction::Continue)
    }

    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| anyhow!("invalid Slack Socket Mode message: {}", err))?;
        let message_type = value.get("type").and_then(Value::as_str);
        let envelope_id = value.get("envelope_id").and_then(Value::as_str);

        let action = if message_type == Some("disconnect") {
            WebSocketHandlerAction::Reconnect
        } else {
            WebSocketHandlerAction::Continue
        };

        let mut meta = json!({
            "frame_type": "text_json",
        });
        if let Some(message_type) = message_type {
            meta["message_type"] = json!(message_type);
        }
        if let Some(envelope_id) = envelope_id {
            meta["envelope_id"] = json!(envelope_id);
            meta["ack_sent"] = json!(true);
        }

        let outbound_text_frames = envelope_id
            .map(|value| json!({ "envelope_id": value }).to_string())
            .into_iter()
            .collect();

        Ok(WebSocketHandlerOutput {
            action,
            data: Some(value),
            meta: Some(meta),
            outbound_text_frames,
            stop_reason: if action == WebSocketHandlerAction::Reconnect {
                Some("disconnect".to_string())
            } else {
                None
            },
        })
    }

    async fn on_binary_frame(&mut self, _bytes: Vec<u8>) -> Result<WebSocketHandlerOutput> {
        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: None,
            outbound_text_frames: Vec::new(),
            stop_reason: None,
        })
    }

    async fn on_stop_requested(&mut self) -> Result<WebSocketStopOutput> {
        Ok(WebSocketStopOutput {
            outbound_text_frames: Vec::new(),
            stop_reason: Some("stopped".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_socket_mode_open_endpoint_from_api_base() {
        assert_eq!(
            derive_socket_mode_open_endpoint("https://slack.com/api").unwrap(),
            "https://slack.com/api/apps.connections.open"
        );
    }

    #[test]
    fn keeps_explicit_open_endpoint() {
        assert_eq!(
            derive_socket_mode_open_endpoint("https://slack.com/api/apps.connections.open")
                .unwrap(),
            "https://slack.com/api/apps.connections.open"
        );
    }

    #[test]
    fn parse_open_response_requires_ok_and_url() {
        let parsed = parse_socket_mode_open_response(&json!({
            "ok": true,
            "url": "wss://example.com/socket"
        }))
        .unwrap();
        assert_eq!(parsed.websocket_url, "wss://example.com/socket");

        let err = parse_socket_mode_open_response(&json!({"ok": false, "error": "invalid_auth"}))
            .unwrap_err();
        assert!(err.to_string().contains("invalid_auth"));
    }

    #[tokio::test]
    async fn handler_acks_enveloped_events() {
        let mut handler = SlackSocketModeHandler::new();
        let output = handler
            .on_text_frame(
                json!({
                    "envelope_id": "abc",
                    "type": "events_api",
                    "payload": {"event": {"type": "message"}}
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert_eq!(output.meta.as_ref().unwrap()["ack_sent"], true);
        let ack: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(ack["envelope_id"], "abc");
    }

    #[tokio::test]
    async fn handler_requests_reconnect_on_disconnect_frame() {
        let mut handler = SlackSocketModeHandler::new();
        let output = handler
            .on_text_frame(json!({"type":"disconnect","reason":"warning"}).to_string())
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Reconnect);
        assert_eq!(output.data.unwrap()["type"], "disconnect");
    }
}
