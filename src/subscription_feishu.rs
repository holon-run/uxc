use crate::auth::Profile;
use crate::subscription_websocket::{
    WebSocketHandlerAction, WebSocketHandlerOutput, WebSocketOpenMeta, WebSocketSessionHandler,
    WebSocketStopOutput,
};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use prost::Message;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const FEISHU_FRAME_METHOD_CONTROL: i32 = 0;
const FEISHU_FRAME_METHOD_DATA: i32 = 1;
const FEISHU_TYPE_EVENT: &str = "event";
const FEISHU_TYPE_PING: &str = "ping";
const FEISHU_TYPE_PONG: &str = "pong";
const FEISHU_HEADER_TYPE: &str = "type";
const FEISHU_HEADER_MESSAGE_ID: &str = "message_id";
const FEISHU_HEADER_SUM: &str = "sum";
const FEISHU_HEADER_SEQ: &str = "seq";
const FEISHU_HEADER_TRACE_ID: &str = "trace_id";
const FEISHU_HEADER_BIZ_RT: &str = "biz_rt";

#[derive(Debug, Clone)]
pub struct FeishuLongConnectionOpenResponse {
    pub websocket_url: String,
    pub ping_interval_secs: u64,
    pub reconnect_count: i64,
    pub reconnect_interval_secs: u64,
    pub reconnect_nonce_secs: u64,
    pub service_id: i32,
}

#[derive(Debug, Clone)]
pub struct FeishuLongConnectionRuntimeConfig {
    pub app_id: String,
    pub app_secret: String,
}

pub fn derive_feishu_ws_config_endpoint(endpoint: &str) -> Result<String> {
    let mut url = url::Url::parse(endpoint).context("invalid Feishu long-connection endpoint")?;
    match url.scheme() {
        "http" | "https" => {}
        other => bail!(
            "feishu-long-connection transport requires an http:// or https:// Feishu/Lark API endpoint, got '{}'",
            other
        ),
    }
    url.set_path("/callback/ws/endpoint");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub fn parse_feishu_long_connection_open_response(
    body: &Value,
) -> Result<FeishuLongConnectionOpenResponse> {
    if body.get("code").and_then(Value::as_i64) != Some(0) {
        let message = body
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        bail!("Feishu long-connection open failed: {}", message);
    }
    let data = body
        .get("data")
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing data"))?;
    let websocket_url = data
        .get("URL")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing data.URL"))?;
    let client_config = data
        .get("ClientConfig")
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing data.ClientConfig"))?;
    let ping_interval_secs = client_config
        .get("PingInterval")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing PingInterval"))?;
    let reconnect_count = client_config
        .get("ReconnectCount")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing ReconnectCount"))?;
    let reconnect_interval_secs = client_config
        .get("ReconnectInterval")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing ReconnectInterval"))?;
    let reconnect_nonce_secs = client_config
        .get("ReconnectNonce")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Feishu long-connection open response missing ReconnectNonce"))?;
    let parsed_url =
        url::Url::parse(websocket_url).context("invalid Feishu long-connection websocket url")?;
    let service_id = parsed_url
        .query_pairs()
        .find(|(key, _)| key == "service_id")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("Feishu long-connection websocket URL missing service_id"))?
        .parse::<i32>()
        .context("invalid Feishu long-connection service_id")?;
    Ok(FeishuLongConnectionOpenResponse {
        websocket_url: websocket_url.to_string(),
        ping_interval_secs,
        reconnect_count,
        reconnect_interval_secs,
        reconnect_nonce_secs,
        service_id,
    })
}

pub fn resolve_feishu_long_connection_runtime_config(
    auth_profile: &Profile,
) -> Result<FeishuLongConnectionRuntimeConfig> {
    let app_id = auth_profile
        .resolve_field_value("app_id")?
        .ok_or_else(|| anyhow!("feishu-long-connection requires credential field 'app_id'"))?;
    let app_secret = auth_profile
        .resolve_field_value("app_secret")?
        .ok_or_else(|| anyhow!("feishu-long-connection requires credential field 'app_secret'"))?;
    Ok(FeishuLongConnectionRuntimeConfig { app_id, app_secret })
}

#[derive(Clone, PartialEq, Message)]
struct FeishuPbHeader {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(string, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, Message)]
struct FeishuPbFrame {
    #[prost(uint64, tag = "1")]
    seq_id: u64,
    #[prost(uint64, tag = "2")]
    log_id: u64,
    #[prost(int32, tag = "3")]
    service: i32,
    #[prost(int32, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<FeishuPbHeader>,
    #[prost(string, tag = "6")]
    payload_encoding: String,
    #[prost(string, tag = "7")]
    payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    payload: Vec<u8>,
    #[prost(string, tag = "9")]
    log_id_new: String,
}

