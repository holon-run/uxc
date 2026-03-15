use crate::subscription_websocket::{
    WebSocketCloseMeta, WebSocketHandlerAction, WebSocketHandlerOutput, WebSocketOpenMeta,
    WebSocketSessionHandler, WebSocketStopOutput,
};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::Instant;

const DISCORD_GATEWAY_HELLO: i64 = 10;
const DISCORD_GATEWAY_DISPATCH: i64 = 0;
const DISCORD_GATEWAY_HEARTBEAT: i64 = 1;
const DISCORD_GATEWAY_IDENTIFY: i64 = 2;
const DISCORD_GATEWAY_RESUME: i64 = 6;
const DISCORD_GATEWAY_RECONNECT: i64 = 7;
const DISCORD_GATEWAY_INVALID_SESSION: i64 = 9;
const DISCORD_GATEWAY_HEARTBEAT_ACK: i64 = 11;

pub const DISCORD_INTENT_GUILDS: u64 = 1 << 0;
pub const DISCORD_INTENT_GUILD_MESSAGES: u64 = 1 << 9;
pub const DISCORD_INTENT_DIRECT_MESSAGES: u64 = 1 << 12;
pub const DISCORD_INTENT_MESSAGE_CONTENT: u64 = 1 << 15;
pub const DISCORD_DEFAULT_MESSAGE_INTENTS: u64 = DISCORD_INTENT_GUILDS
    | DISCORD_INTENT_GUILD_MESSAGES
    | DISCORD_INTENT_DIRECT_MESSAGES
    | DISCORD_INTENT_MESSAGE_CONTENT;

#[derive(Debug, Clone)]
pub struct DiscordGatewayBotResponse {
    pub websocket_url: String,
}

#[derive(Debug, Clone)]
pub struct DiscordGatewayRuntimeConfig {
    pub token: String,
    pub intents: u64,
    pub identify_properties: DiscordIdentifyProperties,
}

#[derive(Debug, Clone)]
pub struct DiscordIdentifyProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

impl Default for DiscordIdentifyProperties {
    fn default() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            browser: "uxc".to_string(),
            device: "uxc".to_string(),
        }
    }
}

