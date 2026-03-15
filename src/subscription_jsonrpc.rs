use crate::subscription_websocket::{
    WebSocketHandlerAction, WebSocketHandlerOutput, WebSocketOpenMeta, WebSocketSessionHandler,
    WebSocketStopOutput,
};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

const SUBSCRIBE_REQUEST_ID: i64 = 1;
const UNSUBSCRIBE_REQUEST_ID: i64 = 2;

#[derive(Debug, Clone)]
pub struct JsonRpcSubscriptionConfig {
    pub operation_id: String,
    pub unsubscribe_operation_id: Option<String>,
    pub params: Option<Value>,
}

pub fn resolve_jsonrpc_unsubscribe_operation(operation_id: &str) -> Result<Option<String>> {
    if let Some(prefix) = operation_id.strip_suffix("_subscribe") {
        return Ok(Some(format!("{prefix}_unsubscribe")));
    }
    if operation_id.contains("unsubscribe") {
        bail!(
            "JSON-RPC subscription operation '{}' cannot be an unsubscribe method",
            operation_id
        );
    }
    if operation_id.contains("subscribe") {
        return Ok(None);
    }
    bail!(
        "JSON-RPC subscription operation '{}' must contain 'subscribe'",
        operation_id
    );
}

pub struct JsonRpcSubscriptionHandler {
    config: JsonRpcSubscriptionConfig,
    subscription_id: Option<Value>,
}

impl JsonRpcSubscriptionHandler {
    pub fn new(config: JsonRpcSubscriptionConfig) -> Self {
        Self {
            config,
            subscription_id: None,
        }
    }

    pub fn subscribe_message(&self) -> String {
        let mut payload = json!({
            "jsonrpc": "2.0",
            "id": SUBSCRIBE_REQUEST_ID,
            "method": self.config.operation_id,
        });
        if let Some(params) = &self.config.params {
            payload["params"] = params.clone();
        }
        payload.to_string()
    }

    fn unsubscribe_message(&self) -> Option<String> {
        let subscription_id = self.subscription_id.as_ref()?;
        let unsubscribe_operation_id = self.config.unsubscribe_operation_id.as_ref()?;
        Some(
            json!({
                "jsonrpc": "2.0",
                "id": UNSUBSCRIBE_REQUEST_ID,
                "method": unsubscribe_operation_id,
                "params": [subscription_id.clone()],
            })
            .to_string(),
        )
    }
}