#[derive(Debug, Clone)]
struct FeishuFrameChunk {
    frame: FeishuPbFrame,
    headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct FeishuEventAssembly {
    created_at: Instant,
    trace_id: Option<String>,
    total_parts: usize,
    parts: Vec<Option<Vec<u8>>>,
}

type FeishuMergedEventPayload = (Value, String, Option<String>, usize);

pub struct FeishuLongConnectionHandler {
    service_id: i32,
    ping_interval: Duration,
    next_ping_at: Option<Instant>,
    event_cache: HashMap<String, FeishuEventAssembly>,
}

impl FeishuLongConnectionHandler {
    pub fn new(service_id: i32, ping_interval_secs: u64) -> Self {
        Self {
            service_id,
            ping_interval: Duration::from_secs(ping_interval_secs.max(1)),
            next_ping_at: None,
            event_cache: HashMap::new(),
        }
    }

    fn make_control_frame(&self, message_type: &str, payload: Vec<u8>) -> Vec<u8> {
        FeishuPbFrame {
            seq_id: 0,
            log_id: 0,
            service: self.service_id,
            method: FEISHU_FRAME_METHOD_CONTROL,
            headers: vec![FeishuPbHeader {
                key: FEISHU_HEADER_TYPE.to_string(),
                value: message_type.to_string(),
            }],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload,
            log_id_new: String::new(),
        }
        .encode_to_vec()
    }

    fn parse_binary_frame(bytes: &[u8]) -> Result<FeishuFrameChunk> {
        let frame = FeishuPbFrame::decode(bytes).context("invalid Feishu protobuf frame")?;
        let headers = frame
            .headers
            .iter()
            .map(|header| (header.key.clone(), header.value.clone()))
            .collect::<HashMap<_, _>>();
        Ok(FeishuFrameChunk { frame, headers })
    }

    fn type_header(headers: &HashMap<String, String>) -> Option<&str> {
        headers.get(FEISHU_HEADER_TYPE).map(String::as_str)
    }

    fn handle_control_frame(&mut self, chunk: FeishuFrameChunk) -> Result<WebSocketHandlerOutput> {
        let message_type = Self::type_header(&chunk.headers).unwrap_or_default();
        match message_type {
            FEISHU_TYPE_PONG => {
                if !chunk.frame.payload.is_empty() {
                    if let Ok(value) = serde_json::from_slice::<Value>(&chunk.frame.payload) {
                        if let Some(seconds) = value.get("PingInterval").and_then(Value::as_u64) {
                            self.ping_interval = Duration::from_secs(seconds.max(1));
                        }
                    }
                }
                self.next_ping_at = Some(Instant::now() + self.ping_interval);
                Ok(WebSocketHandlerOutput::continue_empty())
            }
            FEISHU_TYPE_PING => Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: None,
                meta: None,
                outbound_text_frames: Vec::new(),
                outbound_binary_frames: vec![self.make_control_frame(FEISHU_TYPE_PONG, vec![])],
                stop_reason: None,
            }),
            _ => Ok(WebSocketHandlerOutput::continue_empty()),
        }
    }

    fn merge_event_payload(
        &mut self,
        headers: &HashMap<String, String>,
        payload: Vec<u8>,
    ) -> Result<Option<FeishuMergedEventPayload>> {
        let message_id = headers
            .get(FEISHU_HEADER_MESSAGE_ID)
            .cloned()
            .ok_or_else(|| anyhow!("Feishu event frame missing message_id"))?;
        let total_parts = headers
            .get(FEISHU_HEADER_SUM)
            .ok_or_else(|| anyhow!("Feishu event frame missing sum"))?
            .parse::<usize>()
            .context("invalid Feishu event frame sum")?;
        let seq = headers
            .get(FEISHU_HEADER_SEQ)
            .ok_or_else(|| anyhow!("Feishu event frame missing seq"))?
            .parse::<usize>()
            .context("invalid Feishu event frame seq")?;
        let trace_id = headers.get(FEISHU_HEADER_TRACE_ID).cloned();

        self.event_cache
            .retain(|_, entry| entry.created_at.elapsed() < Duration::from_secs(10));

        let entry = self
            .event_cache
            .entry(message_id.clone())
            .or_insert_with(|| FeishuEventAssembly {
                created_at: Instant::now(),
                trace_id: trace_id.clone(),
                total_parts,
                parts: vec![None; total_parts],
            });
        if entry.total_parts != total_parts {
            bail!("Feishu event frame changed total part count mid-stream");
        }
        if seq >= entry.parts.len() {
            bail!(
                "Feishu event frame seq {} out of range {}",
                seq,
                entry.parts.len()
            );
        }
        entry.parts[seq] = Some(payload);

        if entry.parts.iter().all(|part| part.is_some()) {
            let mut merged = Vec::new();
            for part in &entry.parts {
                merged.extend_from_slice(part.as_ref().expect("parts checked above"));
            }
            let value: Value =
                serde_json::from_slice(&merged).context("invalid Feishu event payload JSON")?;
            let trace_id = entry.trace_id.clone();
            self.event_cache.remove(&message_id);
            Ok(Some((value, message_id, trace_id, total_parts)))
        } else {
            Ok(None)
        }
    }

    fn make_event_ack_frame(
        &self,
        frame: &FeishuPbFrame,
        _headers: &HashMap<String, String>,
        elapsed_ms: u128,
    ) -> Vec<u8> {
        let mut response_headers = frame.headers.clone();
        response_headers.push(FeishuPbHeader {
            key: FEISHU_HEADER_BIZ_RT.to_string(),
            value: elapsed_ms.to_string(),
        });
        FeishuPbFrame {
            seq_id: frame.seq_id,
            log_id: frame.log_id,
            service: frame.service,
            method: frame.method,
            headers: response_headers,
            payload_encoding: frame.payload_encoding.clone(),
            payload_type: frame.payload_type.clone(),
            payload: br#"{"code":200}"#.to_vec(),
            log_id_new: frame.log_id_new.clone(),
        }
        .encode_to_vec()
    }
}

