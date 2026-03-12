use crate::subscription_websocket::{
    WebSocketHandlerAction, WebSocketHandlerOutput, WebSocketSessionHandler,
};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

// Each daemon job owns exactly one GraphQL subscription, so a fixed wire id is enough here.
const DEFAULT_SUBSCRIPTION_ID: &str = "1";

#[derive(Debug, Clone)]
pub struct GraphQLSubscriptionConfig {
    pub operation_id: String,
    pub query: String,
    pub variables: Option<Value>,
}

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
    subscribed: bool,
}

impl GraphQLSubscriptionHandler {
    pub fn new(config: GraphQLSubscriptionConfig) -> Self {
        Self {
            config,
            subscribed: false,
        }
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
            "type": "subscribe",
            "payload": payload,
        })
        .to_string()
    }
}

#[async_trait]
impl WebSocketSessionHandler for GraphQLSubscriptionHandler {
    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| anyhow!("invalid graphql websocket message: {}", err))?;
        let message_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("graphql websocket message missing type"))?;

        match message_type {
            "connection_ack" => {
                if self.subscribed {
                    return Ok(WebSocketHandlerOutput {
                        action: WebSocketHandlerAction::Continue,
                        data: None,
                        meta: None,
                        outbound_text_frames: Vec::new(),
                        stop_reason: None,
                    });
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
            "next" => {
                if value.get("id").and_then(Value::as_str) != Some(DEFAULT_SUBSCRIPTION_ID) {
                    return Ok(WebSocketHandlerOutput {
                        action: WebSocketHandlerAction::Continue,
                        data: None,
                        meta: None,
                        outbound_text_frames: Vec::new(),
                        stop_reason: None,
                    });
                }
                let payload = value.get("payload").cloned().unwrap_or(Value::Null);
                let data = payload.get("data").cloned();
                let mut meta = json!({
                    "operation_id": self.config.operation_id,
                    "message_type": "next",
                });
                if let Some(errors) = payload.get("errors") {
                    meta["errors"] = errors.clone();
                }
                if let Some(extensions) = payload.get("extensions") {
                    meta["extensions"] = extensions.clone();
                }
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data,
                    meta: Some(meta),
                    outbound_text_frames: Vec::new(),
                    stop_reason: None,
                })
            }
            "complete" => {
                if value.get("id").and_then(Value::as_str) != Some(DEFAULT_SUBSCRIPTION_ID) {
                    return Ok(WebSocketHandlerOutput {
                        action: WebSocketHandlerAction::Continue,
                        data: None,
                        meta: None,
                        outbound_text_frames: Vec::new(),
                        stop_reason: None,
                    });
                }
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Stop,
                    data: None,
                    meta: None,
                    outbound_text_frames: Vec::new(),
                    stop_reason: Some("complete".to_string()),
                })
            }
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
            "pong" => Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
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
        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: None,
            outbound_text_frames: Vec::new(),
            stop_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn graphql_handler_sends_subscribe_after_ack() {
        let mut handler = GraphQLSubscriptionHandler::new(GraphQLSubscriptionConfig {
            operation_id: "subscription/messageAdded".to_string(),
            query: "subscription { messageAdded { id } }".to_string(),
            variables: Some(json!({"roomId":"abc"})),
        });

        let output = handler
            .on_text_frame(json!({"type":"connection_ack"}).to_string())
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert_eq!(output.outbound_text_frames.len(), 1);
        let subscribe: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(subscribe["type"], "subscribe");
        assert_eq!(subscribe["payload"]["variables"]["roomId"], "abc");
    }

    #[tokio::test]
    async fn graphql_handler_maps_next_payload_to_data_event() {
        let mut handler = GraphQLSubscriptionHandler::new(GraphQLSubscriptionConfig {
            operation_id: "subscription/messageAdded".to_string(),
            query: "subscription { messageAdded { id } }".to_string(),
            variables: None,
        });

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
        assert_eq!(output.meta.unwrap()["extensions"]["trace"], "ok");
    }

    #[tokio::test]
    async fn graphql_handler_complete_stops_with_complete_reason() {
        let mut handler = GraphQLSubscriptionHandler::new(GraphQLSubscriptionConfig {
            operation_id: "subscription/messageAdded".to_string(),
            query: "subscription { messageAdded { id } }".to_string(),
            variables: None,
        });

        let output = handler
            .on_text_frame(json!({"id":"1","type":"complete"}).to_string())
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Stop);
        assert_eq!(output.stop_reason.as_deref(), Some("complete"));
    }

    #[tokio::test]
    async fn graphql_handler_ignores_complete_for_other_subscription_id() {
        let mut handler = GraphQLSubscriptionHandler::new(GraphQLSubscriptionConfig {
            operation_id: "subscription/messageAdded".to_string(),
            query: "subscription { messageAdded { id } }".to_string(),
            variables: None,
        });

        let output = handler
            .on_text_frame(json!({"id":"2","type":"complete"}).to_string())
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert!(output.stop_reason.is_none());
    }
}