#[async_trait]
impl WebSocketSessionHandler for JsonRpcSubscriptionHandler {
    async fn on_open(&mut self, _meta: &WebSocketOpenMeta) -> Result<WebSocketHandlerAction> {
        self.subscription_id = None;
        Ok(WebSocketHandlerAction::Continue)
    }

    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| anyhow!("invalid JSON-RPC websocket message: {}", err))?;

        if let Some(error) = value.get("error") {
            return Err(anyhow!("json-rpc subscription error: {}", error));
        }

        if value.get("id") == Some(&json!(SUBSCRIBE_REQUEST_ID)) {
            let subscription_id = value
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow!("json-rpc subscribe response missing result"))?;
            self.subscription_id = Some(subscription_id);
            return Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
                outbound_binary_frames: Vec::new(),
                stop_reason: None,
            });
        }

        if value.get("id") == Some(&json!(UNSUBSCRIBE_REQUEST_ID)) {
            return Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
                outbound_binary_frames: Vec::new(),
                stop_reason: None,
            });
        }

        let Some(subscription_id) = self.subscription_id.as_ref() else {
            return Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
                outbound_binary_frames: Vec::new(),
                stop_reason: None,
            });
        };

        let Some(params) = value.get("params") else {
            return Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
                outbound_binary_frames: Vec::new(),
                stop_reason: None,
            });
        };
        if params.get("subscription") != Some(subscription_id) {
            return Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
                outbound_binary_frames: Vec::new(),
                stop_reason: None,
            });
        }

        let data = params.get("result").cloned();
        let meta = Some(json!({
            "operation_id": self.config.operation_id,
            "notification_method": value.get("method").cloned().unwrap_or(Value::Null),
            "subscription_id": subscription_id,
        }));

        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data,
            meta,
            outbound_text_frames: Vec::new(),
            outbound_binary_frames: Vec::new(),
            stop_reason: None,
        })
    }

    async fn on_binary_frame(&mut self, _bytes: Vec<u8>) -> Result<WebSocketHandlerOutput> {
        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: None,
            outbound_text_frames: Vec::new(),
            outbound_binary_frames: Vec::new(),
            stop_reason: None,
        })
    }

    async fn on_stop_requested(&mut self) -> Result<WebSocketStopOutput> {
        Ok(WebSocketStopOutput {
            outbound_text_frames: self.unsubscribe_message().into_iter().collect(),
            outbound_binary_frames: Vec::new(),
            stop_reason: Some("stopped".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_unsubscribe_operation_from_subscribe_operation() {
        assert_eq!(
            resolve_jsonrpc_unsubscribe_operation("eth_subscribe").unwrap(),
            Some("eth_unsubscribe".to_string())
        );
    }

    #[test]
    fn resolves_sui_style_subscribe_without_unsubscribe() {
        assert_eq!(
            resolve_jsonrpc_unsubscribe_operation("suix_subscribeEvent").unwrap(),
            None
        );
    }

    #[test]
    fn rejects_non_subscribe_operation_name() {
        let err = resolve_jsonrpc_unsubscribe_operation("watch_heads").unwrap_err();
        assert!(err.to_string().contains("contain 'subscribe'"));
    }

    #[test]
    fn rejects_unsubscribe_operation_name() {
        let err = resolve_jsonrpc_unsubscribe_operation("eth_unsubscribe").unwrap_err();
        assert!(err.to_string().contains("cannot be an unsubscribe method"));
    }

    #[tokio::test]
    async fn subscribe_message_keeps_raw_params_shape() {
        let handler = JsonRpcSubscriptionHandler::new(JsonRpcSubscriptionConfig {
            operation_id: "eth_subscribe".to_string(),
            unsubscribe_operation_id: Some("eth_unsubscribe".to_string()),
            params: Some(json!(["newHeads"])),
        });

        let message: Value = serde_json::from_str(&handler.subscribe_message()).unwrap();
        assert_eq!(message["method"], "eth_subscribe");
        assert_eq!(message["params"][0], "newHeads");
    }

    #[tokio::test]
    async fn handler_routes_matching_notification_result() {
        let mut handler = JsonRpcSubscriptionHandler::new(JsonRpcSubscriptionConfig {
            operation_id: "eth_subscribe".to_string(),
            unsubscribe_operation_id: Some("eth_unsubscribe".to_string()),
            params: None,
        });
        handler
            .on_text_frame(
                json!({
                    "jsonrpc": "2.0",
                    "id": SUBSCRIBE_REQUEST_ID,
                    "result": "sub-1"
                })
                .to_string(),
            )
            .await
            .unwrap();

        let output = handler
            .on_text_frame(
                json!({
                    "jsonrpc": "2.0",
                    "method": "eth_subscription",
                    "params": {
                        "subscription": "sub-1",
                        "result": {"number":"0x1"}
                    }
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert_eq!(output.data.unwrap()["number"], "0x1");
        assert_eq!(output.meta.unwrap()["subscription_id"], "sub-1");
    }

    #[tokio::test]
    async fn handler_ignores_notification_for_other_subscription() {
        let mut handler = JsonRpcSubscriptionHandler::new(JsonRpcSubscriptionConfig {
            operation_id: "eth_subscribe".to_string(),
            unsubscribe_operation_id: Some("eth_unsubscribe".to_string()),
            params: None,
        });
        handler
            .on_text_frame(
                json!({
                    "jsonrpc": "2.0",
                    "id": SUBSCRIBE_REQUEST_ID,
                    "result": "sub-1"
                })
                .to_string(),
            )
            .await
            .unwrap();

        let output = handler
            .on_text_frame(
                json!({
                    "jsonrpc": "2.0",
                    "method": "eth_subscription",
                    "params": {
                        "subscription": "sub-2",
                        "result": {"number":"0x1"}
                    }
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert!(output.data.is_none());
    }

    #[tokio::test]
    async fn stop_requested_emits_unsubscribe_frame() {
        let mut handler = JsonRpcSubscriptionHandler::new(JsonRpcSubscriptionConfig {
            operation_id: "eth_subscribe".to_string(),
            unsubscribe_operation_id: Some("eth_unsubscribe".to_string()),
            params: None,
        });
        handler
            .on_text_frame(
                json!({
                    "jsonrpc": "2.0",
                    "id": SUBSCRIBE_REQUEST_ID,
                    "result": "sub-1"
                })
                .to_string(),
            )
            .await
            .unwrap();

        let output = handler.on_stop_requested().await.unwrap();
        let message: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(message["method"], "eth_unsubscribe");
        assert_eq!(message["params"][0], "sub-1");
    }

    #[tokio::test]
    async fn stop_requested_without_unsubscribe_sends_no_cleanup_frame() {
        let mut handler = JsonRpcSubscriptionHandler::new(JsonRpcSubscriptionConfig {
            operation_id: "suix_subscribeEvent".to_string(),
            unsubscribe_operation_id: None,
            params: Some(json!([{ "Package": "0x2" }])),
        });
        handler
            .on_text_frame(
                json!({
                    "jsonrpc": "2.0",
                    "id": SUBSCRIBE_REQUEST_ID,
                    "result": "sub-1"
                })
                .to_string(),
            )
            .await
            .unwrap();

        let output = handler.on_stop_requested().await.unwrap();
        assert!(output.outbound_text_frames.is_empty());
        assert_eq!(output.stop_reason.as_deref(), Some("stopped"));
    }
}