#[async_trait]
impl WebSocketSessionHandler for FeishuLongConnectionHandler {
    async fn on_open(&mut self, _meta: &WebSocketOpenMeta) -> Result<WebSocketHandlerAction> {
        self.next_ping_at = Some(Instant::now() + self.ping_interval);
        Ok(WebSocketHandlerAction::Continue)
    }

    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: Some(json!({
                "frame_type": "unexpected_text",
                "text": text,
            })),
            outbound_text_frames: Vec::new(),
            outbound_binary_frames: Vec::new(),
            stop_reason: None,
        })
    }

    async fn on_binary_frame(&mut self, bytes: Vec<u8>) -> Result<WebSocketHandlerOutput> {
        let started_at = Instant::now();
        let chunk = Self::parse_binary_frame(&bytes)?;
        match chunk.frame.method {
            FEISHU_FRAME_METHOD_CONTROL => self.handle_control_frame(chunk),
            FEISHU_FRAME_METHOD_DATA => {
                if Self::type_header(&chunk.headers) != Some(FEISHU_TYPE_EVENT) {
                    return Ok(WebSocketHandlerOutput::continue_empty());
                }
                let ack = self.make_event_ack_frame(
                    &chunk.frame,
                    &chunk.headers,
                    started_at.elapsed().as_millis(),
                );
                let Some((value, message_id, trace_id, total_parts)) =
                    self.merge_event_payload(&chunk.headers, chunk.frame.payload.clone())?
                else {
                    return Ok(WebSocketHandlerOutput {
                        action: WebSocketHandlerAction::Continue,
                        data: None,
                        meta: Some(json!({
                            "frame_type": "event_chunk",
                            "message_id": chunk.headers.get(FEISHU_HEADER_MESSAGE_ID).cloned(),
                            "trace_id": chunk.headers.get(FEISHU_HEADER_TRACE_ID).cloned(),
                        })),
                        outbound_text_frames: Vec::new(),
                        outbound_binary_frames: vec![ack],
                        stop_reason: None,
                    });
                };
                let event_type = value
                    .get("event_type")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let meta = json!({
                    "frame_type": "protobuf_event",
                    "message_type": FEISHU_TYPE_EVENT,
                    "message_id": message_id,
                    "trace_id": trace_id,
                    "event_type": event_type,
                    "parts": total_parts,
                });
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: Some(value),
                    meta: Some(meta),
                    outbound_text_frames: Vec::new(),
                    outbound_binary_frames: vec![ack],
                    stop_reason: None,
                })
            }
            other => bail!("unsupported Feishu frame method {}", other),
        }
    }

    fn next_wakeup_at(&self) -> Option<tokio::time::Instant> {
        self.next_ping_at.map(tokio::time::Instant::from_std)
    }

    async fn on_wakeup(&mut self) -> Result<WebSocketHandlerOutput> {
        self.next_ping_at = Some(Instant::now() + self.ping_interval);
        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: None,
            outbound_text_frames: Vec::new(),
            outbound_binary_frames: vec![self.make_control_frame(FEISHU_TYPE_PING, vec![])],
            stop_reason: None,
        })
    }

    async fn on_stop_requested(&mut self) -> Result<WebSocketStopOutput> {
        Ok(WebSocketStopOutput {
            outbound_text_frames: Vec::new(),
            outbound_binary_frames: Vec::new(),
            stop_reason: Some("stopped".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_frame(payload: Value) -> Vec<u8> {
        FeishuPbFrame {
            seq_id: 1,
            log_id: 2,
            service: 3,
            method: FEISHU_FRAME_METHOD_DATA,
            headers: vec![
                FeishuPbHeader {
                    key: FEISHU_HEADER_TYPE.to_string(),
                    value: FEISHU_TYPE_EVENT.to_string(),
                },
                FeishuPbHeader {
                    key: FEISHU_HEADER_MESSAGE_ID.to_string(),
                    value: "mid".to_string(),
                },
                FeishuPbHeader {
                    key: FEISHU_HEADER_SUM.to_string(),
                    value: "1".to_string(),
                },
                FeishuPbHeader {
                    key: FEISHU_HEADER_SEQ.to_string(),
                    value: "0".to_string(),
                },
                FeishuPbHeader {
                    key: FEISHU_HEADER_TRACE_ID.to_string(),
                    value: "trace".to_string(),
                },
            ],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: serde_json::to_vec(&payload).unwrap(),
            log_id_new: String::new(),
        }
        .encode_to_vec()
    }

    #[test]
    fn derives_feishu_ws_config_endpoint_from_api_base() {
        assert_eq!(
            derive_feishu_ws_config_endpoint("https://open.feishu.cn/open-apis").unwrap(),
            "https://open.feishu.cn/callback/ws/endpoint"
        );
    }

    #[test]
    fn parses_feishu_open_response() {
        let parsed = parse_feishu_long_connection_open_response(&json!({
            "code": 0,
            "data": {
                "URL": "wss://msg-frontier.feishu.cn/ws/v2?service_id=33554678",
                "ClientConfig": {
                    "PingInterval": 90,
                    "ReconnectCount": -1,
                    "ReconnectInterval": 90,
                    "ReconnectNonce": 25
                }
            }
        }))
        .unwrap();
        assert_eq!(parsed.service_id, 33_554_678);
        assert_eq!(parsed.ping_interval_secs, 90);
    }

    #[tokio::test]
    async fn handler_emits_event_and_binary_ack() {
        let mut handler = FeishuLongConnectionHandler::new(3, 90);
        let output = handler
            .on_binary_frame(event_frame(json!({
                "event_type": "im.message.receive_v1",
                "message": { "message_id": "om_1" }
            })))
            .await
            .unwrap();

        assert_eq!(output.action, WebSocketHandlerAction::Continue);
        assert_eq!(
            output.data.as_ref().unwrap()["event_type"],
            "im.message.receive_v1"
        );
        assert_eq!(output.meta.as_ref().unwrap()["trace_id"], "trace");
        assert_eq!(output.outbound_binary_frames.len(), 1);

        let ack = FeishuPbFrame::decode(output.outbound_binary_frames[0].as_slice()).unwrap();
        assert_eq!(ack.method, FEISHU_FRAME_METHOD_DATA);
        assert!(ack
            .headers
            .iter()
            .any(|header| header.key == FEISHU_HEADER_BIZ_RT));
        assert_eq!(
            serde_json::from_slice::<Value>(&ack.payload).unwrap()["code"],
            200
        );
    }

    #[tokio::test]
    async fn handler_schedules_binary_ping_on_wakeup() {
        let mut handler = FeishuLongConnectionHandler::new(3, 90);
        let _ = handler
            .on_open(&WebSocketOpenMeta {
                redacted_url: "wss://example.com".to_string(),
                subprotocol: None,
            })
            .await
            .unwrap();

        let output = handler.on_wakeup().await.unwrap();
        assert_eq!(output.outbound_binary_frames.len(), 1);
        let ping = FeishuPbFrame::decode(output.outbound_binary_frames[0].as_slice()).unwrap();
        assert_eq!(ping.method, FEISHU_FRAME_METHOD_CONTROL);
        assert!(ping
            .headers
            .iter()
            .any(|header| header.key == FEISHU_HEADER_TYPE && header.value == FEISHU_TYPE_PING));
    }
}
