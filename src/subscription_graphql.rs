use crate::subscription_websocket::{
    WebSocketHandlerAction, WebSocketHandlerOutput, WebSocketOpenMeta, WebSocketSessionHandler,
    WebSocketStopOutput,
};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

// Each daemon job owns exactly one GraphQL subscription, so a fixed wire id is enough here.
const DEFAULT_SUBSCRIPTION_ID: &str = "1";
const MODERN_SUBPROTOCOL: &str = "graphql-transport-ws";
const LEGACY_SUBPROTOCOL: &str = "graphql-ws";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphQLWebSocketProfile {
    Modern,
    Legacy,
}

impl GraphQLWebSocketProfile {
    pub fn protocol_label(self) -> &'static str {
        match self {
            Self::Modern => "graphql_transport_ws",
            Self::Legacy => "graphql_ws_legacy",
        }
    }

    pub fn subprotocol(self) -> &'static str {
        match self {
            Self::Modern => MODERN_SUBPROTOCOL,
            Self::Legacy => LEGACY_SUBPROTOCOL,
        }
    }

    fn subscribe_message_type(self) -> &'static str {
        match self {
            Self::Modern => "subscribe",
            Self::Legacy => "start",
        }
    }

    fn stop_message_type(self) -> &'static str {
        match self {
            Self::Modern => "complete",
            Self::Legacy => "stop",
        }
    }

    fn data_message_type(self) -> &'static str {
        match self {
            Self::Modern => "next",
            Self::Legacy => "data",
        }
    }

    fn keepalive_message_type(self) -> Option<&'static str> {
        match self {
            Self::Modern => None,
            Self::Legacy => Some("ka"),
        }
    }

    pub fn alternate(self) -> Self {
        match self {
            Self::Modern => Self::Legacy,
            Self::Legacy => Self::Modern,
        }
    }

    fn recognizes_message_type(self, message_type: &str) -> bool {
        message_type == "connection_ack"
            || message_type == self.data_message_type()
            || message_type == self.stop_message_type()
            || message_type == "error"
            || message_type == "ping"
            || message_type == "pong"
            || self.keepalive_message_type() == Some(message_type)
    }
}

#[derive(Debug, Clone)]
pub struct GraphQLSubscriptionConfig {
    pub operation_id: String,
    pub query: String,
    pub variables: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct GraphQLProfileFallback {
    pub from: GraphQLWebSocketProfile,
    pub to: GraphQLWebSocketProfile,
    pub reason: String,
}

impl std::fmt::Display for GraphQLProfileFallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "graphql websocket profile {} is incompatible: {}; retry with {}",
            self.from.protocol_label(),
            self.reason,
            self.to.protocol_label()
        )
    }
}

impl std::error::Error for GraphQLProfileFallback {}

pub fn graphql_transport_init_message() -> String {
    json!({ "type": "connection_init" }).to_string()
}

pub fn derive_graphql_websocket_endpoint(endpoint: &str) -> Result<String> {
    if let Some(rest) = endpoint.strip_prefix("https://") {
        return Ok(format!("wss://{}", rest));
    }
    if let Some(rest) = endpoint.strip_prefix("http://") {
        return Ok(format!("ws://{}", rest));
    }
    bail!("GraphQL subscriptions require an http:// or https:// endpoint for schema discovery")
}

pub struct GraphQLSubscriptionHandler {
    config: GraphQLSubscriptionConfig,
    profile: GraphQLWebSocketProfile,
    subscribed: bool,
    received_data: bool,
}

impl GraphQLSubscriptionHandler {
    pub fn new(config: GraphQLSubscriptionConfig, profile: GraphQLWebSocketProfile) -> Self {
        Self {
            config,
            profile,
            subscribed: false,
            received_data: false,
        }
    }

    pub fn has_received_data(&self) -> bool {
        self.received_data
    }

    fn fallback_error(&self, reason: impl Into<String>) -> anyhow::Error {
        anyhow!(GraphQLProfileFallback {
            from: self.profile,
            to: self.profile.alternate(),
            reason: reason.into(),
        })
    }