pub fn derive_gateway_bot_endpoint(endpoint: &str) -> Result<String> {
    let mut url = url::Url::parse(endpoint).context("invalid Discord Gateway endpoint")?;
    match url.scheme() {
        "http" | "https" => {}
        other => bail!(
            "Discord Gateway transport requires an http:// or https:// endpoint, got '{}'",
            other
        ),
    }

    let path = url.path().trim_end_matches('/');
    let new_path = if path.is_empty() || path == "/" {
        "/api/v10/gateway/bot".to_string()
    } else if path.ends_with("/gateway/bot") {
        path.to_string()
    } else {
        format!("{path}/gateway/bot")
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub fn prepare_gateway_websocket_url(raw: &str) -> Result<String> {
    let mut url = url::Url::parse(raw).context("invalid Discord gateway websocket url")?;
    match url.scheme() {
        "ws" | "wss" => {}
        other => bail!(
            "Discord Gateway open response must contain a ws:// or wss:// URL, got '{}'",
            other
        ),
    }
    url.query_pairs_mut()
        .append_pair("v", "10")
        .append_pair("encoding", "json");
    Ok(url.to_string())
}

pub fn parse_gateway_bot_response(body: &Value) -> Result<DiscordGatewayBotResponse> {
    let websocket_url = body
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Discord gateway response missing url"))?;
    Ok(DiscordGatewayBotResponse {
        websocket_url: websocket_url.to_string(),
    })
}

pub struct DiscordGatewayHandler {
    config: DiscordGatewayRuntimeConfig,
    sequence: Option<u64>,
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    heartbeat_interval: Option<Duration>,
    next_heartbeat_at: Option<Instant>,
    heartbeat_ack_pending: bool,
    force_resume: bool,
}

impl DiscordGatewayHandler {
    pub fn new(config: DiscordGatewayRuntimeConfig) -> Self {
        Self {
            config,
            sequence: None,
            session_id: None,
            resume_gateway_url: None,
            heartbeat_interval: None,
            next_heartbeat_at: None,
            heartbeat_ack_pending: false,
            force_resume: false,
        }
    }

    pub fn preferred_gateway_websocket_url(&self) -> Option<String> {
        self.resume_gateway_url.clone()
    }

    fn clear_session(&mut self) {
        self.session_id = None;
        self.resume_gateway_url = None;
        self.sequence = None;
        self.force_resume = false;
    }

    fn can_resume(&self) -> bool {
        self.force_resume
            && self.sequence.is_some()
            && self.session_id.is_some()
            && self.resume_gateway_url.is_some()
    }

    fn next_heartbeat_output(&mut self) -> Result<WebSocketHandlerOutput> {
        if self.heartbeat_ack_pending {
            return Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Reconnect,
                data: None,
                meta: Some(json!({
                    "op": DISCORD_GATEWAY_HEARTBEAT,
                    "message_type": "heartbeat_timeout",
                })),
                outbound_text_frames: Vec::new(),
                stop_reason: Some("heartbeat_timeout".to_string()),
            });
        }
        let Some(interval) = self.heartbeat_interval else {
            return Ok(WebSocketHandlerOutput::continue_empty());
        };
        self.next_heartbeat_at = Some(Instant::now() + interval);
        self.heartbeat_ack_pending = true;
        Ok(WebSocketHandlerOutput {
            action: WebSocketHandlerAction::Continue,
            data: None,
            meta: None,
            outbound_text_frames: vec![json!({
                "op": DISCORD_GATEWAY_HEARTBEAT,
                "d": self.sequence
            })
            .to_string()],
            stop_reason: None,
        })
    }

    fn identify_frame(&self) -> String {
        json!({
            "op": DISCORD_GATEWAY_IDENTIFY,
            "d": {
                "token": self.config.token,
                "intents": self.config.intents,
                "properties": {
                    "os": self.config.identify_properties.os,
                    "browser": self.config.identify_properties.browser,
                    "device": self.config.identify_properties.device,
                }
            }
        })
        .to_string()
    }

    fn resume_frame(&self) -> Option<String> {
        Some(
            json!({
                "op": DISCORD_GATEWAY_RESUME,
                "d": {
                    "token": self.config.token,
                    "session_id": self.session_id.as_ref()?,
                    "seq": self.sequence?,
                }
            })
            .to_string(),
        )
    }
}

#[async_trait]
impl WebSocketSessionHandler for DiscordGatewayHandler {
    async fn on_open(&mut self, _meta: &WebSocketOpenMeta) -> Result<WebSocketHandlerAction> {
        self.heartbeat_interval = None;
        self.next_heartbeat_at = None;
        self.heartbeat_ack_pending = false;
        Ok(WebSocketHandlerAction::Continue)
    }

    async fn on_text_frame(&mut self, text: String) -> Result<WebSocketHandlerOutput> {
        let value: Value = serde_json::from_str(&text)
            .map_err(|err| anyhow!("invalid Discord gateway message: {}", err))?;
        let opcode = value
            .get("op")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("Discord gateway message missing op"))?;
        if let Some(sequence) = value.get("s").and_then(Value::as_u64) {
            self.sequence = Some(sequence);
        }

        let mut meta = json!({
            "frame_type": "text_json",
            "op": opcode,
        });
        if let Some(event_type) = value.get("t").and_then(Value::as_str) {
            meta["event_type"] = json!(event_type);
        }

        match opcode {
            DISCORD_GATEWAY_HELLO => {
                let interval_ms = value
                    .get("d")
                    .and_then(|v| v.get("heartbeat_interval"))
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow!("Discord HELLO payload missing heartbeat_interval"))?;
                let interval = Duration::from_millis(interval_ms);
                self.heartbeat_interval = Some(interval);
                self.next_heartbeat_at = Some(Instant::now() + interval);
                self.heartbeat_ack_pending = false;
                meta["message_type"] = json!("hello");
                let outbound = if self.can_resume() {
                    self.resume_frame().into_iter().collect()
                } else {
                    self.force_resume = false;
                    vec![self.identify_frame()]
                };
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: Some(value),
                    meta: Some(meta),
                    outbound_text_frames: outbound,
                    stop_reason: None,
                })
            }
            DISCORD_GATEWAY_DISPATCH => {
                if value.get("t").and_then(Value::as_str) == Some("READY") {
                    self.session_id = value
                        .get("d")
                        .and_then(|v| v.get("session_id"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    self.resume_gateway_url = value
                        .get("d")
                        .and_then(|v| v.get("resume_gateway_url"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .map(|url| prepare_gateway_websocket_url(&url))
                        .transpose()?;
                    self.force_resume = true;
                } else if value.get("t").and_then(Value::as_str) == Some("RESUMED") {
                    self.force_resume = true;
                }
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: Some(value),
                    meta: Some(meta),
                    outbound_text_frames: Vec::new(),
                    stop_reason: None,
                })
            }
            DISCORD_GATEWAY_HEARTBEAT => {
                meta["message_type"] = json!("heartbeat_request");
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: None,
                    meta: Some(meta),
                    outbound_text_frames: vec![json!({
                        "op": DISCORD_GATEWAY_HEARTBEAT,
                        "d": self.sequence
                    })
                    .to_string()],
                    stop_reason: None,
                })
            }
            DISCORD_GATEWAY_HEARTBEAT_ACK => {
                self.heartbeat_ack_pending = false;
                meta["message_type"] = json!("heartbeat_ack");
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Continue,
                    data: None,
                    meta: Some(meta),
                    outbound_text_frames: Vec::new(),
                    stop_reason: None,
                })
            }
            DISCORD_GATEWAY_RECONNECT => {
                meta["message_type"] = json!("reconnect");
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Reconnect,
                    data: Some(value),
                    meta: Some(meta),
                    outbound_text_frames: Vec::new(),
                    stop_reason: Some("gateway_reconnect".to_string()),
                })
            }
            DISCORD_GATEWAY_INVALID_SESSION => {
                let resumable = value.get("d").and_then(Value::as_bool).unwrap_or(false);
                if resumable && self.session_id.is_some() && self.sequence.is_some() {
                    self.force_resume = true;
                } else {
                    self.clear_session();
                }
                meta["message_type"] = json!("invalid_session");
                meta["resumable"] = json!(resumable);
                Ok(WebSocketHandlerOutput {
                    action: WebSocketHandlerAction::Reconnect,
                    data: Some(value),
                    meta: Some(meta),
                    outbound_text_frames: Vec::new(),
                    stop_reason: Some("invalid_session".to_string()),
                })
            }
            _ => Ok(WebSocketHandlerOutput {
                action: WebSocketHandlerAction::Continue,
                data: Some(value),
                meta: Some(meta),
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
                "frame_type": "binary",
                "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            })),
            outbound_text_frames: Vec::new(),
            stop_reason: None,
        })
    }

    async fn on_close(&mut self, meta: WebSocketCloseMeta) -> Result<WebSocketHandlerAction> {
        match meta.code {
            Some(4004) => bail!("Discord gateway authentication failed (close code 4004)"),
            Some(4010) => bail!("Discord gateway invalid shard (close code 4010)"),
            Some(4011) => bail!("Discord gateway sharding required (close code 4011)"),
            Some(4013) => bail!("Discord gateway invalid intents (close code 4013)"),
            Some(4014) => bail!(
                "Discord gateway disallowed intents (close code 4014); enable required bot intents in the Discord developer portal"
            ),
            _ => Ok(WebSocketHandlerAction::Reconnect),
        }
    }

    fn next_wakeup_at(&self) -> Option<Instant> {
        self.next_heartbeat_at
    }

    async fn on_wakeup(&mut self) -> Result<WebSocketHandlerOutput> {
        self.next_heartbeat_output()
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

    fn test_config() -> DiscordGatewayRuntimeConfig {
        DiscordGatewayRuntimeConfig {
            token: "bot-token".to_string(),
            intents: DISCORD_DEFAULT_MESSAGE_INTENTS,
            identify_properties: DiscordIdentifyProperties::default(),
        }
    }

    #[test]
    fn derives_gateway_endpoint_from_api_base() {
        assert_eq!(
            derive_gateway_bot_endpoint("https://discord.com/api/v10").unwrap(),
            "https://discord.com/api/v10/gateway/bot"
        );
    }

    #[test]
    fn prepares_gateway_websocket_url_with_protocol_params() {
        assert_eq!(
            prepare_gateway_websocket_url("wss://gateway.discord.gg").unwrap(),
            "wss://gateway.discord.gg/?v=10&encoding=json"
        );
    }

    #[test]
    fn parses_gateway_bot_response() {
        let parsed = parse_gateway_bot_response(&json!({
            "url": "wss://gateway.discord.gg",
            "session_start_limit": {
                "remaining": 42,
                "reset_after": 12345
            }
        }))
        .unwrap();
        assert_eq!(parsed.websocket_url, "wss://gateway.discord.gg");
    }

    #[tokio::test]
    async fn hello_emits_identify_and_schedules_heartbeat() {
        let mut handler = DiscordGatewayHandler::new(test_config());
        let output = handler
            .on_text_frame(
                json!({
                    "op": DISCORD_GATEWAY_HELLO,
                    "d": { "heartbeat_interval": 45000 }
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert_eq!(output.data.as_ref().unwrap()["op"], DISCORD_GATEWAY_HELLO);
        assert_eq!(output.outbound_text_frames.len(), 1);
        let identify: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(identify["op"], DISCORD_GATEWAY_IDENTIFY);
        assert_eq!(identify["d"]["token"], "bot-token");
        assert!(handler.next_wakeup_at().is_some());
    }

    #[tokio::test]
    async fn ready_captures_resume_state() {
        let mut handler = DiscordGatewayHandler::new(test_config());
        let _ = handler
            .on_text_frame(
                json!({
                    "op": DISCORD_GATEWAY_HELLO,
                    "d": { "heartbeat_interval": 1000 }
                })
                .to_string(),
            )
            .await
            .unwrap();
        let output = handler
            .on_text_frame(
                json!({
                    "op": DISCORD_GATEWAY_DISPATCH,
                    "t": "READY",
                    "s": 7,
                    "d": {
                        "session_id": "session-1",
                        "resume_gateway_url": "wss://gateway.discord.gg"
                    }
                })
                .to_string(),
            )
            .await
            .unwrap();

        assert_eq!(output.meta.as_ref().unwrap()["event_type"], "READY");
        assert_eq!(
            handler.preferred_gateway_websocket_url().as_deref(),
            Some("wss://gateway.discord.gg/?v=10&encoding=json")
        );
        assert!(handler.can_resume());
    }

    #[tokio::test]
    async fn heartbeat_wakeup_emits_heartbeat_frame() {
        let mut handler = DiscordGatewayHandler::new(test_config());
        let _ = handler
            .on_text_frame(
                json!({
                    "op": DISCORD_GATEWAY_HELLO,
                    "d": { "heartbeat_interval": 1000 }
                })
                .to_string(),
            )
            .await
            .unwrap();
        let output = handler.on_wakeup().await.unwrap();

        assert_eq!(output.outbound_text_frames.len(), 1);
        let heartbeat: Value = serde_json::from_str(&output.outbound_text_frames[0]).unwrap();
        assert_eq!(heartbeat["op"], DISCORD_GATEWAY_HEARTBEAT);
    }

    #[tokio::test]
    async fn close_fails_for_disallowed_intents() {
        let mut handler = DiscordGatewayHandler::new(test_config());
        let err = handler
            .on_close(WebSocketCloseMeta {
                code: Some(4014),
                reason: Some("disallowed intents".to_string()),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disallowed intents"));
    }
}