    fn subscribe_message(&self) -> String {
        let mut payload = json!({
            "query": self.config.query,
        });
        if let Some(variables) = &self.config.variables {
            payload["variables"] = variables.clone();
        }
        json!({
            "id": DEFAULT_SUBSCRIPTION_ID,
            "type": self.profile.subscribe_message_type(),
            "payload": payload,
        })
        .to_string()
    }

    fn stop_message(&self) -> String {
        json!({
            "id": DEFAULT_SUBSCRIPTION_ID,
            "type": self.profile.stop_message_type(),
        })
        .to_string()
    }

    fn ignored_output() -> WebSocketHandlerOutput {
        WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: None,
            outbound_text_frames: Vec::new(),
            stop_reason: None,
        }
    }

    fn data_meta(&self, payload: &Value) -> Value {
        let mut meta = json!({
            "operation_id": self.config.operation_id,
            "message_type": self.profile.data_message_type(),
            "graphql_profile": self.profile.protocol_label(),
        });
        if let Some(errors) = payload.get("errors") {
            meta["errors"] = errors.clone();
        }
        if let Some(extensions) = payload.get("extensions") {
            meta["extensions"] = extensions.clone();
        }
        meta
    }
}

#[async_trait]
impl WebSocketSessionHandler for GraphQLSubscriptionHandler {
    async fn on_open(&mut self, meta: &WebSocketOpenMeta) -> Result<WebSocketHandlerAction> {
        if let Some(subprotocol) = meta.subprotocol.as_deref() {
            let trimmed = subprotocol.trim();
            if trimmed == self.profile.alternate().subprotocol() {
                return Err(
                    self.fallback_error(format!("server negotiated subprotocol '{}'", trimmed))
                );
            }
            if trimmed != self.profile.subprotocol() {
                return Err(anyhow!(
                    "unexpected graphql websocket subprotocol '{}'",
                    trimmed
                ));
            }
        }
        Ok(WebSocketHandlerAction::Continue)
    }

    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| anyhow!("invalid graphql websocket message: {}", err))?;
        let message_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("graphql websocket message missing type"))?;

        if !self.profile.recognizes_message_type(message_type)
            && self
                .profile
                .alternate()
                .recognizes_message_type(message_type)
        {
            return Err(self.fallback_error(format!(
                "received '{}' frame for legacy/alternate protocol",
                message_type
            )));
        }

        match message_type {
            "connection_ack" => {
                if self.subscribed {
                    return Ok(Self::ignored_output());
                }
                self.subscribed = true;
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: None,
                    meta: None,
                    outbound_text_frames: vec![self.subscribe_message()],
                    stop_reason: None,
                })
            }
            "next" | "data" => {
                if value.get("id").and_then(Value::as_str) != Some(DEFAULT_SUBSCRIPTION_ID) {
                    return Ok(Self::ignored_output());
                }
                let payload = value.get("payload").cloned().unwrap_or(Value::Null);
                self.received_data = true;
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: payload.get("data").cloned(),
                    meta: Some(self.data_meta(&payload)),
                    outbound_text_frames: Vec::new(),
                    stop_reason: None,
                })
            }
            "complete" => {
                if value.get("id").and_then(Value::as_str) != Some(DEFAULT_SUBSCRIPTION_ID) {
                    return Ok(Self::ignored_output());
                }
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Stop,
                    data: None,
                    meta: None,
                    outbound_text_frames: Vec::new(),
                    stop_reason: Some("complete".to_string()),
                })
            }
            "ka" | "pong" => Ok(Self::ignored_output()),
            "ping" => Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: vec![json!({
                    "type": "pong",
                    "payload": value.get("payload").cloned().unwrap_or(Value::Null),
                })
                .to_string()],
                stop_reason: None,
            }),
            "error" => Err(anyhow!(
                "graphql subscription error: {}",
                value.get("payload").cloned().unwrap_or(Value::Null)
            )),
            other => Err(anyhow!(
                "unsupported graphql websocket message type '{}'",
                other
            )),
        }
    }

    async fn on_binary_frame(&mut self, _bytes: Vec<u8>) -> Result<WebSocketHandlerOutput> {
        Ok(Self::ignored_output())
    }

    async fn on_stop_requested(&mut self) -> Result<WebSocketStopOutput> {
        Ok(WebSocketStopOutput {
            outbound_text_frames: vec![self.stop_message()],
            stop_reason: Some("stopped".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GraphQLSubscriptionConfig {
        GraphQLSubscriptionConfig {
            operation_id: "subscription/messageAdded".to_string(),
            query: "subscription { messageAdded { id } }".to_string(),
            variables: Some(json!({"roomId":"abc"})),
        }
    }

    #[tokio::test]
    async fn modern_handler_sends_subscribe_after_ack() {
        let mut handler =
            GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Modern);

        let output = handler
            .on_text_frame(json!({"type":"connection_ack"}).to_string())
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        let subscribe: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(subscribe["type"], "subscribe");
        assert_eq!(subscribe["payload"]["variables"]["roomId"], "abc");
    }

    #[tokio::test]
    async fn legacy_handler_sends_start_after_ack() {
        let mut handler =
            GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Legacy);

        let output = handler
            .on_text_frame(json!({"type":"connection_ack"}).to_string())
            .await
            .unwrap();

        let subscribe: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(subscribe["type"], "start");
    }

    #[tokio::test]
    async fn modern_handler_maps_next_payload_to_data_event() {
        let mut handler =
            GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Modern);

        let output = handler
            .on_text_frame(
                json!({
                    "id":"1",
                    "type":"next",
                    "payload":{
                        "data":{"messageAdded":{"id":"m1"}},
                        "extensions":{"trace":"ok"}
                    }
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert_eq!(output.data.unwrap()["messageAdded"]["id"], "m1");
        let meta = output.meta.unwrap();
        assert_eq!(meta["extensions"]["trace"], "ok");
        assert_eq!(meta["graphql_profile"], "graphql_transport_ws");
    }

    #[tokio::test]
    async fn legacy_handler_maps_data_payload_and_ignores_keepalive() {
        let mut handler =
            GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Legacy);

        let keepalive = handler
            .on_text_frame(json!({"type":"ka"}).to_string())
            .await
            .unwrap();
        assert!(keepalive.data.is_none());
        assert!(keepalive.meta.is_none());

        let output = handler
            .on_text_frame(
                json!({
                    "id":"1",
                    "type":"data",
                    "payload":{
                        "data":{"messageAdded":{"id":"m1"}}
                    }
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert_eq!(output.data.unwrap()["messageAdded"]["id"], "m1");
        assert_eq!(output.meta.unwrap()["graphql_profile"], "graphql_ws_legacy");
    }

    #[tokio::test]
    async fn handler_stop_message_depends_on_profile() {
        let mut modern = GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Modern);
        let modern_stop = modern.on_stop_requested().await.unwrap();
        let modern_frame: Value =
            serde_json::from_str(&modern_stop.outbound_text_frames[0]).unwrap();
        assert_eq!(modern_frame["type"], "complete");

        let mut legacy = GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Legacy);
        let legacy_stop = legacy.on_stop_requested().await.unwrap();
        let legacy_frame: Value =
            serde_json::from_str(&legacy_stop.outbound_text_frames[0]).unwrap();
        assert_eq!(legacy_frame["type"], "stop");
    }

    #[tokio::test]
    async fn handler_requests_fallback_for_alternate_message_type() {
        let mut handler =
            GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Modern);

        let err = handler
            .on_text_frame(
                json!({"id":"1","type":"data","payload":{"data":{"messageAdded":{"id":"m1"}}}})
                    .to_string(),
            )
            .await
            .unwrap_err();

        let fallback = err.downcast_ref::<GraphQLProfileFallback>().unwrap();
        assert_eq!(fallback.from, GraphQLWebSocketProfile::Modern);
        assert_eq!(fallback.to, GraphQLWebSocketProfile::Legacy);
    }

    #[tokio::test]
    async fn handler_requests_fallback_for_alternate_subprotocol() {
        let mut handler =
            GraphQLSubscriptionHandler::new(config(), GraphQLWebSocketProfile::Modern);
        let err = handler
            .on_open(&WebSocketOpenMeta {
                redacted_url: "ws://example.test/graphql".to_string(),
                subprotocol: Some("graphql-ws".to_string()),
            })
            .await
            .unwrap_err();

        let fallback = err.downcast_ref::<GraphQLProfileFallback>().unwrap();
        assert_eq!(fallback.to, GraphQLWebSocketProfile::Legacy);
    }
}
