use crate::adapters::mcp::types::{JsonRpcNotification, ResourceContents};
use crate::adapters::{
    self, Adapter, AdapterEnum, DetectionOptions, Operation, ProtocolDetector, ProtocolType,
};
use crate::arg_coercion::prepare_execute_args;
use crate::auth::injected_env::{fingerprint_injected_env, render_injected_env, InjectEnvSpec};
use crate::auth::{self, Profile};
use crate::cache::{self, Cache, CacheConfig};
use crate::codegen::build_codegen_host_schema;
use crate::daemon_log::{redact_endpoint, redact_sensitive};
use crate::daemon_log::{DaemonEventType, DaemonLogEntry, DaemonLogger};
use crate::error::{
    structured_error_from_anyhow, structured_error_from_jsonrpc_error, StructuredError,
    StructuredErrorPayload, UxcError,
};
use crate::managed_source_streams::{
    ManagedSourceRecord, ManagedSourceStore, StreamEventRecord, StreamInfoRecord,
};
use crate::subscription_discord::{
    derive_gateway_bot_endpoint, parse_gateway_bot_response, prepare_gateway_websocket_url,
    DiscordGatewayBotResponse, DiscordGatewayHandler, DiscordGatewayRuntimeConfig,
    DiscordIdentifyProperties, DISCORD_DEFAULT_MESSAGE_INTENTS,
};
use crate::subscription_feishu::{
    derive_feishu_ws_config_endpoint, parse_feishu_long_connection_open_response,
    resolve_feishu_long_connection_runtime_config, FeishuLongConnectionHandler,
    FeishuLongConnectionOpenResponse,
};
use crate::subscription_graphql::{
    derive_graphql_websocket_endpoint, graphql_transport_init_message, GraphQLProfileFallback,
    GraphQLSubscriptionConfig, GraphQLSubscriptionHandler, GraphQLWebSocketProfile,
};
use crate::subscription_jsonrpc::{
    resolve_jsonrpc_unsubscribe_operation, JsonRpcSubscriptionConfig, JsonRpcSubscriptionHandler,
};
use crate::subscription_poll::{PollRuntimeContext, PollRuntimeObserver, PollSubscriptionConfig};
use crate::subscription_slack::{
    derive_socket_mode_open_endpoint, parse_socket_mode_open_response, SlackSocketModeHandler,
};
use crate::subscription_websocket::{
    self, RawFrameHandler, WebSocketRunError, WebSocketRuntimeConfig, WebSocketRuntimeObserver,
};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, Mutex, RwLock};
use tokio::task::JoinHandle;

const JSONRPC_VERSION: &str = "2.0";
const START_POLL_TRIES: usize = 30;
const START_POLL_INTERVAL_MS: u64 = 100;
const STOP_POLL_TRIES: usize = 50;
const STOP_POLL_INTERVAL_MS: u64 = 100;
const START_LOCK_STALE_SECS: u64 = 30;
const STDIO_INIT_LOCK_STALE_SECS: u64 = 30;
const MCP_IDLE_TTL_SECS: u64 = 600;
const MCP_CAN_REAP_PROBE_TIMEOUT_MS: u64 = 1_000;
const MCP_CAN_REAP_RETRY_AFTER_SECS: u64 = 30;
// Five seconds is long enough for cooperative stdio servers to notice stdin EOF
// and release external resources, while still bounding daemon-side eviction stalls.
const MCP_STDIO_EXIT_TIMEOUT_SECS: u64 = 5;
const CONNECT_TIMEOUT_SECS: u64 = 2;
const FRAME_IO_TIMEOUT_SECS: u64 = 120;
const MAX_FRAME_BODY_BYTES: usize = 8 * 1024 * 1024;
const SUBSCRIPTION_HTTP_TIMEOUT_SECS: u64 = 300;
const SUBSCRIPTION_STOP_TIMEOUT_SECS: u64 = 5;
const SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS: u64 = 1;
const SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS: u64 = 30;
const SUBSCRIPTION_MAX_BUFFER_BYTES: usize = 1024 * 1024;
const SUBSCRIPTION_EVENT_HISTORY_LIMIT: usize = 1024;
const SUBSCRIPTION_TERMINAL_TTL_SECS: u64 = 60;
const SUBSCRIPTION_EVENTS_DEFAULT_LIMIT: usize = 100;
const SUBSCRIPTION_EVENTS_MAX_LIMIT: usize = 500;
const SUBSCRIPTION_EVENTS_MAX_WAIT_MS: u64 = 15_000;
const MANAGED_STREAM_EVENTS_DEFAULT_LIMIT: usize = 100;
const MANAGED_STREAM_EVENTS_MAX_LIMIT: usize = 500;
const MCP_NOTIFICATION_HISTORY_LIMIT: usize = 256;
const ARTIFACT_COMPACTION_THRESHOLD_BYTES: usize = 64 * 1024;
const ARTIFACT_PREVIEW_MAX_OBJECT_KEYS: usize = 20;
const ARTIFACT_PREVIEW_MAX_ARRAY_ITEMS: usize = 20;
const ARTIFACT_PREVIEW_MAX_STRING_CHARS: usize = 512;
const ARTIFACT_PREVIEW_MAX_DEPTH: usize = 3;
const SUB_STATUS_STOPPED_AFTER_RESTART: &str = "stopped_after_restart";
const SUB_STATUS_RESUME_FAILED: &str = "resume_failed";
const ERR_PROTOCOL_DETECTION: i32 = -32010;
const ERR_OPERATION_NOT_FOUND: i32 = -32011;
const ERR_OAUTH_REQUIRED: i32 = -32012;
const ERR_OAUTH_REFRESH_FAILED: i32 = -32013;
const ERR_OAUTH_SCOPE_INSUFFICIENT: i32 = -32014;
const ERR_SUBSCRIPTION_CURSOR_EXPIRED: i32 = -32015;
const ERR_RUNTIME_GENERIC: i32 = -32030;

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> std::ffi::c_int;
}

pub fn daemon_supported() -> bool {
    cfg!(unix)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAction {
    HostHelp,
    CodegenSchema,
    OperationHelp,
    Execute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInvokeRequest {
    pub request_id: String,
    pub endpoint: String,
    pub action: RuntimeAction,
    pub operation_id: Option<String>,
    pub args: Option<HashMap<String, Value>>,
    pub options: RuntimeInvokeOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInvokeOptions {
    pub auth: Option<String>,
    #[serde(default)]
    pub inject_env: Vec<InjectEnvSpec>,
    pub no_cache: bool,
    pub cache_ttl: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    pub refresh_schema: bool,
    pub schema_url: Option<String>,
    pub link_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_skill_doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_skill_path: Option<String>,
    pub schema_mapping_file: Option<String>,
    #[serde(default)]
    pub daemon_exclusive: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_idle_ttl: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInvokeResponse {
    pub protocol: String,
    pub endpoint: String,
    pub kind: String,
    pub operation: Option<String>,
    pub data: Value,
    pub duration_ms: Option<u64>,
    pub meta: RuntimeMeta,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeMeta {
    pub schema_involved: Option<bool>,
    pub cache_source: Option<String>,
    pub cache_age_ms: Option<u64>,
    pub cache_stale: Option<bool>,
    pub cache_fallback: Option<bool>,
    pub daemon_session_reused: Option<bool>,
    pub response_status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<HashMap<String, String>>,
    pub artifact_truncated: Option<bool>,
    pub artifact_kind: Option<String>,
    pub artifact_bytes: Option<u64>,
    pub artifact_path: Option<String>,
    pub artifact_ref: Option<String>,
    pub artifact_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeStartRequest {
    pub request_id: String,
    pub endpoint: String,
    pub sink: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<HashMap<String, Value>>,
    pub resource_uri: Option<String>,
    #[serde(default)]
    pub read_resource: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_hint: Option<SubscriptionTransportHint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subprotocols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_text_frames: Vec<String>,
    pub mode: SubscriptionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_config: Option<Value>,
    /// If true, do not auto-resume this subscription after daemon restart.
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub internal: bool,
    pub options: RuntimeInvokeOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeStartResponse {
    pub job_id: String,
    pub mode: SubscriptionMode,
    pub protocol: String,
    pub endpoint: String,
    pub sink: String,
    pub resource_uri: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeStopResponse {
    pub job_id: String,
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEventsRequest {
    pub job_id: String,
    #[serde(default)]
    pub after_seq: u64,
    #[serde(default = "default_subscription_events_limit")]
    pub limit: usize,
    #[serde(default)]
    pub wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEventsResponse {
    pub job_id: String,
    pub status: String,
    pub events: Vec<SubscriptionEventEnvelope>,
    pub next_after_seq: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionTransportHint {
    Websocket,
    DiscordGateway,
    SlackSocketMode,
    FeishuLongConnection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionMode {
    Stream,
    Poll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionJobView {
    pub job_id: String,
    pub mode: SubscriptionMode,
    pub endpoint: String,
    pub protocol: String,
    pub sink: String,
    pub resource_uri: Option<String>,
    pub status: String,
    #[serde(default)]
    pub durable: bool,
    #[serde(default)]
    pub auto_resume: bool,
    #[serde(default)]
    pub resume_strategy: String,
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub stopped_at_unix: Option<u64>,
    pub last_event_at_unix: Option<u64>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub restart_count: u64,
    #[serde(default)]
    pub last_resume_at_unix: Option<u64>,
    #[serde(default)]
    pub last_resume_error: Option<String>,
    pub reconnect_count: u64,
    pub written_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEventEnvelope {
    pub version: String,
    pub job_id: String,
    pub seq: u64,
    pub timestamp_unix: u64,
    pub protocol: String,
    pub source_kind: String,
    pub event_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceSpec {
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<HashMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    #[serde(default)]
    pub read_resource: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_hint: Option<SubscriptionTransportHint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subprotocols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_text_frames: Vec<String>,
    pub mode: SubscriptionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_config: Option<Value>,
    pub options: RuntimeInvokeOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceEnsureRequest {
    pub namespace: String,
    pub source_key: String,
    pub spec: ManagedSourceSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceEnsureResponse {
    pub namespace: String,
    pub source_key: String,
    pub run_id: String,
    pub stream_id: String,
    pub status: String,
    pub reused: bool,
    pub replaced_previous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceStatusRequest {
    pub namespace: String,
    pub source_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceView {
    pub namespace: String,
    pub source_key: String,
    pub run_id: String,
    pub stream_id: String,
    pub spec_key: String,
    pub status: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub stopped_at_unix: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceStopResponse {
    pub namespace: String,
    pub source_key: String,
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceDeleteResponse {
    pub namespace: String,
    pub source_key: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedStreamEvent {
    pub stream_id: String,
    pub offset: u64,
    pub ingested_at_unix: u64,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedStreamReadRequest {
    pub stream_id: String,
    #[serde(default)]
    pub after_offset: u64,
    #[serde(default = "default_managed_stream_events_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedStreamReadResponse {
    pub stream_id: String,
    pub events: Vec<ManagedStreamEvent>,
    pub next_after_offset: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedStreamInfo {
    pub stream_id: String,
    pub namespace: String,
    pub source_key: String,
    pub created_at_unix: u64,
    pub earliest_offset: Option<u64>,
    pub latest_offset: Option<u64>,
    pub event_count: u64,
    pub retention_max_rows: u64,
    pub retention_max_age_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedStreamTrimRequest {
    pub stream_id: String,
    pub before_offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedStreamTrimResponse {
    pub stream_id: String,
    pub trimmed: u64,
}

fn default_subscription_events_limit() -> usize {
    SUBSCRIPTION_EVENTS_DEFAULT_LIMIT
}

fn default_managed_stream_events_limit() -> usize {
    MANAGED_STREAM_EVENTS_DEFAULT_LIMIT
}

fn should_read_mcp_resource_snapshot(notification: &JsonRpcNotification) -> bool {
    notification.method == "notifications/resources/updated"
}

async fn append_mcp_resource_snapshot(
    sink: &mut tokio::fs::File,
    view: &Arc<Mutex<SubscriptionJobView>>,
    seq: &mut u64,
    reason: &str,
    resource_contents: ResourceContents,
) -> Result<()> {
    append_subscription_event(
        sink,
        view,
        seq,
        "mcp_resource",
        "snapshot",
        Some(serde_json::to_value(resource_contents)?),
        Some(json!({ "reason": reason })),
    )
    .await
}

async fn append_mcp_resource_read_result(
    sink: &mut tokio::fs::File,
    view: &Arc<Mutex<SubscriptionJobView>>,
    seq: &mut u64,
    resource_uri: &str,
    reason: &str,
    error_context: &str,
    read_result: Result<ResourceContents>,
) -> Result<()> {
    match read_result {
        Ok(contents) => append_mcp_resource_snapshot(sink, view, seq, reason, contents).await,
        Err(err) => {
            let msg = format!("{}: {}", error_context, err);
            append_subscription_event(
                sink,
                view,
                seq,
                "mcp_resource",
                "error",
                None,
                Some(json!({ "message": msg, "resource_uri": resource_uri })),
            )
            .await?;
            update_subscription_view(view, None, Some(msg), false).await;
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub socket: String,
    pub version: Option<String>,
    pub started_at_unix: Option<u64>,
    pub request_count: u64,
    pub mcp_stdio_sessions: usize,
    pub mcp_http_sessions: usize,
    pub mcp_reuse_hits: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSessionView {
    /// Opaque session identifier safe for display in CLI and JSON output.
    pub session_key: String,
    /// Transport family for the daemon-managed session (for example `stdio`).
    pub transport: String,
    /// Protocol identifier for the daemon-managed session (for example `mcp_stdio`).
    pub protocol: String,
    /// Redacted endpoint associated with the live session.
    pub endpoint: String,
    /// Link name from the latest request metadata, if present.
    pub link_name: Option<String>,
    /// Source skill name from the latest request metadata, if present.
    pub link_skill: Option<String>,
    /// Source skill docs URL from the latest request metadata, if present.
    pub link_skill_doc: Option<String>,
    /// Source skill local path from the latest request metadata, if present.
    pub link_skill_path: Option<String>,
    /// Safe summary of the underlying stdio command.
    pub command_summary: String,
    /// Child process id for stdio-backed sessions when available.
    pub child_pid: Option<u32>,
    /// Unix timestamp when the daemon created this live session.
    pub started_at_unix: u64,
    /// Unix timestamp when the daemon last served a request through this session.
    pub last_used_at_unix: u64,
    /// Effective idle TTL in seconds. `0` disables idle reaping for the session.
    pub idle_ttl_secs: u64,
    /// Seconds since this session last served a request.
    pub idle_for_secs: u64,
    /// Seconds until idle reaping. `None` means this session does not expire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_secs: Option<u64>,
    #[serde(default)]
    pub daemon_exclusive: Vec<String>,
    /// Current liveness state for the live daemon cache entry.
    pub state: String,
    /// Number of daemon-tracked in-flight requests currently using this session.
    pub in_flight_requests: u64,
    pub reuse_eligible: bool,
    pub can_reap_contract: CanReapContractView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_summary: Option<String>,
    #[serde(default)]
    pub recent_stderr: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanReapContractSupport {
    #[default]
    Unknown,
    Supported,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanReapContractView {
    pub support: CanReapContractSupport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_reap: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<adapters::mcp::CanReapState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_summary: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct EnsureDaemonOutcome {
    pub started_now: bool,
    pub restarted_for_version_mismatch: bool,
    pub previous_version: Option<String>,
}

#[derive(Debug, Clone)]
struct SchemaCacheMeta {
    age_ms: u64,
    stale: bool,
    fallback: bool,
}

struct ResolveAdapterResult {
    adapter: AdapterEnum,
    cache_meta: Option<SchemaCacheMeta>,
}

#[derive(Default)]
struct ServerState {
    started_at_unix: u64,
    request_count: u64,
}

#[derive(Clone)]
struct McpSessionManager {
    stdio: Arc<Mutex<HashMap<String, Arc<Mutex<McpStdioSession>>>>>,
    stdio_snapshots: Arc<Mutex<HashMap<String, McpStdioSessionSnapshot>>>,
    stdio_init_locks: Arc<Mutex<HashMap<String, InitLockEntry>>>,
    stdio_exclusive_locks: Arc<Mutex<HashMap<String, InitLockEntry>>>,
    stdio_exclusive_owners: Arc<Mutex<HashMap<String, String>>>, // exclusive_key -> session_key
    stdio_session_exclusives: Arc<Mutex<HashMap<String, Vec<String>>>>, // session_key -> [exclusive_key]
    http: Arc<Mutex<HashMap<String, Arc<McpHttpSession>>>>,
    http_lookup: Arc<Mutex<HashMap<String, String>>>, // raw endpoint/auth key -> resolved session key
    reuse_hits: Arc<Mutex<u64>>,
    logger: Option<DaemonLogger>,
}

struct InitLockEntry {
    lock: Arc<Mutex<()>>,
    touched_at: Instant,
}

struct McpStdioSession {
    client: adapters::mcp::McpStdioClient,
    tools: Option<Vec<adapters::mcp::types::Tool>>,
    tools_dirty: bool,
    notifications: SessionNotificationFanout,
    resource_subscriptions: HashMap<String, usize>,
    last_used: Instant,
    last_used_at_unix: u64,
    idle_ttl_secs: u64,
    child_pid: Option<u32>,
    link_name: Option<String>,
    link_skill: Option<String>,
    link_skill_doc: Option<String>,
    link_skill_path: Option<String>,
    endpoint: String,
    daemon_exclusive: Vec<String>,
    can_reap_contract: CanReapContractView,
    next_can_reap_probe_after: Option<Instant>,
}

#[derive(Debug, Clone)]
struct McpStdioSessionSnapshot {
    endpoint: String,
    link_name: Option<String>,
    link_skill: Option<String>,
    link_skill_doc: Option<String>,
    link_skill_path: Option<String>,
    command_summary: String,
    child_pid: Option<u32>,
    started_at_unix: u64,
    last_used_at_unix: u64,
    idle_ttl_secs: u64,
    daemon_exclusive: Vec<String>,
    in_flight_requests: u64,
    reuse_eligible: bool,
    can_reap_contract: CanReapContractView,
    last_error_summary: Option<String>,
    recent_stderr: Vec<String>,
}

struct McpHttpSession {
    transport: adapters::mcp::McpRemoteTransport,
    notifications: Mutex<SessionNotificationFanout>,
    resource_subscriptions: Mutex<HashMap<String, usize>>,
    resource_subscription_ops: Mutex<()>,
    lookup_key: String,
    last_used: Mutex<Instant>,
}

#[derive(Debug, Clone)]
struct FanoutNotification {
    seq: u64,
    notification: JsonRpcNotification,
}

#[derive(Debug, Clone, Default)]
struct SessionNotificationFanout {
    next_seq: u64,
    notifications: VecDeque<FanoutNotification>,
    stream_error: Option<String>,
}

struct StdioSessionRequestMetadata<'a> {
    idle_ttl_secs: Option<u64>,
    link_name: Option<&'a str>,
    link_skill: Option<&'a str>,
    link_skill_doc: Option<&'a str>,
    link_skill_path: Option<&'a str>,
    endpoint: &'a str,
    exclusive_keys: &'a [String],
}

fn resolve_stdio_request_metadata(
    metadata: &StdioSessionRequestMetadata<'_>,
    existing_exclusive_keys: &[String],
) -> (
    u64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Vec<String>,
) {
    let daemon_exclusive = if metadata.exclusive_keys.is_empty() {
        existing_exclusive_keys.to_vec()
    } else {
        metadata.exclusive_keys.to_vec()
    };
    (
        metadata.idle_ttl_secs.unwrap_or(MCP_IDLE_TTL_SECS),
        metadata.link_name.map(str::to_string),
        metadata.link_skill.map(str::to_string),
        metadata.link_skill_doc.map(str::to_string),
        metadata.link_skill_path.map(str::to_string),
        metadata.endpoint.to_string(),
        daemon_exclusive,
    )
}

#[derive(Clone, Default)]
struct SubscriptionManager {
    jobs: Arc<Mutex<HashMap<String, Arc<SubscriptionJobEntry>>>>,
    terminal_jobs: Arc<Mutex<HashMap<String, TerminalSubscriptionEntry>>>,
    next_id: Arc<Mutex<u64>>,
    store_path: PathBuf,
}

#[derive(Clone)]
struct ManagedSourceManager {
    entries: Arc<Mutex<HashMap<String, Arc<ManagedSourceEntry>>>>,
    store: ManagedSourceStore,
}

struct ManagedSourceEntry {
    namespace: String,
    source_key: String,
    stream_id: String,
    state: Arc<Mutex<ManagedSourceView>>,
    underlying_job_id: Arc<Mutex<Option<String>>>,
    mirrored_after_seq: Arc<Mutex<u64>>,
    stop_tx: Mutex<watch::Sender<bool>>,
    tail_task: Mutex<Option<JoinHandle<()>>>,
}

struct SubscriptionJobEntry {
    request: SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    /// Wrapped so resume can replace the sender for a reconstructed job.
    stop_tx: Mutex<watch::Sender<bool>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
struct TerminalSubscriptionEntry {
    view: SubscriptionJobView,
    sink_path: PathBuf,
    expires_at_unix: u64,
    memory_backed: bool,
}

enum PreparedSubscriptionSink {
    File(PathBuf),
    Memory,
}

struct PreparedSubscription {
    sink: PreparedSubscriptionSink,
    sink_spec: String,
    protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSubscriptionRecord {
    request: SubscribeStartRequest,
    view: SubscriptionJobView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSubscriptionStore {
    version: String,
    jobs: Vec<PersistedSubscriptionRecord>,
}

fn record_should_persist(record: &PersistedSubscriptionRecord) -> bool {
    record.view.durable
        && record.view.auto_resume
        && !sink_is_memory(&record.request.sink)
        && !sink_is_memory(&record.view.sink)
}

fn persisted_record_sink_path(record: &PersistedSubscriptionRecord) -> Result<PathBuf> {
    parse_file_sink(&record.request.sink).or_else(|_| parse_file_sink(&record.view.sink))
}

impl McpStdioSession {
    fn apply_request_metadata(&mut self, metadata: &StdioSessionRequestMetadata<'_>) {
        let (
            idle_ttl_secs,
            link_name,
            link_skill,
            link_skill_doc,
            link_skill_path,
            endpoint,
            daemon_exclusive,
        ) = resolve_stdio_request_metadata(metadata, &self.daemon_exclusive);
        self.idle_ttl_secs = idle_ttl_secs;
        self.link_name = link_name;
        self.link_skill = link_skill;
        self.link_skill_doc = link_skill_doc;
        self.link_skill_path = link_skill_path;
        self.endpoint = endpoint;
        self.daemon_exclusive = daemon_exclusive;
    }

    async fn refresh_tools_if_needed(
        &mut self,
        _endpoint: &str,
        _cache: &Arc<dyn Cache>,
        timeout: Duration,
    ) -> Result<Vec<adapters::mcp::types::Tool>> {
        if self.tools.is_none() || self.tools_dirty {
            let tools = self.client.list_tools_with_timeout(timeout).await?;
            self.tools = Some(tools);
            self.tools_dirty = false;
        }

        Ok(self.tools.clone().unwrap_or_default())
    }

    async fn mark_tools_dirty_from_notifications(
        &mut self,
        endpoint: &str,
        cache: &Arc<dyn Cache>,
    ) -> bool {
        let was_dirty = self.tools_dirty;
        let _ = self.sync_notifications(endpoint, Some(cache)).await;
        !was_dirty && self.tools_dirty
    }

    fn record_notifications(
        &mut self,
        notifications: Vec<JsonRpcNotification>,
        endpoint: &str,
        cache: Option<&Arc<dyn Cache>>,
    ) -> Vec<JsonRpcNotification> {
        for notification in &notifications {
            if notification.method == "notifications/tools/list_changed" {
                self.tools_dirty = true;
                if let Some(cache) = cache {
                    let _ = cache.invalidate(endpoint);
                }
            }
        }
        self.notifications.extend(notifications.clone());
        notifications
    }

    async fn sync_notifications(
        &mut self,
        endpoint: &str,
        cache: Option<&Arc<dyn Cache>>,
    ) -> Vec<JsonRpcNotification> {
        let notifications = self.client.drain_notifications().await;
        self.record_notifications(notifications, endpoint, cache)
    }

    async fn notifications_since(
        &mut self,
        cursor: u64,
        endpoint: &str,
        cache: Option<&Arc<dyn Cache>>,
    ) -> (Vec<JsonRpcNotification>, u64) {
        let _ = self.sync_notifications(endpoint, cache).await;
        self.notifications.since(cursor)
    }

    async fn ensure_resource_subscription(
        &mut self,
        uri: &str,
        endpoint: &str,
        cache: Option<&Arc<dyn Cache>>,
    ) -> Result<()> {
        if let Some(count) = self.resource_subscriptions.get_mut(uri) {
            *count = count.saturating_add(1);
            return Ok(());
        }
        self.client.subscribe_resource(uri).await?;
        self.resource_subscriptions.insert(uri.to_string(), 1);
        let _ = self.sync_notifications(endpoint, cache).await;
        Ok(())
    }

    async fn release_resource_subscription(
        &mut self,
        uri: &str,
        endpoint: &str,
        cache: Option<&Arc<dyn Cache>>,
    ) -> Result<()> {
        let Some(count) = self.resource_subscriptions.get_mut(uri) else {
            return Ok(());
        };
        if *count > 1 {
            *count -= 1;
            return Ok(());
        }
        self.resource_subscriptions.remove(uri);
        self.client.unsubscribe_resource(uri).await?;
        let _ = self.sync_notifications(endpoint, cache).await;
        Ok(())
    }

    async fn read_resource(
        &mut self,
        uri: &str,
        endpoint: &str,
        cache: Option<&Arc<dyn Cache>>,
    ) -> Result<ResourceContents> {
        let result = self.client.read_resource(uri).await;
        let _ = self.sync_notifications(endpoint, cache).await;
        result
    }
}

impl SessionNotificationFanout {
    fn extend(&mut self, notifications: Vec<JsonRpcNotification>) {
        for notification in notifications {
            self.next_seq = self.next_seq.saturating_add(1);
            self.notifications.push_back(FanoutNotification {
                seq: self.next_seq,
                notification,
            });
            while self.notifications.len() > MCP_NOTIFICATION_HISTORY_LIMIT {
                self.notifications.pop_front();
            }
        }
    }

    fn since(&self, cursor: u64) -> (Vec<JsonRpcNotification>, u64) {
        let notifications = self
            .notifications
            .iter()
            .filter(|entry| entry.seq > cursor)
            .map(|entry| entry.notification.clone())
            .collect();
        (notifications, self.next_seq)
    }

    fn set_stream_error(&mut self, error: String) {
        if self.stream_error.is_none() {
            self.stream_error = Some(error);
        }
    }

    fn clear_stream_error(&mut self) {
        self.stream_error = None;
    }

    fn stream_error(&self) -> Option<String> {
        self.stream_error.clone()
    }
}

impl McpHttpSession {
    async fn drain_pending_notifications(&self) -> Vec<JsonRpcNotification> {
        if let Err(err) = self.transport.ensure_notification_stream().await {
            self.notifications
                .lock()
                .await
                .set_stream_error(err.to_string());
            return Vec::new();
        }
        self.collect_pending_notifications().await
    }

    async fn collect_pending_notifications(&self) -> Vec<JsonRpcNotification> {
        if let Some(error) = self.transport.take_stream_error().await {
            self.notifications.lock().await.set_stream_error(error);
        }
        let notifications = self.transport.drain_notifications().await;
        self.notifications
            .lock()
            .await
            .extend(notifications.clone());
        notifications
    }

    async fn notifications_since(
        &self,
        cursor: u64,
    ) -> (Vec<JsonRpcNotification>, u64, Option<String>) {
        let _ = self.drain_pending_notifications().await;
        let notifications = self.notifications.lock().await;
        let (items, next_cursor) = notifications.since(cursor);
        (items, next_cursor, notifications.stream_error())
    }

    async fn ensure_resource_subscription(&self, uri: &str) -> Result<()> {
        let _op_guard = self.resource_subscription_ops.lock().await;
        let mut subscriptions = self.resource_subscriptions.lock().await;
        if let Some(count) = subscriptions.get_mut(uri) {
            *count = count.saturating_add(1);
            return Ok(());
        }
        drop(subscriptions);
        self.notifications.lock().await.clear_stream_error();
        self.transport.subscribe_resource(uri).await?;
        self.resource_subscriptions
            .lock()
            .await
            .insert(uri.to_string(), 1);
        let _ = self.collect_pending_notifications().await;
        Ok(())
    }

    async fn release_resource_subscription(&self, uri: &str) -> Result<()> {
        let _op_guard = self.resource_subscription_ops.lock().await;
        let mut subscriptions = self.resource_subscriptions.lock().await;
        let Some(count) = subscriptions.get_mut(uri) else {
            return Ok(());
        };
        if *count > 1 {
            *count -= 1;
            return Ok(());
        }
        subscriptions.remove(uri);
        let no_more_subscriptions = subscriptions.is_empty();
        drop(subscriptions);
        let unsubscribe_result = self.transport.unsubscribe_resource(uri).await;
        if no_more_subscriptions {
            self.transport.shutdown_notification_stream().await;
            self.notifications.lock().await.clear_stream_error();
        }
        let _ = self.collect_pending_notifications().await;
        unsubscribe_result
    }

    async fn read_resource(&self, uri: &str) -> Result<ResourceContents> {
        let result = self.transport.read_resource(uri).await;
        let _ = self.collect_pending_notifications().await;
        result
    }

    async fn mark_used(&self) {
        *self.last_used.lock().await = Instant::now();
    }
}

impl McpStdioSessionSnapshot {
    fn to_view(&self, session_key: &str, now_unix: u64) -> DaemonSessionView {
        let idle_for_secs = now_unix.saturating_sub(self.last_used_at_unix);
        let expires_in_secs = if self.idle_ttl_secs == 0 {
            None
        } else {
            Some(self.idle_ttl_secs.saturating_sub(idle_for_secs))
        };
        DaemonSessionView {
            session_key: display_session_key(session_key),
            transport: "stdio".to_string(),
            protocol: "mcp_stdio".to_string(),
            endpoint: redact_sensitive(&self.endpoint),
            link_name: self.link_name.clone(),
            link_skill: self.link_skill.clone(),
            link_skill_doc: self.link_skill_doc.clone(),
            link_skill_path: self.link_skill_path.clone(),
            command_summary: self.command_summary.clone(),
            child_pid: self.child_pid,
            started_at_unix: self.started_at_unix,
            last_used_at_unix: self.last_used_at_unix,
            idle_ttl_secs: self.idle_ttl_secs,
            idle_for_secs,
            expires_in_secs,
            daemon_exclusive: self.daemon_exclusive.clone(),
            state: if self.in_flight_requests > 0 {
                "active".to_string()
            } else {
                "ready".to_string()
            },
            in_flight_requests: self.in_flight_requests,
            reuse_eligible: self.reuse_eligible,
            can_reap_contract: self.can_reap_contract.clone(),
            last_error_summary: self.last_error_summary.clone(),
            recent_stderr: self.recent_stderr.clone(),
        }
    }
}

fn command_summary_from_endpoint(endpoint: &str) -> String {
    match adapters::mcp::McpAdapter::parse_stdio_command(endpoint) {
        Ok((command, args)) => {
            let base = Path::new(&command)
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or(command);
            if args.is_empty() {
                redact_sensitive(&base)
            } else {
                format!("{} (+{} args)", redact_sensitive(&base), args.len())
            }
        }
        Err(_) => endpoint
            .split_whitespace()
            .next()
            .map(redact_sensitive)
            .unwrap_or_else(|| "<unknown>".to_string()),
    }
}

fn truncate_for_session_summary(value: &str) -> String {
    const MAX: usize = 240;
    if value.chars().count() <= MAX {
        value.to_string()
    } else {
        let truncated = value
            .char_indices()
            .nth(MAX)
            .map(|(idx, _)| &value[..idx])
            .unwrap_or(value);
        format!("{truncated}...")
    }
}

fn redact_recent_stderr(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| truncate_for_session_summary(&redact_sensitive(&line)))
        .collect()
}

fn can_reap_error_summary(err: &anyhow::Error) -> String {
    truncate_for_session_summary(&redact_sensitive(&err.to_string()))
}

fn can_reap_method_not_found(err: &anyhow::Error) -> bool {
    structured_error_from_anyhow(err)
        .and_then(|payload| payload.details)
        .and_then(|details| details.get("jsonrpc_code").and_then(Value::as_i64))
        == Some(-32601)
}

impl McpSessionManager {
    fn new(logger: Option<DaemonLogger>) -> Self {
        Self {
            stdio: Arc::new(Mutex::new(HashMap::new())),
            stdio_snapshots: Arc::new(Mutex::new(HashMap::new())),
            stdio_init_locks: Arc::new(Mutex::new(HashMap::new())),
            stdio_exclusive_locks: Arc::new(Mutex::new(HashMap::new())),
            stdio_exclusive_owners: Arc::new(Mutex::new(HashMap::new())),
            stdio_session_exclusives: Arc::new(Mutex::new(HashMap::new())),
            http: Arc::new(Mutex::new(HashMap::new())),
            http_lookup: Arc::new(Mutex::new(HashMap::new())),
            reuse_hits: Arc::new(Mutex::new(0)),
            logger,
        }
    }

    async fn log_stdio_lifecycle(
        &self,
        event_type: DaemonEventType,
        session_key: &str,
        snapshot: &McpStdioSessionSnapshot,
        reason: Option<&str>,
        error: Option<&str>,
    ) {
        let Some(logger) = self.logger.as_ref() else {
            return;
        };
        let mut meta = json!({
            "session_key": display_session_key(session_key),
            "link_name": snapshot.link_name.clone(),
            "link_skill": snapshot.link_skill.clone(),
            "link_skill_doc": snapshot.link_skill_doc.clone(),
            "link_skill_path": snapshot.link_skill_path.clone(),
            "command_summary": snapshot.command_summary.clone(),
            "daemon_exclusive": snapshot.daemon_exclusive.clone(),
            "child_pid": snapshot.child_pid,
            "idle_ttl_secs": snapshot.idle_ttl_secs,
            "idle_for_secs": now_unix_secs().saturating_sub(snapshot.last_used_at_unix),
            "in_flight_requests": snapshot.in_flight_requests,
            "reuse_eligible": snapshot.reuse_eligible,
            "recent_stderr": snapshot.recent_stderr.clone(),
        });
        if let Some(reason) = reason {
            meta["reason"] = json!(reason);
        }
        let mut entry = DaemonLogEntry::new(event_type)
            .with_endpoint(snapshot.endpoint.clone())
            .with_protocol("mcp_stdio".to_string())
            .with_meta(meta);
        if let Some(error) = error {
            entry = entry.with_error(error.to_string());
        }
        let _ = logger.log(&entry).await;
    }

    async fn log_stdio_reap_deferred(
        &self,
        session_key: &str,
        snapshot: &McpStdioSessionSnapshot,
        contract: &CanReapContractView,
    ) {
        let Some(logger) = self.logger.as_ref() else {
            return;
        };
        let mut meta = json!({
            "session_key": display_session_key(session_key),
            "link_name": snapshot.link_name.clone(),
            "link_skill": snapshot.link_skill.clone(),
            "link_skill_doc": snapshot.link_skill_doc.clone(),
            "link_skill_path": snapshot.link_skill_path.clone(),
            "command_summary": snapshot.command_summary.clone(),
            "daemon_exclusive": snapshot.daemon_exclusive.clone(),
            "child_pid": snapshot.child_pid,
            "idle_ttl_secs": snapshot.idle_ttl_secs,
            "idle_for_secs": now_unix_secs().saturating_sub(snapshot.last_used_at_unix),
            "in_flight_requests": snapshot.in_flight_requests,
            "reason": contract.reason.clone(),
            "retry_after_secs": contract.retry_after_secs,
            "state": contract.state.clone(),
        });
        if let Some(can_reap) = contract.can_reap {
            meta["can_reap"] = json!(can_reap);
        }
        if let Some(checked_at_unix) = contract.checked_at_unix {
            meta["checked_at_unix"] = json!(checked_at_unix);
        }
        let mut entry = DaemonLogEntry::new(DaemonEventType::DaemonSessionReapDeferred)
            .with_endpoint(snapshot.endpoint.clone())
            .with_protocol("mcp_stdio".to_string())
            .with_meta(meta);
        if let Some(error) = &contract.last_error_summary {
            entry = entry.with_error(error.clone());
        }
        let _ = logger.log(&entry).await;
    }

    async fn upsert_stdio_snapshot<F>(&self, session_key: &str, update: F)
    where
        F: FnOnce(&mut McpStdioSessionSnapshot),
    {
        let mut snapshots = self.stdio_snapshots.lock().await;
        if let Some(snapshot) = snapshots.get_mut(session_key) {
            update(snapshot);
        }
    }

    async fn remove_stdio_snapshot(
        &self,
        session_key: &str,
        reason: &'static str,
        error: Option<String>,
    ) {
        let snapshot = {
            let mut snapshots = self.stdio_snapshots.lock().await;
            snapshots.remove(session_key)
        };
        if let Some(snapshot) = snapshot {
            self.log_stdio_lifecycle(
                DaemonEventType::DaemonSessionRemoved,
                session_key,
                &snapshot,
                Some(reason),
                error.as_deref(),
            )
            .await;
        }
    }

    async fn cleanup_idle(&self) {
        let http_cutoff = Instant::now() - Duration::from_secs(MCP_IDLE_TTL_SECS);
        let stdio_entries: Vec<(String, Arc<Mutex<McpStdioSession>>)> = {
            let map = self.stdio.lock().await;
            map.iter().map(|(k, s)| (k.clone(), s.clone())).collect()
        };
        let mut stdio_remove = Vec::new();
        for (key, session) in &stdio_entries {
            // Use try_lock to avoid blocking on sessions that may be held across .await in invoke_mcp.
            // If a session is busy, we'll check it again in the next cleanup cycle.
            if let Ok(mut guard) = session.try_lock() {
                if guard.idle_ttl_secs == 0 {
                    continue;
                }
                let now = Instant::now();
                let cutoff = now - Duration::from_secs(guard.idle_ttl_secs);
                if guard.last_used < cutoff {
                    if guard
                        .next_can_reap_probe_after
                        .is_some_and(|next_probe_after| next_probe_after > now)
                    {
                        continue;
                    }
                    let checked_at_unix = now_unix_secs();
                    let idle_for_secs = checked_at_unix.saturating_sub(guard.last_used_at_unix);
                    let idle_ttl_secs = guard.idle_ttl_secs;
                    match guard
                        .client
                        .probe_can_reap(
                            idle_for_secs,
                            idle_ttl_secs,
                            Duration::from_millis(MCP_CAN_REAP_PROBE_TIMEOUT_MS),
                        )
                        .await
                    {
                        Ok(result) => {
                            guard.can_reap_contract = CanReapContractView {
                                support: CanReapContractSupport::Supported,
                                checked_at_unix: Some(checked_at_unix),
                                can_reap: Some(result.can_reap),
                                reason: result.reason.clone(),
                                retry_after_secs: result.retry_after_secs,
                                state: result.state.clone(),
                                last_error_summary: None,
                            };
                            if result.can_reap {
                                guard.next_can_reap_probe_after = None;
                            } else {
                                let retry_after_secs = result
                                    .retry_after_secs
                                    .unwrap_or(MCP_CAN_REAP_RETRY_AFTER_SECS);
                                guard.next_can_reap_probe_after =
                                    Some(now + Duration::from_secs(retry_after_secs));
                                let contract = guard.can_reap_contract.clone();
                                drop(guard);
                                self.upsert_stdio_snapshot(key, |snapshot| {
                                    snapshot.can_reap_contract = contract.clone();
                                })
                                .await;
                                if let Some(snapshot) = {
                                    let snapshots = self.stdio_snapshots.lock().await;
                                    snapshots.get(key).cloned()
                                } {
                                    self.log_stdio_reap_deferred(key, &snapshot, &contract)
                                        .await;
                                }
                                continue;
                            }
                        }
                        Err(err) => {
                            guard.next_can_reap_probe_after = None;
                            guard.can_reap_contract = CanReapContractView {
                                support: if can_reap_method_not_found(&err) {
                                    CanReapContractSupport::Unsupported
                                } else {
                                    CanReapContractSupport::Error
                                },
                                checked_at_unix: Some(checked_at_unix),
                                can_reap: None,
                                reason: None,
                                retry_after_secs: None,
                                state: None,
                                last_error_summary: (!can_reap_method_not_found(&err))
                                    .then(|| can_reap_error_summary(&err)),
                            };
                        }
                    }
                    let contract = guard.can_reap_contract.clone();
                    // Idle cleanup is best-effort: even if shutdown waiting fails, we still drop
                    // the cached session so cleanup can make forward progress on the next cycle.
                    if let Err(err) = guard
                        .client
                        .kill_and_wait(Duration::from_secs(MCP_STDIO_EXIT_TIMEOUT_SECS))
                        .await
                    {
                        tracing::warn!(
                            session_key = %key,
                            error = %err,
                            "Failed waiting for idle MCP stdio session to exit after kill"
                        );
                    }
                    drop(guard);
                    self.upsert_stdio_snapshot(key, |snapshot| {
                        snapshot.can_reap_contract = contract;
                    })
                    .await;
                    stdio_remove.push(key.clone());
                }
            }
        }
        if !stdio_remove.is_empty() {
            {
                let mut map = self.stdio.lock().await;
                for key in &stdio_remove {
                    map.remove(key);
                }
            }
            for key in stdio_remove {
                self.cleanup_stdio_exclusive_for_session_key(&key).await;
                self.remove_stdio_snapshot(&key, "idle_reaped", None).await;
            }
        }

        let init_lock_cutoff = Instant::now() - Duration::from_secs(STDIO_INIT_LOCK_STALE_SECS);
        let mut lock_map = self.stdio_init_locks.lock().await;
        // Retain locks that are:
        // 1. Still in use (strong_count > 1 means someone is holding the lock), or
        // 2. Were touched recently (not stale)
        // This avoids dropping an init lock during an ongoing initialization,
        // which could otherwise allow a concurrent cold call to create a duplicate
        // lock and spawn another MCP process, breaking the singleflight guarantee.
        lock_map.retain(|_, v| Arc::strong_count(&v.lock) > 1 || v.touched_at >= init_lock_cutoff);

        let mut exclusive_lock_map = self.stdio_exclusive_locks.lock().await;
        exclusive_lock_map
            .retain(|_, v| Arc::strong_count(&v.lock) > 1 || v.touched_at >= init_lock_cutoff);

        let http_entries: Vec<(String, Arc<McpHttpSession>)> = {
            let map = self.http.lock().await;
            map.iter().map(|(k, s)| (k.clone(), s.clone())).collect()
        };
        let mut http_remove = Vec::new();
        for (key, session) in &http_entries {
            let last_used = *session.last_used.lock().await;
            if last_used < http_cutoff {
                http_remove.push(key.clone());
            }
        }
        if !http_remove.is_empty() {
            let removed_lookup_keys = {
                let mut map = self.http.lock().await;
                let mut removed = Vec::new();
                for key in http_remove {
                    if let Some(session) = map.remove(&key) {
                        removed.push(session.lookup_key.clone());
                    }
                }
                removed
            };
            if !removed_lookup_keys.is_empty() {
                let mut lookup = self.http_lookup.lock().await;
                for lookup_key in removed_lookup_keys {
                    lookup.remove(&lookup_key);
                }
            }
        }
    }

    async fn get_or_create_stdio(
        &self,
        session_key: &str,
        command: &str,
        args: &[String],
        spawn_options: &adapters::mcp::StdioSpawnOptions,
        request_timeout: Option<Duration>,
        metadata: StdioSessionRequestMetadata<'_>,
    ) -> Result<(Arc<Mutex<McpStdioSession>>, bool)> {
        let exclusive_keys = normalize_exclusive_keys(metadata.exclusive_keys);
        let metadata = StdioSessionRequestMetadata {
            idle_ttl_secs: metadata.idle_ttl_secs,
            link_name: metadata.link_name,
            link_skill: metadata.link_skill,
            link_skill_doc: metadata.link_skill_doc,
            link_skill_path: metadata.link_skill_path,
            endpoint: metadata.endpoint,
            exclusive_keys: &exclusive_keys,
        };

        // If exclusives are requested, enforce/claim them before returning an existing session
        // so the invariant holds even when the session was created without exclusives.
        let _exclusive_guards = if exclusive_keys.is_empty() {
            Vec::new()
        } else {
            self.acquire_stdio_exclusive_locks(&exclusive_keys).await
        };

        if !exclusive_keys.is_empty() {
            self.evict_stdio_exclusive_conflicts(session_key, &exclusive_keys)
                .await?;
        }

        let existing = {
            let map = self.stdio.lock().await;
            map.get(session_key).cloned()
        };
        if let Some(session) = existing {
            if let Some(reused) = self
                .try_reuse_existing_stdio_session(session_key, session, &metadata, &exclusive_keys)
                .await?
            {
                return Ok(reused);
            }
        }

        // Singleflight for stdio process initialization by endpoint key.
        // This avoids duplicate process spawns under concurrent cold requests.
        let key_lock = {
            let mut lock_map = self.stdio_init_locks.lock().await;
            let entry = lock_map
                .entry(session_key.to_string())
                .or_insert_with(|| InitLockEntry {
                    lock: Arc::new(Mutex::new(())),
                    touched_at: Instant::now(),
                });
            entry.touched_at = Instant::now();
            entry.lock.clone()
        };
        let _guard = key_lock.lock().await;

        let existing = {
            let map = self.stdio.lock().await;
            map.get(session_key).cloned()
        };
        if let Some(session) = existing {
            if let Some(reused) = self
                .try_reuse_existing_stdio_session(session_key, session, &metadata, &exclusive_keys)
                .await?
            {
                return Ok(reused);
            }
        }

        let client = adapters::mcp::McpStdioClient::connect_with_options_and_timeout(
            command,
            args,
            spawn_options.clone(),
            request_timeout.unwrap_or_else(
                adapters::mcp::transport::McpStdioTransport::default_request_timeout,
            ),
        )
        .await?;
        let created_at_unix = now_unix_secs();
        let command_summary = command_summary_from_endpoint(metadata.endpoint);
        let child_pid = client.child_id();
        let session = Arc::new(Mutex::new(McpStdioSession {
            child_pid,
            client,
            tools: None,
            tools_dirty: false,
            notifications: SessionNotificationFanout::default(),
            resource_subscriptions: HashMap::new(),
            last_used: Instant::now(),
            last_used_at_unix: created_at_unix,
            idle_ttl_secs: metadata.idle_ttl_secs.unwrap_or(MCP_IDLE_TTL_SECS),
            link_name: metadata.link_name.map(str::to_string),
            link_skill: metadata.link_skill.map(str::to_string),
            link_skill_doc: metadata.link_skill_doc.map(str::to_string),
            link_skill_path: metadata.link_skill_path.map(str::to_string),
            endpoint: metadata.endpoint.to_string(),
            daemon_exclusive: exclusive_keys.clone(),
            can_reap_contract: CanReapContractView::default(),
            next_can_reap_probe_after: None,
        }));

        {
            let mut map = self.stdio.lock().await;
            map.insert(session_key.to_string(), session.clone());
        }
        self.stdio_snapshots.lock().await.insert(
            session_key.to_string(),
            McpStdioSessionSnapshot {
                endpoint: metadata.endpoint.to_string(),
                link_name: metadata.link_name.map(str::to_string),
                link_skill: metadata.link_skill.map(str::to_string),
                link_skill_doc: metadata.link_skill_doc.map(str::to_string),
                link_skill_path: metadata.link_skill_path.map(str::to_string),
                command_summary,
                child_pid,
                started_at_unix: created_at_unix,
                last_used_at_unix: created_at_unix,
                idle_ttl_secs: metadata.idle_ttl_secs.unwrap_or(MCP_IDLE_TTL_SECS),
                daemon_exclusive: exclusive_keys.clone(),
                in_flight_requests: 0,
                reuse_eligible: true,
                can_reap_contract: CanReapContractView::default(),
                last_error_summary: None,
                recent_stderr: Vec::new(),
            },
        );
        if let Some(snapshot) = {
            let snapshots = self.stdio_snapshots.lock().await;
            snapshots.get(session_key).cloned()
        } {
            self.log_stdio_lifecycle(
                DaemonEventType::DaemonSessionCreated,
                session_key,
                &snapshot,
                None,
                None,
            )
            .await;
        }
        if !exclusive_keys.is_empty() {
            self.register_stdio_exclusive_keys(session_key, &exclusive_keys)
                .await;
        }
        Ok((session, false))
    }

    async fn get_stdio(&self, session_key: &str) -> Option<Arc<Mutex<McpStdioSession>>> {
        let map = self.stdio.lock().await;
        map.get(session_key).cloned()
    }

    async fn try_reuse_existing_stdio_session(
        &self,
        session_key: &str,
        session: Arc<Mutex<McpStdioSession>>,
        metadata: &StdioSessionRequestMetadata<'_>,
        exclusive_keys: &[String],
    ) -> Result<Option<(Arc<Mutex<McpStdioSession>>, bool)>> {
        let mut guard = session.lock().await;
        let child_exited = guard.client.child_has_exited().unwrap_or(false);
        if child_exited {
            let recent_stderr = redact_recent_stderr(guard.client.recent_stderr_lines(5).await);
            drop(guard);
            {
                let mut map = self.stdio.lock().await;
                map.remove(session_key);
            }
            self.cleanup_stdio_exclusive_for_session_key(session_key)
                .await;
            self.upsert_stdio_snapshot(session_key, |snapshot| {
                snapshot.reuse_eligible = false;
                snapshot.last_error_summary =
                    Some("cached MCP stdio child already exited".to_string());
                snapshot.recent_stderr = recent_stderr;
            })
            .await;
            self.remove_stdio_snapshot(
                session_key,
                "child_exited_before_reuse",
                Some("cached MCP stdio child already exited".to_string()),
            )
            .await;
            return Ok(None);
        }

        *self.reuse_hits.lock().await += 1;
        guard.apply_request_metadata(metadata);
        let recent_stderr = redact_recent_stderr(guard.client.recent_stderr_lines(5).await);
        let endpoint = guard.endpoint.clone();
        let link_name = guard.link_name.clone();
        let link_skill = guard.link_skill.clone();
        let link_skill_doc = guard.link_skill_doc.clone();
        let link_skill_path = guard.link_skill_path.clone();
        let child_pid = guard.child_pid;
        let idle_ttl_secs = guard.idle_ttl_secs;
        let last_used_at_unix = guard.last_used_at_unix;
        let daemon_exclusive = guard.daemon_exclusive.clone();
        let can_reap_contract = guard.can_reap_contract.clone();
        drop(guard);
        self.upsert_stdio_snapshot(session_key, |snapshot| {
            snapshot.endpoint = endpoint;
            snapshot.link_name = link_name;
            snapshot.link_skill = link_skill;
            snapshot.link_skill_doc = link_skill_doc;
            snapshot.link_skill_path = link_skill_path;
            snapshot.child_pid = child_pid;
            snapshot.idle_ttl_secs = idle_ttl_secs;
            snapshot.last_used_at_unix = last_used_at_unix;
            snapshot.daemon_exclusive = daemon_exclusive;
            snapshot.can_reap_contract = can_reap_contract;
            snapshot.recent_stderr = recent_stderr;
        })
        .await;
        if let Some(snapshot) = {
            let snapshots = self.stdio_snapshots.lock().await;
            snapshots.get(session_key).cloned()
        } {
            self.log_stdio_lifecycle(
                DaemonEventType::DaemonSessionReused,
                session_key,
                &snapshot,
                None,
                None,
            )
            .await;
        }
        if !exclusive_keys.is_empty() {
            self.register_stdio_exclusive_keys(session_key, exclusive_keys)
                .await;
        }
        Ok(Some((session, true)))
    }

    async fn mark_stdio_request_started(&self, session_key: &str) {
        self.upsert_stdio_snapshot(session_key, |snapshot| {
            snapshot.in_flight_requests = snapshot.in_flight_requests.saturating_add(1);
            snapshot.last_used_at_unix = now_unix_secs();
        })
        .await;
    }

    async fn mark_stdio_request_finished(
        &self,
        session_key: &str,
        request_error: Option<&anyhow::Error>,
        recent_stderr: Vec<String>,
        child_exited: bool,
    ) {
        let mut unhealthy_reason = None;
        self.upsert_stdio_snapshot(session_key, |snapshot| {
            snapshot.in_flight_requests = snapshot.in_flight_requests.saturating_sub(1);
            snapshot.last_used_at_unix = now_unix_secs();
            snapshot.recent_stderr = redact_recent_stderr(recent_stderr.clone());
            if let Some(err) = request_error {
                snapshot.last_error_summary = Some(truncate_for_session_summary(&err.to_string()));
            } else {
                snapshot.last_error_summary = None;
            }
            if child_exited {
                snapshot.reuse_eligible = false;
                unhealthy_reason = Some("child_exited");
            }
        })
        .await;
        if let Some(reason) = unhealthy_reason {
            let snapshot = {
                let snapshots = self.stdio_snapshots.lock().await;
                snapshots.get(session_key).cloned()
            };
            if let Some(snapshot) = snapshot {
                self.log_stdio_lifecycle(
                    DaemonEventType::DaemonSessionUnhealthy,
                    session_key,
                    &snapshot,
                    Some(reason),
                    request_error.map(|err| err.to_string()).as_deref(),
                )
                .await;
            }
        }
    }

    async fn acquire_stdio_exclusive_locks(
        &self,
        exclusive_keys: &[String],
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut keys = exclusive_keys.to_vec();
        keys.sort();
        keys.dedup();

        // Hold guards to keep locks alive for the duration of get_or_create_stdio.
        let mut guards = Vec::new();
        for key in keys {
            let lock = {
                let mut lock_map = self.stdio_exclusive_locks.lock().await;
                let entry = lock_map.entry(key).or_insert_with(|| InitLockEntry {
                    lock: Arc::new(Mutex::new(())),
                    touched_at: Instant::now(),
                });
                entry.touched_at = Instant::now();
                entry.lock.clone()
            };
            guards.push(lock.clone().lock_owned().await);
        }
        guards
    }

    async fn evict_stdio_exclusive_conflicts(
        &self,
        session_key: &str,
        exclusive_keys: &[String],
    ) -> Result<()> {
        // Discover distinct conflicting owners first so we don't scan the session map repeatedly.
        let mut owners = Vec::new();
        {
            let owners_map = self.stdio_exclusive_owners.lock().await;
            for key in exclusive_keys {
                if let Some(owner) = owners_map.get(key) {
                    if owner != session_key {
                        owners.push(owner.clone());
                    }
                }
            }
        }
        owners.sort();
        owners.dedup();

        for owner_session_key in owners {
            // Try-lock to avoid blocking; if busy, refuse to evict by default.
            let session_opt = {
                let map = self.stdio.lock().await;
                map.get(&owner_session_key).cloned()
            };
            let Some(session) = session_opt else {
                // Best-effort cleanup for stale owner mapping.
                {
                    let mut owners_map = self.stdio_exclusive_owners.lock().await;
                    for key in exclusive_keys {
                        if owners_map.get(key).is_some_and(|o| o == &owner_session_key) {
                            owners_map.remove(key);
                        }
                    }
                }
                self.cleanup_stdio_exclusive_for_session_key(&owner_session_key)
                    .await;
                continue;
            };

            match session.try_lock() {
                Ok(mut guard) => {
                    guard
                        .client
                        .kill_and_wait(Duration::from_secs(MCP_STDIO_EXIT_TIMEOUT_SECS))
                        .await
                        .with_context(|| {
                            format!(
                                "Failed waiting for conflicting MCP stdio session {} to exit",
                                redact_sensitive(&owner_session_key)
                            )
                        })?;
                    // Remove session + exclusive registry entries.
                    {
                        let mut map = self.stdio.lock().await;
                        map.remove(&owner_session_key);
                    }
                    self.cleanup_stdio_exclusive_for_session_key(&owner_session_key)
                        .await;
                    self.remove_stdio_snapshot(&owner_session_key, "exclusive_evicted", None)
                        .await;
                }
                Err(_) => {
                    // Find the first conflicting key for a helpful message.
                    let mut conflicting = None;
                    let owners_map = self.stdio_exclusive_owners.lock().await;
                    for key in exclusive_keys {
                        if owners_map.get(key).is_some_and(|o| o == &owner_session_key) {
                            conflicting = Some(key.clone());
                            break;
                        }
                    }
                    let key = conflicting.unwrap_or_else(|| "<unknown>".to_string());

                    // session_key format: "stdio:{endpoint}:{auth_fingerprint}[:{env_fingerprint}]"
                    // endpoint can contain ":", so parse from the last ":".
                    // With env fingerprint, we need to handle: "stdio:endpoint:auth_fp:env_fp"
                    // Without env fingerprint: "stdio:endpoint:auth_fp"
                    // Split from the end twice to handle both cases.
                    let (owner_endpoint, owner_auth_fp, owner_env_fp) = match owner_session_key
                        .strip_prefix("stdio:")
                    {
                        Some(rest) => {
                            // Try splitting twice for env fingerprint format
                            if let Some((before_env, _env_fp)) = rest.rsplit_once(':') {
                                if let Some((before_auth, auth_fp)) = before_env.rsplit_once(':') {
                                    // Has both auth and env fingerprints
                                    (Some(before_auth), Some(auth_fp), Some(_env_fp))
                                } else {
                                    // Only has auth fingerprint, env_fp was actually endpoint
                                    (Some(before_env), Some(_env_fp), None)
                                }
                            } else {
                                // No fingerprint at all
                                (Some(rest), None, None)
                            }
                        }
                        None => (None, None, None),
                    };
                    let owner_endpoint = owner_endpoint
                        .map(redact_endpoint)
                        .map(|s| redact_sensitive(&s));
                    let owner_auth_fp = owner_auth_fp.map(|s| s.to_string());
                    let owner_env_fp = owner_env_fp.map(|s| s.to_string());
                    bail!(
                        "Another MCP stdio session is currently using daemon exclusive key {} (owner_endpoint={}, owner_auth_fingerprint={}, owner_env_fingerprint={}). Close it (or run `uxc daemon stop`) before switching.",
                        key,
                        owner_endpoint.unwrap_or_else(|| "<unknown>".to_string()),
                        owner_auth_fp.unwrap_or_else(|| "<unknown>".to_string()),
                        owner_env_fp.unwrap_or_else(|| "<none>".to_string()),
                    );
                }
            };
        }

        Ok(())
    }

    async fn register_stdio_exclusive_keys(&self, session_key: &str, exclusive_keys: &[String]) {
        if exclusive_keys.is_empty() {
            return;
        }

        // Replace previous mapping for this session key.
        self.cleanup_stdio_exclusive_for_session_key(session_key)
            .await;

        {
            let mut session_map = self.stdio_session_exclusives.lock().await;
            session_map.insert(session_key.to_string(), exclusive_keys.to_vec());
        }
        {
            let mut owners_map = self.stdio_exclusive_owners.lock().await;
            for key in exclusive_keys {
                owners_map.insert(key.clone(), session_key.to_string());
            }
        }
    }

    async fn cleanup_stdio_exclusive_for_session_key(&self, session_key: &str) {
        let keys = {
            let mut session_map = self.stdio_session_exclusives.lock().await;
            session_map.remove(session_key).unwrap_or_default()
        };

        if keys.is_empty() {
            return;
        }

        let mut owners_map = self.stdio_exclusive_owners.lock().await;
        for key in keys {
            if owners_map.get(&key).is_some_and(|o| o == session_key) {
                owners_map.remove(&key);
            }
        }
    }

    async fn get_http_by_lookup_key(
        &self,
        lookup_key: &str,
    ) -> Option<(Arc<McpHttpSession>, bool)> {
        let session_key = {
            let lookup = self.http_lookup.lock().await;
            lookup.get(lookup_key).cloned()
        }?;
        let session = {
            let map = self.http.lock().await;
            map.get(&session_key).cloned()
        };
        if let Some(session) = session {
            *self.reuse_hits.lock().await += 1;
            session.mark_used().await;
            return Some((session, true));
        }

        let mut lookup = self.http_lookup.lock().await;
        lookup.remove(lookup_key);
        None
    }

    async fn has_http_by_lookup_key(&self, lookup_key: &str) -> bool {
        let session_key = {
            let lookup = self.http_lookup.lock().await;
            lookup.get(lookup_key).cloned()
        };
        let Some(session_key) = session_key else {
            return false;
        };
        let map = self.http.lock().await;
        map.contains_key(&session_key)
    }

    async fn get_or_create_http(
        &self,
        lookup_key: &str,
        key: &str,
        resolved: &adapters::mcp::ResolvedMcpHttpTransport,
        auth_profile: Option<Profile>,
        request_timeout: Option<Duration>,
    ) -> Result<(Arc<McpHttpSession>, bool)> {
        let existing = {
            let map = self.http.lock().await;
            map.get(key).cloned()
        };
        if let Some(session) = existing {
            {
                let mut lookup = self.http_lookup.lock().await;
                lookup.insert(lookup_key.to_string(), key.to_string());
            }
            *self.reuse_hits.lock().await += 1;
            session.mark_used().await;
            return Ok((session, true));
        }

        let transport = adapters::mcp::McpRemoteTransport::with_auth_and_timeout(
            resolved.clone(),
            auth_profile,
            request_timeout.unwrap_or_else(|| Duration::from_secs(30)),
        )?;
        transport.initialize().await?;
        let session = Arc::new(McpHttpSession {
            transport,
            notifications: Mutex::new(SessionNotificationFanout::default()),
            resource_subscriptions: Mutex::new(HashMap::new()),
            resource_subscription_ops: Mutex::new(()),
            lookup_key: lookup_key.to_string(),
            last_used: Mutex::new(Instant::now()),
        });

        {
            let mut map = self.http.lock().await;
            map.insert(key.to_string(), session.clone());
        }
        {
            let mut lookup = self.http_lookup.lock().await;
            lookup.insert(lookup_key.to_string(), key.to_string());
        }
        Ok((session, false))
    }

    async fn status_counts(&self) -> (usize, usize, u64) {
        let stdio_count = self.stdio.lock().await.len();
        let http_count = self.http.lock().await.len();
        let reuse_hits = *self.reuse_hits.lock().await;
        (stdio_count, http_count, reuse_hits)
    }

    async fn session_views(&self) -> Vec<DaemonSessionView> {
        let stdio_entries: Vec<(String, Arc<Mutex<McpStdioSession>>)> = {
            let map = self.stdio.lock().await;
            map.iter().map(|(k, s)| (k.clone(), s.clone())).collect()
        };
        for (session_key, session) in &stdio_entries {
            if let Ok(mut guard) = session.try_lock() {
                let recent_stderr = redact_recent_stderr(guard.client.recent_stderr_lines(5).await);
                let child_exited = guard.client.child_has_exited().unwrap_or(false);
                self.upsert_stdio_snapshot(session_key, |snapshot| {
                    snapshot.endpoint = guard.endpoint.clone();
                    snapshot.link_name = guard.link_name.clone();
                    snapshot.link_skill = guard.link_skill.clone();
                    snapshot.link_skill_doc = guard.link_skill_doc.clone();
                    snapshot.link_skill_path = guard.link_skill_path.clone();
                    snapshot.child_pid = guard.child_pid;
                    snapshot.last_used_at_unix = guard.last_used_at_unix;
                    snapshot.idle_ttl_secs = guard.idle_ttl_secs;
                    snapshot.daemon_exclusive = guard.daemon_exclusive.clone();
                    snapshot.can_reap_contract = guard.can_reap_contract.clone();
                    snapshot.recent_stderr = recent_stderr;
                    if child_exited {
                        snapshot.reuse_eligible = false;
                    }
                })
                .await;
            }
        }
        let snapshots = self.stdio_snapshots.lock().await;
        let now_unix = now_unix_secs();
        let mut views = snapshots
            .iter()
            .map(|(session_key, snapshot)| snapshot.to_view(session_key, now_unix))
            .collect::<Vec<_>>();
        views.sort_by(|a, b| a.session_key.cmp(&b.session_key));
        views
    }
}

fn resolve_stream_subscription_protocol(request: &SubscribeStartRequest) -> Result<String> {
    if let Some(transport_hint) = request.transport_hint.as_ref() {
        let lower = request.endpoint.to_ascii_lowercase();
        match transport_hint {
            SubscriptionTransportHint::Websocket => {
                if !lower.starts_with("ws://") && !lower.starts_with("wss://") {
                    bail!("websocket subscription transport requires a ws:// or wss:// endpoint");
                }
                return Ok("websocket".to_string());
            }
            SubscriptionTransportHint::DiscordGateway => {
                if !lower.starts_with("http://") && !lower.starts_with("https://") {
                    bail!(
                        "discord-gateway transport requires an http:// or https:// Discord API endpoint"
                    );
                }
                return Ok("discord_gateway".to_string());
            }
            SubscriptionTransportHint::SlackSocketMode => {
                if !lower.starts_with("http://") && !lower.starts_with("https://") {
                    bail!(
                        "slack-socket-mode transport requires an http:// or https:// Slack API endpoint"
                    );
                }
                return Ok("slack_socket_mode".to_string());
            }
            SubscriptionTransportHint::FeishuLongConnection => {
                if !lower.starts_with("http://") && !lower.starts_with("https://") {
                    bail!(
                        "feishu-long-connection transport requires an http:// or https:// Feishu/Lark API endpoint"
                    );
                }
                return Ok("feishu_long_connection".to_string());
            }
        }
    }

    if let Some(operation_id) = request.operation_id.as_deref() {
        let lower = request.endpoint.to_ascii_lowercase();
        if operation_id.starts_with("subscription/") {
            if !lower.starts_with("http://") && !lower.starts_with("https://") {
                bail!(
                    "GraphQL subscriptions require an http:// or https:// endpoint for schema discovery"
                );
            }
            return Ok("graphql".to_string());
        }
        if lower.starts_with("ws://") || lower.starts_with("wss://") {
            resolve_jsonrpc_unsubscribe_operation(operation_id)?;
            return Ok("jsonrpc".to_string());
        }
        bail!(
            "JSON-RPC subscriptions require a ws:// or wss:// endpoint; GraphQL subscriptions require subscription/<field> on an http:// or https:// endpoint"
        );
    }

    if request.resource_uri.is_some() {
        if !adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint)
            && !adapters::mcp::McpAdapter::is_http_url(&request.endpoint)
        {
            bail!(
                "MCP subscriptions require a stdio command or http(s) MCP endpoint when --resource-uri is set"
            );
        }
        return Ok("mcp".to_string());
    }

    if request.endpoint.starts_with("http://") || request.endpoint.starts_with("https://") {
        return Ok("http".to_string());
    }

    bail!("subscribe start requires an http(s) endpoint or --resource-uri for MCP subscriptions")
}

impl SubscriptionManager {
    fn new(store_path: PathBuf) -> Result<Self> {
        let manager = Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            terminal_jobs: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(0)),
            store_path,
        };
        manager.load_persisted_records()?;
        Ok(manager)
    }

    fn load_persisted_records(&self) -> Result<()> {
        if let Some(parent) = self.store_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create subscription store directory {}",
                    parent.display()
                )
            })?;
        }
        if !self.store_path.exists() {
            return Ok(());
        }
        let raw = fs::read_to_string(&self.store_path).with_context(|| {
            format!(
                "Failed to read subscription store {}",
                self.store_path.display()
            )
        })?;
        if raw.trim().is_empty() {
            return Ok(());
        }
        let store: PersistedSubscriptionStore = match serde_json::from_str(&raw) {
            Ok(store) => store,
            Err(err) => {
                quarantine_subscription_store(&self.store_path, "parse_error")?;
                tracing::warn!(
                    "Ignoring malformed subscription store {}: {}",
                    self.store_path.display(),
                    err
                );
                return Ok(());
            }
        };
        if store.version != "v1" {
            quarantine_subscription_store(&self.store_path, "unsupported_version")?;
            tracing::warn!(
                "Ignoring unsupported subscription store version '{}' at {}",
                store.version,
                self.store_path.display()
            );
            return Ok(());
        }
        let mut next_id = 0_u64;
        let mut jobs = HashMap::new();
        let mut sanitized_records = Vec::new();
        let mut skipped_records = 0_usize;
        for record in store.jobs {
            next_id = next_id.max(parse_subscription_numeric_id(&record.view.job_id).unwrap_or(0));

            if !record_should_persist(&record) {
                skipped_records += 1;
                continue;
            }

            let sink_path = match persisted_record_sink_path(&record) {
                Ok(path) => path,
                Err(err) => {
                    skipped_records += 1;
                    tracing::warn!(
                        "Ignoring persisted subscription {} during daemon init: {}",
                        record.view.job_id,
                        err
                    );
                    continue;
                }
            };

            let (stop_tx, _) = watch::channel(false);
            jobs.insert(
                record.view.job_id.clone(),
                Arc::new(SubscriptionJobEntry {
                    request: record.request.clone(),
                    sink_path,
                    view: Arc::new(Mutex::new(record.view.clone())),
                    stop_tx: Mutex::new(stop_tx),
                    task: Mutex::new(None),
                }),
            );
            sanitized_records.push(record);
        }
        *self
            .jobs
            .try_lock()
            .expect("subscription job map should not be contended during init") = jobs;
        *self
            .next_id
            .try_lock()
            .expect("subscription id counter should not be contended during init") = next_id;
        if skipped_records > 0 {
            tracing::warn!(
                "Dropped {} invalid or non-resumable persisted subscriptions from {}",
                skipped_records,
                self.store_path.display()
            );
            if let Err(err) = write_subscription_store(&self.store_path, &sanitized_records) {
                tracing::warn!(
                    "Failed to rewrite sanitized subscription store {} after dropping invalid or non-resumable records: {}",
                    self.store_path.display(),
                    err
                );
            }
        }
        Ok(())
    }

    async fn snapshot_persisted_records(&self) -> Vec<PersistedSubscriptionRecord> {
        let entries = {
            let jobs = self.jobs.lock().await;
            jobs.values().cloned().collect::<Vec<_>>()
        };
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            let record = PersistedSubscriptionRecord {
                request: entry.request.clone(),
                view: entry.view.lock().await.clone(),
            };
            if record_should_persist(&record) {
                records.push(record);
            }
        }
        records.sort_by(|a, b| a.view.job_id.cmp(&b.view.job_id));
        records
    }

    async fn persist_state(&self) -> Result<()> {
        let records = self.snapshot_persisted_records().await;
        write_subscription_store(&self.store_path, &records)
    }

    async fn cleanup_terminal_jobs(&self) {
        let now = now_unix_secs();
        let mut terminal = self.terminal_jobs.lock().await;
        let mut expired = Vec::new();
        for (job_id, entry) in terminal.iter() {
            if entry.expires_at_unix <= now {
                expired.push((job_id.clone(), entry.sink_path.clone(), entry.memory_backed));
            }
        }
        for (job_id, _, _) in &expired {
            terminal.remove(job_id);
        }
        drop(terminal);
        for (_, path, memory_backed) in expired {
            if memory_backed {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
    }

    async fn prepare_request(
        &self,
        runtime: &DaemonRuntime,
        request: &SubscribeStartRequest,
    ) -> Result<PreparedSubscription> {
        let prepared_sink = match request.sink.as_str() {
            "memory:" => PreparedSubscriptionSink::Memory,
            _ => {
                let sink_path = parse_file_sink(&request.sink)?;
                if is_nonstandard_subscription_sink_path(&sink_path) {
                    tracing::warn!(
                        "subscription sink path is outside HOME/temp directories: {}. This path may be unavailable after daemon restart if permissions or mounts change.",
                        sink_path.display()
                    );
                }
                PreparedSubscriptionSink::File(sink_path)
            }
        };
        let sink_spec = match &prepared_sink {
            PreparedSubscriptionSink::File(sink_path) => format!("file:{}", sink_path.display()),
            PreparedSubscriptionSink::Memory => "memory:".to_string(),
        };
        if request.resource_uri.is_some() && request.operation_id.is_some() {
            bail!("subscribe start cannot combine --resource-uri with an operation_id");
        }
        if matches!(
            request.transport_hint,
            Some(SubscriptionTransportHint::Websocket)
        ) {
            if request.operation_id.is_some() {
                bail!("websocket transport cannot be combined with an operation_id");
            }
            if request.resource_uri.is_some() {
                bail!("websocket transport cannot be combined with --resource-uri");
            }
            if request.mode != SubscriptionMode::Stream {
                bail!("websocket transport is only valid with stream mode");
            }
        } else if matches!(
            request.transport_hint,
            Some(SubscriptionTransportHint::DiscordGateway)
        ) {
            if request.operation_id.is_some() {
                bail!("discord-gateway transport cannot be combined with an operation_id");
            }
            if request.resource_uri.is_some() {
                bail!("discord-gateway transport cannot be combined with --resource-uri");
            }
            if request.mode != SubscriptionMode::Stream {
                bail!("discord-gateway transport is only valid with stream mode");
            }
            if !request.subprotocols.is_empty() || !request.initial_text_frames.is_empty() {
                bail!("discord-gateway transport manages its own websocket setup and does not accept --subprotocol or --init-frame");
            }
        } else if matches!(
            request.transport_hint,
            Some(SubscriptionTransportHint::SlackSocketMode)
        ) {
            if request.operation_id.is_some() {
                bail!("slack-socket-mode transport cannot be combined with an operation_id");
            }
            if request.resource_uri.is_some() {
                bail!("slack-socket-mode transport cannot be combined with --resource-uri");
            }
            if request.mode != SubscriptionMode::Stream {
                bail!("slack-socket-mode transport is only valid with stream mode");
            }
            if !request.subprotocols.is_empty() || !request.initial_text_frames.is_empty() {
                bail!("slack-socket-mode transport manages its own websocket setup and does not accept --subprotocol or --init-frame");
            }
        } else if matches!(
            request.transport_hint,
            Some(SubscriptionTransportHint::FeishuLongConnection)
        ) {
            if request.operation_id.is_some() {
                bail!("feishu-long-connection transport cannot be combined with an operation_id");
            }
            if request.resource_uri.is_some() {
                bail!("feishu-long-connection transport cannot be combined with --resource-uri");
            }
            if request.mode != SubscriptionMode::Stream {
                bail!("feishu-long-connection transport is only valid with stream mode");
            }
            if !request.subprotocols.is_empty() || !request.initial_text_frames.is_empty() {
                bail!("feishu-long-connection transport manages its own websocket setup and does not accept --subprotocol or --init-frame");
            }
        } else if !request.subprotocols.is_empty() || !request.initial_text_frames.is_empty() {
            bail!("websocket subprotocols and init frames require websocket transport");
        }
        if request.args.is_some()
            && request.operation_id.is_none()
            && !matches!(
                request.transport_hint,
                Some(SubscriptionTransportHint::DiscordGateway)
                    | Some(SubscriptionTransportHint::FeishuLongConnection)
            )
        {
            bail!("subscribe start cannot accept args without an operation_id");
        }
        match request.mode {
            SubscriptionMode::Stream => {
                if request.poll_config.is_some() {
                    bail!("--poll-config is only valid with --mode poll");
                }
            }
            SubscriptionMode::Poll => {
                let _ = resolve_poll_subscription_config(request)?;
            }
        }
        let protocol = match request.mode {
            SubscriptionMode::Stream => resolve_stream_subscription_protocol(request)?,
            SubscriptionMode::Poll => runtime.detect_poll_subscription_protocol(request).await?,
        };
        Ok(PreparedSubscription {
            sink: prepared_sink,
            sink_spec,
            protocol,
        })
    }

    fn spawn_job_task(
        &self,
        runtime: &DaemonRuntime,
        job_id: &str,
        request: &SubscribeStartRequest,
        sink_path: PathBuf,
        view: Arc<Mutex<SubscriptionJobView>>,
        stop_rx: watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        let manager = self.clone();
        let request_clone = request.clone();
        let job_id_clone = job_id.to_string();
        let runtime_clone = runtime.clone();
        tokio::spawn(async move {
            let result = match request_clone.mode {
                SubscriptionMode::Stream => {
                    run_stream_subscription_job(
                        &runtime_clone,
                        &job_id_clone,
                        &request_clone,
                        sink_path,
                        view.clone(),
                        stop_rx,
                    )
                    .await
                }
                SubscriptionMode::Poll => {
                    run_poll_subscription_job(
                        &runtime_clone,
                        &job_id_clone,
                        &request_clone,
                        sink_path,
                        view.clone(),
                        stop_rx,
                    )
                    .await
                }
            };

            let mut guard = view.lock().await;
            let was_resumed = guard.status == "resumed" || guard.restart_count > 0;
            if guard.status != "stopped" {
                match result {
                    Ok(()) => {
                        guard.status = "stopped".to_string();
                    }
                    Err(err) => {
                        let message = err.to_string();
                        if was_resumed {
                            guard.status = SUB_STATUS_RESUME_FAILED.to_string();
                            guard.last_resume_error = Some(message.clone());
                        } else {
                            guard.status = "failed".to_string();
                        }
                        guard.last_error = Some(message);
                    }
                }
            }
            guard.stopped_at_unix = Some(now_unix_secs());
            drop(guard);
            if let Err(err) = manager.persist_state().await {
                tracing::warn!(
                    "Failed to persist subscription state after task exit: {}",
                    err
                );
            }
        })
    }

    async fn resume_all(&self, runtime: &DaemonRuntime) -> Result<()> {
        self.cleanup_terminal_jobs().await;
        let entries = {
            let jobs = self.jobs.lock().await;
            jobs.values().cloned().collect::<Vec<_>>()
        };
        for entry in entries {
            if entry.request.ephemeral {
                let mut view = entry.view.lock().await;
                view.status = SUB_STATUS_STOPPED_AFTER_RESTART.to_string();
                view.stopped_at_unix = Some(now_unix_secs());
                view.last_resume_error = None;
                continue;
            }

            let prepared = match self.prepare_request(runtime, &entry.request).await {
                Ok(prepared) => prepared,
                Err(err) => {
                    let mut view = entry.view.lock().await;
                    view.status = SUB_STATUS_RESUME_FAILED.to_string();
                    view.last_resume_error = Some(err.to_string());
                    view.stopped_at_unix = Some(now_unix_secs());
                    continue;
                }
            };
            let sink_path = match prepared.sink {
                PreparedSubscriptionSink::File(path) => path,
                PreparedSubscriptionSink::Memory => {
                    let mut view = entry.view.lock().await;
                    view.status = SUB_STATUS_STOPPED_AFTER_RESTART.to_string();
                    view.stopped_at_unix = Some(now_unix_secs());
                    view.last_resume_error = None;
                    continue;
                }
            };

            let (stop_tx, stop_rx) = watch::channel(false);
            {
                let mut sender = entry.stop_tx.lock().await;
                *sender = stop_tx;
            }
            let job_id = {
                let mut view = entry.view.lock().await;
                view.endpoint = entry.request.endpoint.clone();
                view.protocol = prepared.protocol;
                view.sink = prepared.sink_spec;
                view.resource_uri = entry.request.resource_uri.clone();
                view.status = "resumed".to_string();
                view.durable = true;
                view.auto_resume = true;
                if view.resume_strategy.is_empty() {
                    view.resume_strategy = "reconnect".to_string();
                }
                view.started_at_unix = Some(now_unix_secs());
                view.stopped_at_unix = None;
                view.last_error = None;
                view.restart_count = view.restart_count.saturating_add(1);
                view.last_resume_at_unix = Some(now_unix_secs());
                view.last_resume_error = None;
                view.job_id.clone()
            };
            let task = self.spawn_job_task(
                runtime,
                &job_id,
                &entry.request,
                sink_path,
                entry.view.clone(),
                stop_rx,
            );
            *entry.task.lock().await = Some(task);
        }
        self.persist_state().await
    }

    async fn start(
        &self,
        runtime: &DaemonRuntime,
        request: &SubscribeStartRequest,
    ) -> Result<SubscribeStartResponse> {
        self.cleanup_terminal_jobs().await;
        let prepared = self.prepare_request(runtime, request).await?;

        let job_id = {
            let mut next = self.next_id.lock().await;
            *next += 1;
            format!("sub_{}", *next)
        };
        let sink_path = match prepared.sink {
            PreparedSubscriptionSink::File(path) => path,
            PreparedSubscriptionSink::Memory => internal_memory_sink_path(&job_id),
        };
        let now = now_unix_secs();
        let view = Arc::new(Mutex::new(SubscriptionJobView {
            job_id: job_id.clone(),
            mode: request.mode,
            endpoint: request.endpoint.clone(),
            protocol: prepared.protocol.clone(),
            sink: prepared.sink_spec.clone(),
            resource_uri: request.resource_uri.clone(),
            status: "running".to_string(),
            durable: !request.ephemeral && !sink_is_memory(&request.sink),
            auto_resume: !request.ephemeral && !sink_is_memory(&request.sink),
            resume_strategy: if request.ephemeral || sink_is_memory(&request.sink) {
                "none".to_string()
            } else {
                "reconnect".to_string()
            },
            created_at_unix: now,
            started_at_unix: Some(now),
            stopped_at_unix: None,
            last_event_at_unix: None,
            last_error: None,
            restart_count: 0,
            last_resume_at_unix: None,
            last_resume_error: None,
            reconnect_count: 0,
            written_events: 0,
        }));
        let (stop_tx, stop_rx) = watch::channel(false);
        let entry = Arc::new(SubscriptionJobEntry {
            request: request.clone(),
            sink_path: sink_path.clone(),
            view: view.clone(),
            stop_tx: Mutex::new(stop_tx),
            task: Mutex::new(None),
        });
        {
            let mut jobs = self.jobs.lock().await;
            jobs.insert(job_id.clone(), entry.clone());
        }
        if let Err(err) = self.persist_state().await {
            let mut jobs = self.jobs.lock().await;
            jobs.remove(&job_id);
            return Err(err);
        }
        let task = self.spawn_job_task(runtime, &job_id, request, sink_path, view.clone(), stop_rx);
        *entry.task.lock().await = Some(task);

        let guard = view.lock().await;
        Ok(SubscribeStartResponse {
            job_id,
            mode: request.mode,
            protocol: prepared.protocol,
            endpoint: guard.endpoint.clone(),
            sink: prepared.sink_spec,
            resource_uri: guard.resource_uri.clone(),
            status: guard.status.clone(),
        })
    }

    async fn list(&self) -> Vec<SubscriptionJobView> {
        self.cleanup_terminal_jobs().await;
        let entries = {
            let jobs = self.jobs.lock().await;
            jobs.values().cloned().collect::<Vec<_>>()
        };
        let mut views = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry.request.internal {
                continue;
            }
            views.push(entry.view.lock().await.clone());
        }
        views.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        views
    }

    async fn status(&self, job_id: &str) -> Result<SubscriptionJobView> {
        self.cleanup_terminal_jobs().await;
        let entry = {
            let jobs = self.jobs.lock().await;
            jobs.get(job_id).cloned()
        };
        if let Some(entry) = entry {
            return Ok(entry.view.lock().await.clone());
        }
        let terminal = {
            let jobs = self.terminal_jobs.lock().await;
            jobs.get(job_id).cloned()
        };
        terminal
            .map(|entry| entry.view)
            .ok_or_else(|| {
                UxcError::OperationNotFound(format!("subscription job not found: {}", job_id))
            })
            .map_err(Into::into)
    }

    async fn stop(&self, job_id: &str) -> Result<SubscribeStopResponse> {
        self.cleanup_terminal_jobs().await;
        let entry = {
            let jobs = self.jobs.lock().await;
            jobs.get(job_id).cloned()
        }
        .ok_or_else(|| {
            UxcError::OperationNotFound(format!("subscription job not found: {}", job_id))
        })?;

        let _ = entry.stop_tx.lock().await.send(true);
        if let Some(mut handle) = entry.task.lock().await.take() {
            if tokio::time::timeout(
                Duration::from_secs(SUBSCRIPTION_STOP_TIMEOUT_SECS),
                &mut handle,
            )
            .await
            .is_err()
            {
                tracing::warn!(
                    "subscription job {} did not stop within {}s; aborting task",
                    job_id,
                    SUBSCRIPTION_STOP_TIMEOUT_SECS
                );
                handle.abort();
                let _ = handle.await;
            }
        }
        let view = {
            let mut guard = entry.view.lock().await;
            guard.status = "stopped".to_string();
            guard.stopped_at_unix = Some(now_unix_secs());
            guard.clone()
        };
        {
            let mut jobs = self.jobs.lock().await;
            jobs.remove(job_id);
        }
        {
            let mut terminal = self.terminal_jobs.lock().await;
            terminal.insert(
                job_id.to_string(),
                TerminalSubscriptionEntry {
                    view,
                    sink_path: entry.sink_path.clone(),
                    expires_at_unix: now_unix_secs().saturating_add(SUBSCRIPTION_TERMINAL_TTL_SECS),
                    memory_backed: sink_is_memory(&entry.request.sink),
                },
            );
        }
        self.persist_state().await?;
        Ok(SubscribeStopResponse {
            job_id: job_id.to_string(),
            stopped: true,
        })
    }

    async fn events(
        &self,
        request: &SubscriptionEventsRequest,
    ) -> Result<SubscriptionEventsResponse> {
        self.cleanup_terminal_jobs().await;
        let limit = request.limit.clamp(1, SUBSCRIPTION_EVENTS_MAX_LIMIT);
        let wait_ms = request.wait_ms.min(SUBSCRIPTION_EVENTS_MAX_WAIT_MS);
        let deadline = Instant::now() + Duration::from_millis(wait_ms);

        loop {
            let active_entry = {
                let jobs = self.jobs.lock().await;
                jobs.get(&request.job_id).cloned()
            };
            let terminal_entry = if active_entry.is_none() {
                let jobs = self.terminal_jobs.lock().await;
                jobs.get(&request.job_id).cloned()
            } else {
                None
            };

            let (status, sink_path, retain_all) = if let Some(entry) = active_entry {
                (
                    entry.view.lock().await.status.clone(),
                    entry.sink_path.clone(),
                    entry.request.internal,
                )
            } else if let Some(entry) = terminal_entry {
                (entry.view.status.clone(), entry.sink_path.clone(), false)
            } else {
                return Err(UxcError::OperationNotFound(format!(
                    "subscription job not found: {}",
                    request.job_id
                ))
                .into());
            };

            let loaded =
                load_subscription_events(&sink_path, request.after_seq, limit, retain_all).await?;
            if !loaded.events.is_empty() || wait_ms == 0 || status != "running" {
                return Ok(SubscriptionEventsResponse {
                    job_id: request.job_id.clone(),
                    status,
                    events: loaded.events,
                    next_after_seq: loaded.next_after_seq,
                    has_more: loaded.has_more,
                });
            }

            if Instant::now() >= deadline {
                return Ok(SubscriptionEventsResponse {
                    job_id: request.job_id.clone(),
                    status,
                    events: Vec::new(),
                    next_after_seq: request.after_seq,
                    has_more: false,
                });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl ManagedSourceManager {
    fn new(store: ManagedSourceStore) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            store,
        }
    }

    async fn ensure(
        &self,
        runtime: &DaemonRuntime,
        request: ManagedSourceEnsureRequest,
    ) -> Result<ManagedSourceEnsureResponse> {
        validate_managed_source_identity(&request.namespace, &request.source_key)?;
        let spec_key = compute_managed_source_spec_key(&request.spec)?;
        let identity_key = managed_source_identity_key(&request.namespace, &request.source_key);
        let existing = self
            .store
            .get_source(&request.namespace, &request.source_key)
            .await?;

        let mut reused = false;
        let mut replaced_previous = false;
        let record = if let Some(existing) = existing {
            if existing.spec_key == spec_key && existing.underlying_job_id.is_some() {
                reused = true;
                existing
            } else {
                if existing.underlying_job_id.is_some() {
                    let _ = self
                        .stop_internal(runtime, &request.namespace, &request.source_key)
                        .await;
                }
                replaced_previous = existing.spec_key != spec_key;
                ManagedSourceRecord {
                    namespace: request.namespace.clone(),
                    source_key: request.source_key.clone(),
                    spec_json: serde_json::to_value(&request.spec)?,
                    spec_key,
                    run_id: new_managed_source_run_id(),
                    stream_id: existing.stream_id,
                    status: "starting".to_string(),
                    created_at_unix: existing.created_at_unix,
                    updated_at_unix: now_unix_secs(),
                    started_at_unix: None,
                    stopped_at_unix: None,
                    last_error: None,
                    underlying_job_id: None,
                    mirrored_after_seq: 0,
                }
            }
        } else {
            ManagedSourceRecord {
                namespace: request.namespace.clone(),
                source_key: request.source_key.clone(),
                spec_json: serde_json::to_value(&request.spec)?,
                spec_key,
                run_id: new_managed_source_run_id(),
                stream_id: managed_stream_id(&request.namespace, &request.source_key),
                status: "starting".to_string(),
                created_at_unix: now_unix_secs(),
                updated_at_unix: now_unix_secs(),
                started_at_unix: None,
                stopped_at_unix: None,
                last_error: None,
                underlying_job_id: None,
                mirrored_after_seq: 0,
            }
        };

        if reused {
            self.ensure_tailer_for_record(runtime, record.clone())
                .await?;
            let state = self.status(&request.namespace, &request.source_key).await?;
            return Ok(ManagedSourceEnsureResponse {
                namespace: state.namespace,
                source_key: state.source_key,
                run_id: state.run_id,
                stream_id: state.stream_id,
                status: state.status,
                reused: true,
                replaced_previous: false,
            });
        }

        let started = self
            .start_managed_source(runtime, record, request.spec.clone())
            .await?;
        self.entries.lock().await.remove(&identity_key);
        self.ensure_tailer_for_record(runtime, started.clone())
            .await?;
        let state = self.status(&request.namespace, &request.source_key).await?;
        Ok(ManagedSourceEnsureResponse {
            namespace: state.namespace,
            source_key: state.source_key,
            run_id: state.run_id,
            stream_id: state.stream_id,
            status: state.status,
            reused: false,
            replaced_previous,
        })
    }

    async fn status(&self, namespace: &str, source_key: &str) -> Result<ManagedSourceView> {
        let identity_key = managed_source_identity_key(namespace, source_key);
        if let Some(entry) = self.entries.lock().await.get(&identity_key).cloned() {
            return Ok(entry.state.lock().await.clone());
        }
        let record = self
            .store
            .get_source(namespace, source_key)
            .await?
            .ok_or_else(|| {
                UxcError::OperationNotFound(format!(
                    "managed source not found: {}/{}",
                    namespace, source_key
                ))
            })?;
        Ok(view_from_record(&record))
    }

    async fn stop(
        &self,
        runtime: &DaemonRuntime,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceStopResponse> {
        validate_managed_source_identity(&request.namespace, &request.source_key)?;
        self.stop_internal(runtime, &request.namespace, &request.source_key)
            .await?;
        Ok(ManagedSourceStopResponse {
            namespace: request.namespace.clone(),
            source_key: request.source_key.clone(),
            stopped: true,
        })
    }

    async fn delete(
        &self,
        runtime: &DaemonRuntime,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceDeleteResponse> {
        validate_managed_source_identity(&request.namespace, &request.source_key)?;
        let _ = self
            .stop_internal(runtime, &request.namespace, &request.source_key)
            .await;
        self.store
            .delete_source(&request.namespace, &request.source_key)
            .await?;
        self.entries
            .lock()
            .await
            .remove(&managed_source_identity_key(
                &request.namespace,
                &request.source_key,
            ));
        Ok(ManagedSourceDeleteResponse {
            namespace: request.namespace.clone(),
            source_key: request.source_key.clone(),
            deleted: true,
        })
    }

    async fn resume_all(&self, runtime: &DaemonRuntime) -> Result<()> {
        let records = self.store.load_sources().await?;
        for record in records {
            if record.status == "stopped" {
                continue;
            }
            if let Err(err) = self.ensure_tailer_for_record(runtime, record.clone()).await {
                tracing::warn!(
                    "failed to resume managed source {}/{}: {}",
                    record.namespace,
                    record.source_key,
                    err
                );
                self.store
                    .clear_source_job(
                        &record.namespace,
                        &record.source_key,
                        "failed",
                        now_unix_secs(),
                        Some(now_unix_secs()),
                        Some(err.to_string()),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn stream_read(
        &self,
        request: &ManagedStreamReadRequest,
    ) -> Result<ManagedStreamReadResponse> {
        let limit = request.limit.clamp(1, MANAGED_STREAM_EVENTS_MAX_LIMIT);
        let page = self
            .store
            .read_stream(&request.stream_id, request.after_offset, limit)
            .await?;
        Ok(ManagedStreamReadResponse {
            stream_id: request.stream_id.clone(),
            events: page.events.into_iter().map(stream_event_view).collect(),
            next_after_offset: page.next_after_offset,
            has_more: page.has_more,
        })
    }

    async fn stream_info(&self, stream_id: &str) -> Result<ManagedStreamInfo> {
        let info = self.store.stream_info(stream_id).await?.ok_or_else(|| {
            UxcError::OperationNotFound(format!("managed stream not found: {}", stream_id))
        })?;
        Ok(stream_info_view(&info))
    }

    async fn stream_trim(
        &self,
        request: &ManagedStreamTrimRequest,
    ) -> Result<ManagedStreamTrimResponse> {
        let trimmed = self
            .store
            .trim_stream_before(&request.stream_id, request.before_offset)
            .await?;
        Ok(ManagedStreamTrimResponse {
            stream_id: request.stream_id.clone(),
            trimmed,
        })
    }

    async fn start_managed_source(
        &self,
        runtime: &DaemonRuntime,
        mut record: ManagedSourceRecord,
        spec: ManagedSourceSpec,
    ) -> Result<ManagedSourceRecord> {
        let sink_path = runtime.managed_source_sink_path(&record.run_id);
        let subscription = managed_source_subscribe_request(&record, &spec, &sink_path);
        let started = runtime.subscriptions.start(runtime, &subscription).await?;
        record.status = started.status;
        record.updated_at_unix = now_unix_secs();
        record.started_at_unix = Some(now_unix_secs());
        record.stopped_at_unix = None;
        record.last_error = None;
        record.underlying_job_id = Some(started.job_id);
        record.mirrored_after_seq = 0;
        self.store.upsert_source(&record, true).await?;
        Ok(record)
    }

    async fn ensure_tailer_for_record(
        &self,
        runtime: &DaemonRuntime,
        record: ManagedSourceRecord,
    ) -> Result<()> {
        let Some(job_id) = record.underlying_job_id.clone() else {
            return Err(anyhow!(
                "managed source {}/{} has no underlying subscription job",
                record.namespace,
                record.source_key
            ));
        };
        let identity_key = managed_source_identity_key(&record.namespace, &record.source_key);
        if self.entries.lock().await.contains_key(&identity_key) {
            return Ok(());
        }

        let state = Arc::new(Mutex::new(view_from_record(&record)));
        let (stop_tx, stop_rx) = watch::channel(false);
        let entry = Arc::new(ManagedSourceEntry {
            namespace: record.namespace.clone(),
            source_key: record.source_key.clone(),
            stream_id: record.stream_id.clone(),
            state: state.clone(),
            underlying_job_id: Arc::new(Mutex::new(Some(job_id.clone()))),
            mirrored_after_seq: Arc::new(Mutex::new(record.mirrored_after_seq)),
            stop_tx: Mutex::new(stop_tx),
            tail_task: Mutex::new(None),
        });
        self.entries
            .lock()
            .await
            .insert(identity_key.clone(), entry.clone());
        let manager = self.clone();
        let runtime = runtime.clone();
        let tail_entry = entry.clone();
        let task = tokio::spawn(async move {
            manager
                .tail_managed_source(runtime, identity_key, tail_entry, job_id, stop_rx)
                .await;
        });
        *entry.tail_task.lock().await = Some(task);
        Ok(())
    }

    async fn tail_managed_source(
        &self,
        runtime: DaemonRuntime,
        identity_key: String,
        entry: Arc<ManagedSourceEntry>,
        job_id: String,
        stop_rx: watch::Receiver<bool>,
    ) {
        let mut after_seq = *entry.mirrored_after_seq.lock().await;
        loop {
            if *stop_rx.borrow() {
                break;
            }
            let result = runtime
                .subscriptions
                .events(&SubscriptionEventsRequest {
                    job_id: job_id.clone(),
                    after_seq,
                    limit: SUBSCRIPTION_EVENTS_MAX_LIMIT,
                    wait_ms: SUBSCRIPTION_EVENTS_MAX_WAIT_MS,
                })
                .await;
            let batch = match result {
                Ok(batch) => batch,
                Err(err) => {
                    let _ = self
                        .store
                        .clear_source_job(
                            &entry.namespace,
                            &entry.source_key,
                            "failed",
                            now_unix_secs(),
                            Some(now_unix_secs()),
                            Some(err.to_string()),
                        )
                        .await;
                    let mut state = entry.state.lock().await;
                    state.status = "failed".to_string();
                    state.updated_at_unix = now_unix_secs();
                    state.stopped_at_unix = Some(now_unix_secs());
                    state.last_error = Some(err.to_string());
                    break;
                }
            };

            for event in &batch.events {
                if let Some(payload) = event.data.as_ref() {
                    let _ = self
                        .store
                        .append_event(&entry.stream_id, event.timestamp_unix, payload)
                        .await;
                }
            }
            if batch.next_after_seq > after_seq {
                after_seq = batch.next_after_seq;
                *entry.mirrored_after_seq.lock().await = after_seq;
                let _ = self
                    .store
                    .update_source_runtime(
                        &entry.namespace,
                        &entry.source_key,
                        &batch.status,
                        now_unix_secs(),
                        Some(now_unix_secs()),
                        None,
                        None,
                        None,
                        Some(after_seq),
                    )
                    .await;
            }
            {
                let mut state = entry.state.lock().await;
                state.status = batch.status.clone();
                state.updated_at_unix = now_unix_secs();
                if batch.status != "running" {
                    state.stopped_at_unix = Some(now_unix_secs());
                }
            }
            if batch.status != "running" && batch.events.is_empty() {
                let _ = self
                    .store
                    .clear_source_job(
                        &entry.namespace,
                        &entry.source_key,
                        &batch.status,
                        now_unix_secs(),
                        Some(now_unix_secs()),
                        None,
                    )
                    .await;
                break;
            }
        }

        self.entries.lock().await.remove(&identity_key);
    }

    async fn stop_internal(
        &self,
        runtime: &DaemonRuntime,
        namespace: &str,
        source_key: &str,
    ) -> Result<()> {
        let identity_key = managed_source_identity_key(namespace, source_key);
        let stored = self
            .store
            .get_source(namespace, source_key)
            .await?
            .ok_or_else(|| {
                UxcError::OperationNotFound(format!(
                    "managed source not found: {}/{}",
                    namespace, source_key
                ))
            })?;
        let active_entry = { self.entries.lock().await.get(&identity_key).cloned() };
        if let Some(entry) = active_entry {
            let _ = entry.stop_tx.lock().await.send(true);
            if let Some(job_id) = entry.underlying_job_id.lock().await.clone() {
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    let _ = runtime.subscriptions.stop(&job_id).await;
                });
            }
            if let Some(task) = entry.tail_task.lock().await.take() {
                let task = task;
                task.abort();
                let _ = task.await;
            }
            self.entries.lock().await.remove(&identity_key);
        } else if let Some(job_id) = stored.underlying_job_id.clone() {
            let runtime = runtime.clone();
            tokio::spawn(async move {
                let _ = runtime.subscriptions.stop(&job_id).await;
            });
        }
        self.store
            .clear_source_job(
                namespace,
                source_key,
                "stopped",
                now_unix_secs(),
                Some(now_unix_secs()),
                None,
            )
            .await?;
        Ok(())
    }
}

fn validate_managed_source_identity(namespace: &str, source_key: &str) -> Result<()> {
    if namespace.trim().is_empty() {
        bail!("managed source namespace cannot be empty");
    }
    if source_key.trim().is_empty() {
        bail!("managed source source_key cannot be empty");
    }
    Ok(())
}

fn managed_source_identity_key(namespace: &str, source_key: &str) -> String {
    format!("{namespace}\u{0}{source_key}")
}

fn managed_source_streams_db_path(base_dir: &Path) -> PathBuf {
    base_dir.join("managed-source-streams.db")
}

fn managed_source_sink_path(base_dir: &Path, run_id: &str) -> PathBuf {
    base_dir
        .join("managed-source-sinks")
        .join(format!("{run_id}.ndjson"))
}

fn managed_stream_id(namespace: &str, source_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(source_key.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("stream_{}", &digest[..16])
}

fn new_managed_source_run_id() -> String {
    format!(
        "run_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn compute_managed_source_spec_key(spec: &ManagedSourceSpec) -> Result<String> {
    let payload = json!({
        "endpoint": spec.endpoint,
        "operation_id": spec.operation_id,
        "args": spec.args,
        "resource_uri": spec.resource_uri,
        "read_resource": spec.read_resource,
        "transport_hint": spec.transport_hint,
        "subprotocols": spec.subprotocols,
        "initial_text_frames": spec.initial_text_frames,
        "mode": spec.mode,
        "poll_config": spec.poll_config,
        "auth": spec.options.auth,
        "inject_env": spec
            .options
            .inject_env
            .iter()
            .map(|spec| json!({"name": spec.name, "template": spec.template}))
            .collect::<Vec<_>>(),
        "timeout_ms": spec.options.timeout_ms,
        "request_headers": spec.options.request_headers,
        "schema_url": spec.options.schema_url,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{:x}", digest))
}

fn managed_source_subscribe_request(
    record: &ManagedSourceRecord,
    spec: &ManagedSourceSpec,
    sink_path: &Path,
) -> SubscribeStartRequest {
    SubscribeStartRequest {
        request_id: format!("managed-source:{}:{}", record.namespace, record.source_key),
        endpoint: spec.endpoint.clone(),
        sink: format!("file:{}", sink_path.display()),
        operation_id: spec.operation_id.clone(),
        args: spec.args.clone(),
        resource_uri: spec.resource_uri.clone(),
        read_resource: spec.read_resource,
        transport_hint: spec.transport_hint.clone(),
        subprotocols: spec.subprotocols.clone(),
        initial_text_frames: spec.initial_text_frames.clone(),
        mode: spec.mode,
        poll_config: spec.poll_config.clone(),
        ephemeral: false,
        internal: true,
        options: spec.options.clone(),
    }
}

fn view_from_record(record: &ManagedSourceRecord) -> ManagedSourceView {
    ManagedSourceView {
        namespace: record.namespace.clone(),
        source_key: record.source_key.clone(),
        run_id: record.run_id.clone(),
        stream_id: record.stream_id.clone(),
        spec_key: record.spec_key.clone(),
        status: record.status.clone(),
        created_at_unix: record.created_at_unix,
        updated_at_unix: record.updated_at_unix,
        started_at_unix: record.started_at_unix,
        stopped_at_unix: record.stopped_at_unix,
        last_error: record.last_error.clone(),
    }
}

fn stream_event_view(record: StreamEventRecord) -> ManagedStreamEvent {
    ManagedStreamEvent {
        stream_id: record.stream_id,
        offset: record.offset,
        ingested_at_unix: record.ingested_at_unix,
        raw_payload: record.raw_payload,
    }
}

fn stream_info_view(record: &StreamInfoRecord) -> ManagedStreamInfo {
    ManagedStreamInfo {
        stream_id: record.stream_id.clone(),
        namespace: record.namespace.clone(),
        source_key: record.source_key.clone(),
        created_at_unix: record.created_at_unix,
        earliest_offset: record.earliest_offset,
        latest_offset: record.latest_offset,
        event_count: record.event_count,
        retention_max_rows: record.retention_max_rows,
        retention_max_age_secs: record.retention_max_age_secs,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperationSummary {
    operation_id: String,
    display_name: String,
    summary: Option<String>,
    required: Vec<String>,
    input_shape_hint: String,
    protocol_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Clone)]
pub struct DaemonRuntime {
    state: Arc<Mutex<ServerState>>,
    mcp: McpSessionManager,
    subscriptions: SubscriptionManager,
    managed_sources: ManagedSourceManager,
    managed_source_base_dir: PathBuf,
    should_stop: Arc<RwLock<bool>>,
    schema_mapping_lock: Arc<Mutex<()>>,
    logger: Option<DaemonLogger>,
}

impl DaemonRuntime {
    pub fn new() -> Self {
        Self::try_new().expect("daemon runtime should initialize")
    }

    pub fn try_new() -> Result<Self> {
        Self::try_new_with_subscription_store_path(subscription_store_path())
    }

    fn try_new_with_subscription_store_path(store_path: PathBuf) -> Result<Self> {
        let logger = Self::initialize_logger();
        let managed_source_base_dir = store_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(daemon_dir);
        let managed_source_store =
            ManagedSourceStore::new(managed_source_streams_db_path(&managed_source_base_dir))?;
        Ok(Self {
            state: Arc::new(Mutex::new(ServerState {
                started_at_unix: now_unix_secs(),
                request_count: 0,
            })),
            mcp: McpSessionManager::new(logger.clone()),
            subscriptions: SubscriptionManager::new(store_path)?,
            managed_sources: ManagedSourceManager::new(managed_source_store),
            managed_source_base_dir,
            should_stop: Arc::new(RwLock::new(false)),
            schema_mapping_lock: Arc::new(Mutex::new(())),
            logger,
        })
    }

    fn initialize_logger() -> Option<DaemonLogger> {
        let dir = daemon_dir();
        match DaemonLogger::new(&dir) {
            Ok(logger) => Some(logger),
            Err(e) => {
                tracing::warn!("Failed to initialize daemon logger: {}", e);
                None
            }
        }
    }

    async fn log(&self, entry: DaemonLogEntry) {
        if let Some(ref logger) = self.logger {
            if let Err(e) = logger.log(&entry).await {
                tracing::debug!("Failed to write daemon log: {}", e);
            }
        }
    }

    pub async fn invoke(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
        if request
            .options
            .schema_mapping_file
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        {
            let _invoke_guard = self.schema_mapping_lock.lock().await;
            let _schema_mapping_guard =
                SchemaMappingEnvGuard::new(request.options.schema_mapping_file.clone());
            return self.invoke_inner(request).await;
        }

        self.invoke_inner(request).await
    }

    async fn invoke_inner(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
        self.mcp.cleanup_idle().await;
        {
            let mut st = self.state.lock().await;
            st.request_count = st.request_count.saturating_add(1);
        }

        let start = Instant::now();

        // Log runtime invoke start
        self.log(
            DaemonLogEntry::new(DaemonEventType::RuntimeInvokeStart)
                .with_request_id(request.request_id.clone())
                .with_endpoint(request.endpoint.clone())
                .with_operation_id(request.operation_id.clone().unwrap_or_default()),
        )
        .await;

        let cache = self.build_cache(&request.options)?;
        let cache_for_fallback = cache.clone();
        let cache_for_mcp = cache.clone();
        let root_auth_profile =
            auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
        let detection_auth_profile = if request.options.schema_url.is_some() {
            None
        } else {
            root_auth_profile.clone()
        };
        let stdio_spawn_options = build_stdio_spawn_options(
            &request.endpoint,
            &request.options,
            root_auth_profile.as_ref(),
        )?;

        let detection_options = DetectionOptions {
            schema_url: request.options.schema_url.clone(),
            auth_profile: detection_auth_profile.clone(),
            stdio_spawn_options: stdio_spawn_options.clone(),
            request_timeout: request_timeout_duration(request.options.timeout_ms),
        };

        if let Some((kind, operation, data, reused)) = self
            .try_invoke_existing_mcp_execute_without_detection(
                &request,
                root_auth_profile.clone(),
                stdio_spawn_options.clone(),
                cache_for_mcp.clone(),
            )
            .await?
        {
            let duration_ms = start.elapsed().as_millis() as u64;
            if reused {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::DaemonSessionReused)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone()),
                )
                .await;
            }
            self.log(
                DaemonLogEntry::new(DaemonEventType::RuntimeInvokeSuccess)
                    .with_request_id(request.request_id.clone())
                    .with_endpoint(request.endpoint.clone())
                    .with_operation_id(request.operation_id.clone().unwrap_or_default())
                    .with_protocol("mcp".to_string())
                    .with_duration_ms(duration_ms),
            )
            .await;
            let mut response_meta = RuntimeMeta {
                schema_involved: Some(true),
                daemon_session_reused: Some(reused),
                ..Default::default()
            };
            let mut response_data = data;
            apply_runtime_artifact_compaction(&kind, &mut response_data, &mut response_meta)?;
            return Ok(RuntimeInvokeResponse {
                protocol: "mcp".to_string(),
                endpoint: request.endpoint,
                kind,
                operation,
                data: response_data,
                duration_ms: Some(duration_ms),
                meta: response_meta,
            });
        }

        let resolved = resolve_adapter_with_schema_cache(
            &request.endpoint,
            &detection_options,
            cache,
            detection_auth_profile.clone(),
            request.options.no_cache,
            request.options.refresh_schema,
        )
        .await;

        let mut resolved = match resolved {
            Ok(r) => r,
            Err(e) => {
                // Log protocol detection failure
                if let Some(uxc_err) = e.downcast_ref::<UxcError>() {
                    if matches!(
                        uxc_err,
                        UxcError::ProtocolDetectionFailed(_) | UxcError::UnsupportedProtocol(_)
                    ) {
                        self.log(
                            DaemonLogEntry::new(DaemonEventType::ProtocolDetectionFailure)
                                .with_request_id(request.request_id.clone())
                                .with_endpoint(request.endpoint.clone())
                                .with_error(e.to_string()),
                        )
                        .await;
                    }
                }
                return Err(e);
            }
        };

        let mut protocol = resolved.adapter.protocol_type().as_str().to_string();
        let execution_auth_profile = effective_runtime_auth_profile(
            &request,
            resolved.adapter.protocol_type(),
            root_auth_profile.clone(),
        )?;
        resolved.adapter =
            inject_auth_if_supported(resolved.adapter, execution_auth_profile.clone());
        resolved.adapter = inject_timeout_if_supported(
            resolved.adapter,
            request_timeout_duration(request.options.timeout_ms),
        );
        resolved.adapter = inject_request_headers_if_supported(
            resolved.adapter,
            request.options.request_headers.clone(),
        );
        let mut meta = RuntimeMeta::default();
        if let Some(cache_meta) = resolved.cache_meta {
            meta.schema_involved = Some(true);
            meta.cache_source = Some("schema_cache".to_string());
            meta.cache_age_ms = Some(cache_meta.age_ms);
            meta.cache_stale = Some(cache_meta.stale);
            meta.cache_fallback = Some(cache_meta.fallback);

            // Log cache events
            if cache_meta.fallback {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::CacheFallback)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone())
                        .with_protocol(protocol.clone()),
                )
                .await;
            } else if cache_meta.stale {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::CacheStale)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone())
                        .with_protocol(protocol.clone()),
                )
                .await;
            } else {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::CacheHit)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone())
                        .with_protocol(protocol.clone()),
                )
                .await;
            }
        } else if matches!(protocol.as_str(), "jsonrpc" | "grpc" | "mcp") {
            meta.schema_involved = Some(true);
        }

        let mut result: Result<(
            String,
            Option<String>,
            Value,
            Option<adapters::ExecutionMetadata>,
        )> = if protocol == "mcp" && matches!(request.action, RuntimeAction::Execute) {
            let prepared_args = prepare_runtime_execute_args(&resolved.adapter, &request).await?;
            // Clone the pre-computed stdio_spawn_options to avoid duplicate secret resolution
            let stdio_options = stdio_spawn_options.clone();
            let (kind, operation, data, reused) = self
                .invoke_mcp_execute(
                    &request,
                    prepared_args,
                    execution_auth_profile.clone(),
                    stdio_options,
                    cache_for_mcp.clone(),
                )
                .await?;
            meta.daemon_session_reused = Some(reused);

            if reused {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::DaemonSessionReused)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone()),
                )
                .await;
            }

            Ok((kind, operation, data, None))
        } else if protocol == "mcp" {
            if let Some(live_result) = invoke_live_stdio_mcp_help(
                self,
                &request,
                execution_auth_profile.as_ref(),
                cache_for_mcp.clone(),
            )
            .await?
            {
                Ok((live_result.0, live_result.1, live_result.2, None))
            } else {
                invoke_with_adapter(&resolved.adapter, &request).await
            }
        } else {
            invoke_with_adapter(&resolved.adapter, &request).await
        };

        // If invocation failed, attempt a stale-cache fallback even when protocol detection
        // succeeded without network access. This keeps `-h` flows resilient when the cached
        // schema is expired but still useful.
        if result.is_err() && !request.options.no_cache && !request.options.refresh_schema {
            if let cache::CacheLookup::Hit(hit) = cache_for_fallback
                .get_with_policy(&request.endpoint, cache::CacheReadPolicy::AllowStale)?
            {
                if hit.stale {
                    if let Some(fallback_protocol) = protocol_from_cached_schema(&hit.schema) {
                        // Refresh TTL so adapters using normal cache reads can consume this schema.
                        let _ = cache_for_fallback.put(&request.endpoint, &hit.schema);
                        let mut adapter =
                            adapter_from_protocol(fallback_protocol, &detection_options);
                        adapter = inject_cache_if_supported(adapter, cache_for_fallback.clone());
                        let fallback_auth_profile = effective_runtime_auth_profile(
                            &request,
                            fallback_protocol,
                            root_auth_profile.clone(),
                        )?;
                        adapter = inject_auth_if_supported(adapter, fallback_auth_profile.clone());
                        adapter = inject_timeout_if_supported(
                            adapter,
                            request_timeout_duration(request.options.timeout_ms),
                        );
                        adapter = inject_request_headers_if_supported(
                            adapter,
                            request.options.request_headers.clone(),
                        );
                        adapter =
                            inject_refresh_if_supported(adapter, request.options.refresh_schema);

                        protocol = adapter.protocol_type().as_str().to_string();
                        meta.schema_involved = Some(true);
                        meta.cache_source = Some("schema_cache".to_string());
                        meta.cache_age_ms = Some(cache_age_ms(hit.fetched_at));
                        meta.cache_stale = Some(true);
                        meta.cache_fallback = Some(true);

                        self.log(
                            DaemonLogEntry::new(DaemonEventType::CacheFallback)
                                .with_request_id(request.request_id.clone())
                                .with_endpoint(request.endpoint.clone())
                                .with_protocol(protocol.clone()),
                        )
                        .await;

                        result = if protocol == "mcp"
                            && matches!(request.action, RuntimeAction::Execute)
                        {
                            let prepared_args =
                                prepare_runtime_execute_args(&adapter, &request).await?;
                            // For cache fallback, recompute stdio_spawn_options since we don't have
                            // the original detection_options available
                            let (kind, operation, data, reused) = self
                                .invoke_mcp_execute(
                                    &request,
                                    prepared_args,
                                    fallback_auth_profile.clone(),
                                    None,
                                    cache_for_fallback.clone(),
                                )
                                .await?;
                            meta.daemon_session_reused = Some(reused);
                            Ok((kind, operation, data, None))
                        } else {
                            invoke_with_adapter(&adapter, &request).await
                        };
                    }
                }
            }
        }

        match result {
            Ok((kind, operation, mut data, execution_meta)) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                if let Some(execution_meta) = execution_meta {
                    meta.response_status_code = execution_meta.response_status_code;
                    if !execution_meta.response_headers.is_empty() {
                        meta.response_headers = Some(execution_meta.response_headers);
                    }
                }
                apply_runtime_artifact_compaction(&kind, &mut data, &mut meta)?;
                self.log(
                    DaemonLogEntry::new(DaemonEventType::RuntimeInvokeSuccess)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone())
                        .with_operation_id(request.operation_id.clone().unwrap_or_default())
                        .with_protocol(protocol.clone())
                        .with_duration_ms(duration_ms),
                )
                .await;

                Ok(RuntimeInvokeResponse {
                    protocol,
                    endpoint: request.endpoint,
                    kind,
                    operation,
                    data,
                    duration_ms: Some(duration_ms),
                    meta,
                })
            }
            Err(e) => {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::RuntimeInvokeFailure)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone())
                        .with_operation_id(request.operation_id.clone().unwrap_or_default())
                        .with_error(e.to_string()),
                )
                .await;
                Err(e)
            }
        }
    }

    pub async fn status(&self) -> DaemonStatus {
        let state = self.state.lock().await;
        let (stdio_sessions, http_sessions, reuse_hits) = self.mcp.status_counts().await;
        let log_file: Option<String> = self
            .logger
            .as_ref()
            .map(|l: &DaemonLogger| l.log_file_path().display().to_string());
        DaemonStatus {
            running: true,
            pid: Some(std::process::id()),
            socket: socket_path().display().to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            started_at_unix: Some(state.started_at_unix),
            request_count: state.request_count,
            mcp_stdio_sessions: stdio_sessions,
            mcp_http_sessions: http_sessions,
            mcp_reuse_hits: reuse_hits,
            log_file,
        }
    }

    pub async fn session_views(&self) -> Vec<DaemonSessionView> {
        self.mcp.session_views().await
    }

    pub async fn subscribe_start(
        &self,
        request: SubscribeStartRequest,
    ) -> Result<SubscribeStartResponse> {
        self.subscriptions.start(self, &request).await
    }

    pub async fn subscribe_list(&self) -> Vec<SubscriptionJobView> {
        self.subscriptions.list().await
    }

    pub async fn subscribe_status(&self, job_id: &str) -> Result<SubscriptionJobView> {
        self.subscriptions.status(job_id).await
    }

    pub async fn subscribe_stop(&self, job_id: &str) -> Result<SubscribeStopResponse> {
        self.subscriptions.stop(job_id).await
    }

    pub async fn subscribe_events(
        &self,
        request: &SubscriptionEventsRequest,
    ) -> Result<SubscriptionEventsResponse> {
        self.subscriptions.events(request).await
    }

    pub async fn resume_persisted_subscriptions(&self) -> Result<()> {
        self.subscriptions.resume_all(self).await
    }

    pub async fn source_ensure(
        &self,
        request: ManagedSourceEnsureRequest,
    ) -> Result<ManagedSourceEnsureResponse> {
        self.managed_sources.ensure(self, request).await
    }

    pub async fn source_status(
        &self,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceView> {
        self.managed_sources
            .status(&request.namespace, &request.source_key)
            .await
    }

    pub async fn source_stop(
        &self,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceStopResponse> {
        self.managed_sources.stop(self, request).await
    }

    pub async fn source_delete(
        &self,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceDeleteResponse> {
        self.managed_sources.delete(self, request).await
    }

    pub async fn stream_read(
        &self,
        request: &ManagedStreamReadRequest,
    ) -> Result<ManagedStreamReadResponse> {
        self.managed_sources.stream_read(request).await
    }

    pub async fn stream_info(&self, stream_id: &str) -> Result<ManagedStreamInfo> {
        self.managed_sources.stream_info(stream_id).await
    }

    pub async fn stream_trim(
        &self,
        request: &ManagedStreamTrimRequest,
    ) -> Result<ManagedStreamTrimResponse> {
        self.managed_sources.stream_trim(request).await
    }

    pub async fn resume_managed_sources(&self) -> Result<()> {
        self.managed_sources.resume_all(self).await
    }

    fn managed_source_sink_path(&self, run_id: &str) -> PathBuf {
        managed_source_sink_path(&self.managed_source_base_dir, run_id)
    }

    async fn detect_poll_subscription_protocol(
        &self,
        request: &SubscribeStartRequest,
    ) -> Result<String> {
        let operation_id = request
            .operation_id
            .as_deref()
            .ok_or_else(|| anyhow!("poll subscriptions require an operation_id"))?;
        if request.resource_uri.is_some() {
            bail!("poll subscriptions do not support --resource-uri");
        }
        if request.transport_hint.is_some() {
            bail!("poll subscriptions do not support transport hints");
        }
        if operation_id.starts_with("subscription/") {
            bail!("poll subscriptions do not support GraphQL subscription/<field> operations");
        }

        let cache = self.build_cache(&request.options)?;
        let root_auth_profile =
            auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
        let stdio_spawn_options = build_stdio_spawn_options(
            &request.endpoint,
            &request.options,
            root_auth_profile.as_ref(),
        )?;
        let detection_options = DetectionOptions {
            schema_url: request.options.schema_url.clone(),
            auth_profile: root_auth_profile.clone(),
            stdio_spawn_options,
            request_timeout: request_timeout_duration(request.options.timeout_ms),
        };
        let resolved = resolve_adapter_with_schema_cache(
            &request.endpoint,
            &detection_options,
            cache,
            root_auth_profile,
            request.options.no_cache,
            request.options.refresh_schema,
        )
        .await?;

        Ok(resolved.adapter.protocol_type().as_str().to_string())
    }

    pub async fn request_stop(&self) {
        let mut stop = self.should_stop.write().await;
        *stop = true;
        // Nudge the accept loop to exit promptly.
        #[cfg(unix)]
        {
            let _ = UnixStream::connect(socket_path()).await;
        }
    }

    pub async fn should_stop(&self) -> bool {
        *self.should_stop.read().await
    }

    fn build_cache(&self, options: &RuntimeInvokeOptions) -> Result<Arc<dyn Cache>> {
        let cfg = if options.no_cache {
            CacheConfig {
                enabled: false,
                ..Default::default()
            }
        } else if let Some(ttl) = options.cache_ttl {
            CacheConfig {
                ttl,
                ..Default::default()
            }
        } else {
            CacheConfig::load_from_file().unwrap_or_default()
        };
        cache::create_cache(cfg)
    }

    async fn try_invoke_existing_mcp_execute_without_detection(
        &self,
        request: &RuntimeInvokeRequest,
        auth_profile: Option<Profile>,
        precomputed_stdio_spawn_options: Option<adapters::mcp::StdioSpawnOptions>,
        cache: Arc<dyn Cache>,
    ) -> Result<Option<(String, Option<String>, Value, bool)>> {
        if !matches!(request.action, RuntimeAction::Execute) {
            return Ok(None);
        }

        let has_live_session = if adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint) {
            let key = stdio_session_key(
                &request.endpoint,
                auth_profile.as_ref(),
                &request.options.inject_env,
            )?;
            self.mcp.get_stdio(&key).await.is_some()
        } else if adapters::mcp::McpAdapter::is_http_url(&request.endpoint) {
            let lookup_key = http_session_lookup_key(&request.endpoint, auth_profile.as_ref());
            self.mcp.has_http_by_lookup_key(&lookup_key).await
        } else {
            false
        };

        if !has_live_session {
            return Ok(None);
        }

        // Preserve daemon-managed MCP session semantics for endpoints whose discovery surface is
        // bound to the existing session. We intentionally forward the already-parsed CLI args.
        let result = self
            .invoke_mcp_execute(
                request,
                request.args.clone().unwrap_or_default(),
                auth_profile,
                precomputed_stdio_spawn_options,
                cache,
            )
            .await?;
        Ok(Some(result))
    }

    async fn invoke_mcp_execute(
        &self,
        request: &RuntimeInvokeRequest,
        args: HashMap<String, Value>,
        auth_profile: Option<Profile>,
        precomputed_stdio_spawn_options: Option<adapters::mcp::StdioSpawnOptions>,
        cache: Arc<dyn Cache>,
    ) -> Result<(String, Option<String>, Value, bool)> {
        let endpoint = &request.endpoint;
        let op = request
            .operation_id
            .as_ref()
            .ok_or_else(|| anyhow!("operation_id is required"))?;
        let arguments = Some(Value::Object(args.into_iter().collect()));

        if adapters::mcp::McpAdapter::is_stdio_command(endpoint) {
            let (cmd, cmd_args) = adapters::mcp::McpAdapter::parse_stdio_command(endpoint)?;
            // Use pre-computed spawn options if available (from detection phase),
            // otherwise compute them now. This avoids duplicate secret resolution.
            let spawn_options = match precomputed_stdio_spawn_options {
                Some(options) => options,
                None => {
                    build_stdio_spawn_options(endpoint, &request.options, auth_profile.as_ref())?
                        .unwrap_or_default()
                }
            };
            let key =
                stdio_session_key(endpoint, auth_profile.as_ref(), &request.options.inject_env)?;
            let (session, reused) = self
                .mcp
                .get_or_create_stdio(
                    &key,
                    &cmd,
                    &cmd_args,
                    &spawn_options,
                    request_timeout_duration(request.options.timeout_ms),
                    StdioSessionRequestMetadata {
                        idle_ttl_secs: request.options.daemon_idle_ttl,
                        link_name: request.options.link_name.as_deref(),
                        link_skill: request.options.link_skill.as_deref(),
                        link_skill_doc: request.options.link_skill_doc.as_deref(),
                        link_skill_path: request.options.link_skill_path.as_deref(),
                        endpoint,
                        exclusive_keys: &request.options.daemon_exclusive,
                    },
                )
                .await?;
            self.mcp.mark_stdio_request_started(&key).await;
            let mut guard = session.lock().await;
            guard.last_used = Instant::now();
            guard.last_used_at_unix = now_unix_secs();
            let result = guard
                .client
                .call_tool_with_timeout(
                    op,
                    arguments,
                    request_timeout_duration(request.options.timeout_ms).unwrap_or_else(
                        adapters::mcp::transport::McpStdioTransport::default_request_timeout,
                    ),
                )
                .await;
            let _ = guard
                .mark_tools_dirty_from_notifications(endpoint, &cache)
                .await;
            let recent_stderr = guard.client.recent_stderr_lines(5).await;
            let child_exited = guard.client.child_has_exited().unwrap_or(false);
            drop(guard);
            match result {
                Ok(result) => {
                    self.mcp
                        .mark_stdio_request_finished(&key, None, recent_stderr, child_exited)
                        .await;
                    Ok((
                        "call_result".to_string(),
                        Some(op.clone()),
                        adapters::mcp::convert_tool_result_to_value(&result),
                        reused,
                    ))
                }
                Err(err) => {
                    self.mcp
                        .mark_stdio_request_finished(&key, Some(&err), recent_stderr, child_exited)
                        .await;
                    Err(err)
                }
            }
        } else {
            let lookup_key = http_session_lookup_key(endpoint, auth_profile.as_ref());
            let (session, reused) = if let Some((session, reused)) =
                self.mcp.get_http_by_lookup_key(&lookup_key).await
            {
                (session, reused)
            } else {
                let resolved_transport =
                    resolve_mcp_http_endpoint(endpoint, auth_profile.clone()).await?;
                let key = format!(
                    "http:{:?}:{}:{}",
                    resolved_transport.mode,
                    resolved_transport.connect_url,
                    auth_fingerprint(auth_profile.as_ref())
                );
                self.mcp
                    .get_or_create_http(
                        &lookup_key,
                        &key,
                        &resolved_transport,
                        auth_profile,
                        request_timeout_duration(request.options.timeout_ms),
                    )
                    .await?
            };
            session.mark_used().await;
            let result = session.transport.call_tool(op, arguments).await?;
            let _ = session.collect_pending_notifications().await;
            Ok((
                "call_result".to_string(),
                Some(op.clone()),
                adapters::mcp::convert_tool_result_to_value(&result),
                reused,
            ))
        }
    }
}

enum SubscriptionRunError {
    Retry(anyhow::Error),
    Fatal(anyhow::Error),
}

fn parse_file_sink(spec: &str) -> Result<PathBuf> {
    let Some(path) = spec.strip_prefix("file:") else {
        bail!("subscribe sink must use file:<path> or memory:");
    };
    if path.trim().is_empty() {
        bail!("subscribe sink path cannot be empty");
    }
    let path = PathBuf::from(path);
    validate_subscription_sink_path(&path)?;
    Ok(path)
}

fn internal_memory_sink_path(job_id: &str) -> PathBuf {
    daemon_dir()
        .join("subscription-events")
        .join(format!("{}.ndjson", job_id))
}

fn sink_is_memory(spec: &str) -> bool {
    spec == "memory:"
}

fn validate_subscription_sink_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("subscribe sink path cannot be empty");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("subscribe sink path cannot contain '..'");
    }
    Ok(())
}

fn is_nonstandard_subscription_sink_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if let Some(home) = resolve_home_dir_for_tilde() {
        if path.starts_with(&home) {
            return false;
        }
    }
    let temp_dir = std::env::temp_dir();
    !path.starts_with(&temp_dir)
}

async fn open_subscription_sink(path: &Path) -> Result<tokio::fs::File> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create sink directory {}", parent.display()))?;
    }
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("Failed to open sink file {}", path.display()))
}

struct LoadedSubscriptionEvents {
    events: Vec<SubscriptionEventEnvelope>,
    next_after_seq: u64,
    has_more: bool,
}

async fn load_subscription_events(
    path: &Path,
    after_seq: u64,
    limit: usize,
    retain_all: bool,
) -> Result<LoadedSubscriptionEvents> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("Failed to read subscription sink {}", path.display()))
        }
    };
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event =
            serde_json::from_str::<SubscriptionEventEnvelope>(trimmed).with_context(|| {
                format!(
                    "Invalid subscription event in {} at line {}",
                    path.display(),
                    index + 1
                )
            })?;
        events.push(event);
    }

    let retained = if retain_all {
        events.as_slice()
    } else {
        let retained_start = events
            .len()
            .saturating_sub(SUBSCRIPTION_EVENT_HISTORY_LIMIT);
        &events[retained_start..]
    };
    if let Some(first) = retained.first() {
        if after_seq > 0 && after_seq < first.seq.saturating_sub(1) {
            return Err(UxcError::InvalidArguments(format!(
                "subscription cursor expired for job {}: earliest available seq is {}",
                first.job_id, first.seq
            ))
            .into());
        }
    }

    let filtered = retained
        .iter()
        .filter(|event| event.seq > after_seq)
        .cloned()
        .collect::<Vec<_>>();
    let has_more = filtered.len() > limit;
    let limited = filtered.into_iter().take(limit).collect::<Vec<_>>();
    let next_after_seq = limited.last().map(|event| event.seq).unwrap_or(after_seq);

    Ok(LoadedSubscriptionEvents {
        events: limited,
        next_after_seq,
        has_more,
    })
}

async fn update_subscription_view(
    view: &Arc<Mutex<SubscriptionJobView>>,
    status: Option<&str>,
    last_error: Option<String>,
    increment_reconnect: bool,
) {
    let mut guard = view.lock().await;
    if let Some(status) = status {
        guard.status = status.to_string();
    }
    if increment_reconnect {
        guard.reconnect_count = guard.reconnect_count.saturating_add(1);
    }
    if let Some(last_error) = last_error {
        guard.last_error = Some(last_error);
    } else if status == Some("running") {
        guard.last_error = None;
    }
}

async fn append_subscription_event(
    sink: &mut tokio::fs::File,
    view: &Arc<Mutex<SubscriptionJobView>>,
    seq: &mut u64,
    source_kind: &str,
    event_kind: &str,
    data: Option<Value>,
    meta: Option<Value>,
) -> Result<()> {
    let next_seq = seq.saturating_add(1);
    let snapshot = view.lock().await.clone();
    let record = SubscriptionEventEnvelope {
        version: "v1".to_string(),
        job_id: snapshot.job_id.clone(),
        seq: next_seq,
        timestamp_unix: now_unix_secs(),
        protocol: snapshot.protocol,
        source_kind: source_kind.to_string(),
        event_kind: event_kind.to_string(),
        data,
        meta,
    };
    let mut line = serde_json::to_vec(&record)?;
    line.push(b'\n');
    sink.write_all(&line).await?;
    sink.flush().await?;

    *seq = next_seq;
    let mut guard = view.lock().await;
    guard.written_events = guard.written_events.saturating_add(1);
    guard.last_event_at_unix = Some(now_unix_secs());
    Ok(())
}

fn ensure_subscription_buffer_limit(len: usize, kind: &str) -> Result<()> {
    if len > SUBSCRIPTION_MAX_BUFFER_BYTES {
        bail!(
            "{} buffer exceeded {} bytes without a complete event",
            kind,
            SUBSCRIPTION_MAX_BUFFER_BYTES
        );
    }
    Ok(())
}

fn sse_delimiter(input: &str) -> Option<(usize, usize)> {
    if let Some(pos) = input.find("\r\n\r\n") {
        return Some((pos, 4));
    }
    input.find("\n\n").map(|pos| (pos, 2))
}

fn drain_sse_json_events(buffer: &mut String) -> Result<Vec<Value>> {
    let mut events = Vec::new();
    while let Some((pos, delim_len)) = sse_delimiter(buffer) {
        let chunk = buffer[..pos].to_string();
        buffer.drain(..pos + delim_len);
        let mut data_lines = Vec::new();
        for raw in chunk.lines() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                data_lines.push(rest.trim_start().to_string());
            }
        }
        if data_lines.is_empty() {
            continue;
        }
        let payload = data_lines.join("\n");
        if payload == "[DONE]" {
            continue;
        }
        let value = serde_json::from_str::<Value>(&payload)
            .with_context(|| format!("SSE event data is not valid JSON: {}", payload))?;
        events.push(value);
    }
    ensure_subscription_buffer_limit(buffer.len(), "sse")?;
    Ok(events)
}

fn drain_ndjson_events(buffer: &mut String) -> Result<Vec<Value>> {
    let mut events = Vec::new();
    while let Some(pos) = buffer.find('\n') {
        let line = buffer[..pos].trim_end_matches('\r').trim().to_string();
        buffer.drain(..=pos);
        if line.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(&line)
            .with_context(|| format!("stream line is not valid JSON: {}", line))?;
        events.push(value);
    }
    ensure_subscription_buffer_limit(buffer.len(), "ndjson")?;
    Ok(events)
}

fn decode_utf8_prefix(buffer: &mut Vec<u8>) -> Result<Option<String>> {
    match std::str::from_utf8(buffer) {
        Ok(decoded) => {
            if decoded.is_empty() {
                return Ok(None);
            }
            let text = decoded.to_string();
            buffer.clear();
            Ok(Some(text))
        }
        Err(err) => {
            if err.error_len().is_some() {
                bail!("stream chunk contains invalid utf-8");
            }
            let valid_up_to = err.valid_up_to();
            if valid_up_to == 0 {
                ensure_subscription_buffer_limit(buffer.len(), "utf8")?;
                return Ok(None);
            }
            let text = std::str::from_utf8(&buffer[..valid_up_to])?.to_string();
            buffer.drain(..valid_up_to);
            Ok(Some(text))
        }
    }
}

async fn subscription_stop_requested(stop_rx: &mut watch::Receiver<bool>) -> bool {
    if *stop_rx.borrow() {
        return true;
    }
    matches!(stop_rx.changed().await, Ok(())) && *stop_rx.borrow()
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

async fn close_subscription_as_stopped(
    sink: &mut tokio::fs::File,
    view: &Arc<Mutex<SubscriptionJobView>>,
    seq: &mut u64,
    source_kind: &str,
) -> Result<()> {
    append_subscription_event(
        sink,
        view,
        seq,
        source_kind,
        "closed",
        None,
        Some(json!({"reason":"stopped"})),
    )
    .await?;
    update_subscription_view(view, Some("stopped"), None, false).await;
    Ok(())
}

async fn execute_http_stream_once(
    request: &SubscribeStartRequest,
    view: &Arc<Mutex<SubscriptionJobView>>,
    sink: &mut tokio::fs::File,
    seq: &mut u64,
    stop_rx: &mut watch::Receiver<bool>,
) -> std::result::Result<(), SubscriptionRunError> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())
            .map_err(SubscriptionRunError::Fatal)?;
    let resolved_auth = auth_profile
        .as_ref()
        .map(|profile| {
            auth::resolve_profile_request_auth_with_context(
                &auth::AuthRequestContext::new("GET", &request.endpoint),
                profile,
            )
        })
        .transpose()
        .map_err(SubscriptionRunError::Fatal)?;
    let client = crate::http_client::build_resilient_http_client(
        Duration::from_secs(SUBSCRIPTION_HTTP_TIMEOUT_SECS),
        "subscription http stream",
    )
    .map_err(SubscriptionRunError::Retry)?;
    let target_url = resolved_auth
        .as_ref()
        .map(|resolved| resolved.url.as_str())
        .unwrap_or(request.endpoint.as_str());
    let mut req = client.get(target_url).header(
        reqwest::header::ACCEPT,
        "application/json, application/x-ndjson, text/event-stream",
    );
    if let Some(resolved) = resolved_auth.as_ref() {
        for (name, value) in &resolved.headers {
            req = req.header(name, value);
        }
    }
    if *stop_rx.borrow() {
        close_subscription_as_stopped(sink, view, seq, "http")
            .await
            .map_err(SubscriptionRunError::Fatal)?;
        return Ok(());
    }
    let response = tokio::select! {
        changed = stop_rx.changed() => {
            if changed.is_ok() && *stop_rx.borrow() {
                close_subscription_as_stopped(sink, view, seq, "http").await
                    .map_err(SubscriptionRunError::Fatal)?;
                return Ok(());
            }
            return Err(SubscriptionRunError::Retry(anyhow!("subscription stop channel closed unexpectedly")));
        }
        result = req.send() => {
            result.map_err(|err| SubscriptionRunError::Retry(anyhow!(err)))?
        }
    };
    if !response.status().is_success() {
        return Err(SubscriptionRunError::Retry(anyhow!(
            "HTTP subscribe request failed with status {}",
            response.status()
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let source_kind = if content_type.contains("text/event-stream") {
        "http_sse"
    } else {
        "http_ndjson"
    };

    append_subscription_event(
        sink,
        view,
        seq,
        source_kind,
        "open",
        None,
        Some(json!({ "content_type": content_type, "url": redact_endpoint(target_url) })),
    )
    .await
    .map_err(SubscriptionRunError::Fatal)?;
    update_subscription_view(view, Some("running"), None, false).await;

    let mut stream = response.bytes_stream();
    let mut raw_buffer = Vec::new();
    let mut text_buffer = String::new();
    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(sink, view, seq, source_kind)
                .await
                .map_err(SubscriptionRunError::Fatal)?;
            return Ok(());
        }
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_ok() && *stop_rx.borrow() {
                    close_subscription_as_stopped(sink, view, seq, source_kind).await
                        .map_err(SubscriptionRunError::Fatal)?;
                    return Ok(());
                }
            }
            item = stream.next() => {
                match item {
                    Some(Ok(bytes)) => {
                        raw_buffer.extend_from_slice(&bytes);
                        ensure_subscription_buffer_limit(raw_buffer.len(), "http stream")
                            .map_err(SubscriptionRunError::Fatal)?;
                        if let Some(chunk) = decode_utf8_prefix(&mut raw_buffer).map_err(SubscriptionRunError::Fatal)? {
                            text_buffer.push_str(&chunk);
                        }
                        let values = if source_kind == "http_sse" {
                            drain_sse_json_events(&mut text_buffer).map_err(SubscriptionRunError::Fatal)?
                        } else {
                            drain_ndjson_events(&mut text_buffer).map_err(SubscriptionRunError::Fatal)?
                        };
                        for value in values {
                            append_subscription_event(
                                sink,
                                view,
                                seq,
                                source_kind,
                                "data",
                                Some(value),
                                None,
                            ).await.map_err(SubscriptionRunError::Fatal)?;
                        }
                    }
                    Some(Err(err)) => {
                        return Err(SubscriptionRunError::Retry(anyhow!("stream read failed: {}", err)));
                    }
                    None => {
                        return Err(SubscriptionRunError::Retry(anyhow!("stream closed by remote peer")));
                    }
                }
            }
        }
    }
}

async fn run_http_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut sink, &view, &mut seq, "http").await?;
            return Ok(());
        }
        match execute_http_stream_once(request, &view, &mut sink, &mut seq, &mut stop_rx).await {
            Ok(()) => return Ok(()),
            Err(SubscriptionRunError::Fatal(err)) => return Err(err),
            Err(SubscriptionRunError::Retry(err)) => {
                let msg = err.to_string();
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "http",
                    "error",
                    None,
                    Some(json!({ "message": msg })),
                )
                .await?;
                update_subscription_view(&view, Some("reconnecting"), Some(msg.clone()), true)
                    .await;
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "http",
                    "reconnect",
                    None,
                    Some(json!({ "delay_secs": delay_secs })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "http").await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

struct DaemonWebSocketObserver<'a> {
    sink: &'a mut tokio::fs::File,
    view: &'a Arc<Mutex<SubscriptionJobView>>,
    seq: &'a mut u64,
    source_kind: &'a str,
}

#[async_trait::async_trait]
impl WebSocketRuntimeObserver for DaemonWebSocketObserver<'_> {
    async fn emit(
        &mut self,
        event_kind: &str,
        data: Option<Value>,
        meta: Option<Value>,
    ) -> Result<()> {
        append_subscription_event(
            self.sink,
            self.view,
            self.seq,
            self.source_kind,
            event_kind,
            data,
            meta,
        )
        .await
    }

    async fn update_status(
        &mut self,
        status: Option<&str>,
        last_error: Option<String>,
        increment_reconnect: bool,
    ) -> Result<()> {
        update_subscription_view(self.view, status, last_error, increment_reconnect).await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl PollRuntimeObserver for DaemonWebSocketObserver<'_> {
    async fn emit(
        &mut self,
        event_kind: &str,
        data: Option<Value>,
        meta: Option<Value>,
    ) -> Result<()> {
        append_subscription_event(
            self.sink,
            self.view,
            self.seq,
            self.source_kind,
            event_kind,
            data,
            meta,
        )
        .await
    }

    async fn update_status(
        &mut self,
        status: Option<&str>,
        last_error: Option<String>,
        increment_reconnect: bool,
    ) -> Result<()> {
        update_subscription_view(self.view, status, last_error, increment_reconnect).await;
        Ok(())
    }
}

struct GraphQLFallbackObserver<'a, O> {
    inner: &'a mut O,
    saw_data: bool,
}

#[async_trait::async_trait]
impl<O: WebSocketRuntimeObserver + Send> WebSocketRuntimeObserver
    for GraphQLFallbackObserver<'_, O>
{
    async fn emit(
        &mut self,
        event_kind: &str,
        data: Option<Value>,
        meta: Option<Value>,
    ) -> Result<()> {
        if event_kind == "data" {
            self.saw_data = true;
        }
        self.inner.emit(event_kind, data, meta).await
    }

    async fn update_status(
        &mut self,
        status: Option<&str>,
        last_error: Option<String>,
        increment_reconnect: bool,
    ) -> Result<()> {
        self.inner
            .update_status(status, last_error, increment_reconnect)
            .await
    }
}

fn graphql_retry_supports_fallback(profile: GraphQLWebSocketProfile, err: &anyhow::Error) -> bool {
    if profile != GraphQLWebSocketProfile::Modern {
        return false;
    }
    let message = err.to_string();
    message.contains("websocket connection failed")
        || message.contains("tls handshake eof")
        || message.contains("http error")
        || message.contains("handshake not finished")
        || message.contains("websocket closed by remote peer")
        || message.contains("websocket stream ended")
        || message.contains("protocol error")
}

async fn run_stream_subscription_job(
    runtime: &DaemonRuntime,
    job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::Websocket)
    ) {
        return run_websocket_subscription_job(job_id, request, sink_path, view, stop_rx).await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::DiscordGateway)
    ) {
        return run_discord_gateway_subscription_job(job_id, request, sink_path, view, stop_rx)
            .await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::SlackSocketMode)
    ) {
        return run_slack_socket_mode_subscription_job(job_id, request, sink_path, view, stop_rx)
            .await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::FeishuLongConnection)
    ) {
        return run_feishu_long_connection_subscription_job(
            job_id, request, sink_path, view, stop_rx,
        )
        .await;
    }

    if request.operation_id.is_some() {
        if request
            .operation_id
            .as_deref()
            .is_some_and(|operation_id| operation_id.starts_with("subscription/"))
        {
            return run_graphql_subscription_job(job_id, request, sink_path, view, stop_rx).await;
        }
        return run_jsonrpc_subscription_job(job_id, request, sink_path, view, stop_rx).await;
    }

    if request.resource_uri.is_some() {
        return run_mcp_subscription_job(runtime, job_id, request, sink_path, view, stop_rx).await;
    }

    run_http_subscription_job(job_id, request, sink_path, view, stop_rx).await
}

async fn run_websocket_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut handler = RawFrameHandler;
    let mut observer = DaemonWebSocketObserver {
        sink: &mut sink,
        view: &view,
        seq: &mut seq,
        source_kind: "websocket",
    };

    subscription_websocket::run_websocket_subscription_runtime(
        WebSocketRuntimeConfig {
            endpoint: request.endpoint.clone(),
            auth_profile,
            subprotocols: request.subprotocols.clone(),
            initial_text_frames: request.initial_text_frames.clone(),
            first_message_timeout_secs: None,
            initial_reconnect_delay_secs: SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS,
            max_reconnect_delay_secs: SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS,
        },
        &mut handler,
        &mut observer,
        &mut stop_rx,
    )
    .await
}

async fn run_discord_gateway_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let resolved = resolve_discord_gateway_runtime_config(request)?;
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
    let mut handler = DiscordGatewayHandler::new(resolved.session);

    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut sink, &view, &mut seq, "discord_gateway").await?;
            return Ok(());
        }

        let websocket_url = if let Some(url) = handler.preferred_gateway_websocket_url() {
            url
        } else {
            match open_discord_gateway_websocket_url(request, &resolved.auth_profile).await {
                Ok((url, _open_meta)) => url,
                Err(err) => {
                    let message = err.to_string();
                    append_subscription_event(
                        &mut sink,
                        &view,
                        &mut seq,
                        "discord_gateway",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                    update_subscription_view(&view, Some("reconnecting"), Some(message), true)
                        .await;
                    append_subscription_event(
                        &mut sink,
                        &view,
                        &mut seq,
                        "discord_gateway",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs, "phase": "gateway_open" })),
                    )
                    .await?;
                    if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await
                    {
                        close_subscription_as_stopped(
                            &mut sink,
                            &view,
                            &mut seq,
                            "discord_gateway",
                        )
                        .await?;
                        return Ok(());
                    }
                    delay_secs =
                        (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
                    continue;
                }
            }
        };

        let config = WebSocketRuntimeConfig {
            endpoint: websocket_url,
            auth_profile: None,
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
            first_message_timeout_secs: Some(10),
            initial_reconnect_delay_secs: SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS,
            max_reconnect_delay_secs: SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS,
        };

        let result = {
            let mut observer = DaemonWebSocketObserver {
                sink: &mut sink,
                view: &view,
                seq: &mut seq,
                source_kind: "discord_gateway",
            };
            subscription_websocket::run_websocket_subscription_session_once(
                &config,
                &mut handler,
                &mut observer,
                &mut stop_rx,
            )
            .await
        };

        match result {
            Ok(()) => return Ok(()),
            Err(WebSocketRunError::Fatal(err)) => {
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "discord_gateway",
                    "error",
                    None,
                    Some(json!({ "message": err.to_string() })),
                )
                .await?;
                return Err(err);
            }
            Err(WebSocketRunError::Retry(err)) => {
                let message = err.to_string();
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "discord_gateway",
                    "error",
                    None,
                    Some(json!({ "message": message })),
                )
                .await?;
                update_subscription_view(&view, Some("reconnecting"), Some(err.to_string()), true)
                    .await;
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "discord_gateway",
                    "reconnect",
                    None,
                    Some(json!({ "delay_secs": delay_secs })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "discord_gateway")
                        .await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

async fn open_slack_socket_mode_websocket_url(request: &SubscribeStartRequest) -> Result<String> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?
            .ok_or_else(|| {
                anyhow!(
                    "Slack Socket Mode requires an auth credential with an app-level xapp token"
                )
            })?;
    let open_endpoint = derive_socket_mode_open_endpoint(&request.endpoint)?;
    let resolved = auth::resolve_profile_operation_request_auth(
        &auth::AuthRequestContext::new("POST", &open_endpoint),
        &auth_profile,
    )?;
    let client = crate::http_client::build_resilient_http_client(
        Duration::from_secs(10),
        "Slack Socket Mode open",
    )?;
    let mut req = client.post(&resolved.url);
    for (name, value) in resolved.headers {
        req = req.header(name, value);
    }
    let response = req.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read response body>".to_string());
        let body = truncate_error_body(&body, 512);
        bail!(
            "Slack Socket Mode open request failed with status {}: {}",
            status,
            body
        );
    }
    let body = response.json::<Value>().await?;
    let parsed = parse_socket_mode_open_response(&body)?;
    Ok(parsed.websocket_url)
}

async fn open_feishu_long_connection_websocket_url(
    request: &SubscribeStartRequest,
) -> Result<FeishuLongConnectionOpenResponse> {
    let auth_profile = auth::resolve_auth_for_endpoint(
        &request.endpoint,
        request.options.auth.clone(),
    )?
    .ok_or_else(|| {
        anyhow!("Feishu long-connection requires an auth credential with app_id/app_secret fields")
    })?;
    let runtime_config = resolve_feishu_long_connection_runtime_config(&auth_profile)?;
    let open_endpoint = derive_feishu_ws_config_endpoint(&request.endpoint)?;
    let client = crate::http_client::build_resilient_http_client(
        Duration::from_secs(10),
        "Feishu long-connection open",
    )?;
    let response = client
        .post(&open_endpoint)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("locale", "zh")
        .json(&json!({
            "AppID": runtime_config.app_id,
            "AppSecret": runtime_config.app_secret,
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read response body>".to_string());
        let body = truncate_error_body(&body, 512);
        bail!(
            "Feishu long-connection open request failed with status {}: {}",
            status,
            body
        );
    }
    let body = response.json::<Value>().await?;
    parse_feishu_long_connection_open_response(&body)
}

fn truncate_error_body(body: &str, max_chars: usize) -> String {
    let mut chars = body.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

struct DiscordGatewayResolvedConfig {
    auth_profile: Profile,
    session: DiscordGatewayRuntimeConfig,
}

fn resolve_discord_gateway_runtime_config(
    request: &SubscribeStartRequest,
) -> Result<DiscordGatewayResolvedConfig> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?
            .ok_or_else(|| {
                anyhow!("Discord Gateway requires an auth credential with a bot token")
            })?;
    let token = auth_profile
        .resolve_secret()?
        .ok_or_else(|| anyhow!("Discord Gateway requires a credential with a bot token secret"))?;

    let args = request.args.as_ref().cloned().unwrap_or_default();
    let allowed: std::collections::HashSet<&str> =
        ["intents", "os", "browser", "device"].into_iter().collect();
    if let Some(unexpected) = args.keys().find(|key| !allowed.contains(key.as_str())) {
        bail!(
            "discord-gateway transport does not support argument '{}' (allowed: intents, os, browser, device)",
            unexpected
        );
    }

    let intents = match args.get("intents") {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| anyhow!("discord-gateway intents must be a non-negative integer"))?,
        Some(Value::String(raw)) => raw
            .parse::<u64>()
            .with_context(|| format!("invalid discord-gateway intents '{}'", raw))?,
        Some(other) => bail!(
            "discord-gateway intents must be a number or string, got {}",
            other
        ),
        None => DISCORD_DEFAULT_MESSAGE_INTENTS,
    };

    let read_string = |name: &str| -> Result<Option<String>> {
        match args.get(name) {
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(other) => bail!("discord-gateway '{}' must be a string, got {}", name, other),
            None => Ok(None),
        }
    };

    let mut identify_properties = DiscordIdentifyProperties::default();
    if let Some(value) = read_string("os")? {
        identify_properties.os = value;
    }
    if let Some(value) = read_string("browser")? {
        identify_properties.browser = value;
    }
    if let Some(value) = read_string("device")? {
        identify_properties.device = value;
    }

    Ok(DiscordGatewayResolvedConfig {
        auth_profile,
        session: DiscordGatewayRuntimeConfig {
            token,
            intents,
            identify_properties,
        },
    })
}

async fn open_discord_gateway_websocket_url(
    request: &SubscribeStartRequest,
    auth_profile: &Profile,
) -> Result<(String, DiscordGatewayBotResponse)> {
    let open_endpoint = derive_gateway_bot_endpoint(&request.endpoint)?;
    let resolved = auth::resolve_profile_operation_request_auth(
        &auth::AuthRequestContext::new("GET", &open_endpoint),
        auth_profile,
    )?;
    let client = crate::http_client::build_resilient_http_client(
        Duration::from_secs(10),
        "Discord Gateway open",
    )?;
    let mut req = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        req = req.header(name, value);
    }
    let response = req.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read response body>".to_string());
        let body = truncate_error_body(&body, 512);
        bail!(
            "Discord Gateway open request failed with status {}: {}",
            status,
            body
        );
    }
    let body = response.json::<Value>().await?;
    let parsed = parse_gateway_bot_response(&body)?;
    Ok((
        prepare_gateway_websocket_url(&parsed.websocket_url)?,
        parsed,
    ))
}

async fn run_slack_socket_mode_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;

    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut sink, &view, &mut seq, "slack_socket_mode").await?;
            return Ok(());
        }

        let websocket_url = match open_slack_socket_mode_websocket_url(request).await {
            Ok(url) => url,
            Err(err) => {
                let message = err.to_string();
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "slack_socket_mode",
                    "error",
                    None,
                    Some(json!({ "message": message })),
                )
                .await?;
                update_subscription_view(&view, Some("reconnecting"), Some(message), true).await;
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "slack_socket_mode",
                    "reconnect",
                    None,
                    Some(json!({ "delay_secs": delay_secs, "phase": "open_url" })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "slack_socket_mode")
                        .await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
                continue;
            }
        };

        let mut handler = SlackSocketModeHandler::new();
        let config = WebSocketRuntimeConfig {
            endpoint: websocket_url,
            auth_profile: None,
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
            first_message_timeout_secs: Some(5),
            initial_reconnect_delay_secs: SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS,
            max_reconnect_delay_secs: SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS,
        };

        let result = {
            let mut observer = DaemonWebSocketObserver {
                sink: &mut sink,
                view: &view,
                seq: &mut seq,
                source_kind: "slack_socket_mode",
            };
            subscription_websocket::run_websocket_subscription_session_once(
                &config,
                &mut handler,
                &mut observer,
                &mut stop_rx,
            )
            .await
        };

        match result {
            Ok(()) => return Ok(()),
            Err(WebSocketRunError::Fatal(err)) => {
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "slack_socket_mode",
                    "error",
                    None,
                    Some(json!({ "message": err.to_string() })),
                )
                .await?;
                return Err(err);
            }
            Err(WebSocketRunError::Retry(err)) => {
                let message = err.to_string();
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "slack_socket_mode",
                    "error",
                    None,
                    Some(json!({ "message": message })),
                )
                .await?;
                update_subscription_view(&view, Some("reconnecting"), Some(err.to_string()), true)
                    .await;
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "slack_socket_mode",
                    "reconnect",
                    None,
                    Some(json!({ "delay_secs": delay_secs })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "slack_socket_mode")
                        .await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

async fn run_feishu_long_connection_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;

    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut sink, &view, &mut seq, "feishu_long_connection")
                .await?;
            return Ok(());
        }

        let open = match open_feishu_long_connection_websocket_url(request).await {
            Ok(open) => open,
            Err(err) => {
                let message = err.to_string();
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "feishu_long_connection",
                    "error",
                    None,
                    Some(json!({ "message": message })),
                )
                .await?;
                update_subscription_view(&view, Some("reconnecting"), Some(message), true).await;
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "feishu_long_connection",
                    "reconnect",
                    None,
                    Some(json!({ "delay_secs": delay_secs, "phase": "open_url" })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(
                        &mut sink,
                        &view,
                        &mut seq,
                        "feishu_long_connection",
                    )
                    .await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
                continue;
            }
        };

        let config = WebSocketRuntimeConfig {
            endpoint: open.websocket_url.clone(),
            auth_profile: None,
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
            first_message_timeout_secs: None,
            initial_reconnect_delay_secs: SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS,
            max_reconnect_delay_secs: SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS,
        };
        let mut handler =
            FeishuLongConnectionHandler::new(open.service_id, open.ping_interval_secs);

        let result = {
            let mut observer = DaemonWebSocketObserver {
                sink: &mut sink,
                view: &view,
                seq: &mut seq,
                source_kind: "feishu_long_connection",
            };
            subscription_websocket::run_websocket_subscription_session_once(
                &config,
                &mut handler,
                &mut observer,
                &mut stop_rx,
            )
            .await
        };

        match result {
            Ok(()) => return Ok(()),
            Err(WebSocketRunError::Fatal(err)) => {
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "feishu_long_connection",
                    "error",
                    None,
                    Some(json!({ "message": err.to_string() })),
                )
                .await?;
                return Err(err);
            }
            Err(WebSocketRunError::Retry(err)) => {
                let message = err.to_string();
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "feishu_long_connection",
                    "error",
                    None,
                    Some(json!({ "message": message })),
                )
                .await?;
                update_subscription_view(&view, Some("reconnecting"), Some(err.to_string()), true)
                    .await;
                append_subscription_event(
                    &mut sink,
                    &view,
                    &mut seq,
                    "feishu_long_connection",
                    "reconnect",
                    None,
                    Some(json!({
                        "delay_secs": delay_secs,
                        "ping_interval_secs": open.ping_interval_secs,
                        "reconnect_count": open.reconnect_count,
                        "reconnect_interval_secs": open.reconnect_interval_secs,
                        "reconnect_nonce_secs": open.reconnect_nonce_secs,
                    })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(
                        &mut sink,
                        &view,
                        &mut seq,
                        "feishu_long_connection",
                    )
                    .await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

fn resolve_jsonrpc_subscription_config(
    request: &SubscribeStartRequest,
) -> Result<JsonRpcSubscriptionConfig> {
    let operation_id = request
        .operation_id
        .as_ref()
        .ok_or_else(|| anyhow!("operation_id is required for JSON-RPC subscriptions"))?;
    let unsubscribe_operation_id = resolve_jsonrpc_unsubscribe_operation(operation_id)?;
    let params = match request.args.clone().unwrap_or_default().remove("params") {
        Some(params) => params,
        None => Value::Null,
    };
    let has_extra_args = request
        .args
        .as_ref()
        .is_some_and(|args| args.keys().any(|key| key != "params"));
    if has_extra_args {
        bail!("JSON-RPC subscriptions accept only a top-level 'params' argument");
    }

    Ok(JsonRpcSubscriptionConfig {
        operation_id: operation_id.clone(),
        unsubscribe_operation_id,
        params: if params.is_null() { None } else { Some(params) },
    })
}

fn resolve_poll_subscription_config(
    request: &SubscribeStartRequest,
) -> Result<PollSubscriptionConfig> {
    if request.operation_id.is_none() {
        bail!("poll subscriptions require an operation_id");
    }
    if request.resource_uri.is_some() {
        bail!("poll subscriptions do not support --resource-uri");
    }
    let raw = request
        .poll_config
        .clone()
        .ok_or_else(|| anyhow!("poll subscriptions require --poll-config"))?;
    let config: PollSubscriptionConfig =
        serde_json::from_value(raw).context("invalid poll subscription config")?;
    config.validate()?;
    Ok(config)
}

async fn resolve_graphql_subscription_prepared_operation(
    request: &SubscribeStartRequest,
) -> Result<(
    adapters::graphql::GraphQLAdapter,
    String,
    HashMap<String, Value>,
)> {
    let operation_id = request
        .operation_id
        .as_ref()
        .ok_or_else(|| anyhow!("operation_id is required for GraphQL subscriptions"))?;
    if !operation_id.starts_with("subscription/") {
        bail!("subscribe start currently supports only GraphQL subscription/<field> operation IDs");
    }

    let cache = if request.options.no_cache {
        cache::create_cache(CacheConfig {
            enabled: false,
            ..Default::default()
        })?
    } else if let Some(ttl) = request.options.cache_ttl {
        cache::create_cache(CacheConfig {
            ttl,
            ..Default::default()
        })?
    } else {
        cache::create_cache(CacheConfig::load_from_file().unwrap_or_default())?
    };
    let root_auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let detection_options = DetectionOptions {
        schema_url: request.options.schema_url.clone(),
        auth_profile: root_auth_profile.clone(),
        stdio_spawn_options: None,
        request_timeout: request_timeout_duration(request.options.timeout_ms),
    };

    let resolved = resolve_adapter_with_schema_cache(
        &request.endpoint,
        &detection_options,
        cache,
        root_auth_profile.clone(),
        request.options.no_cache,
        request.options.refresh_schema,
    )
    .await?;

    let adapter = match resolved.adapter {
        AdapterEnum::GraphQL(adapter) => adapter,
        other => bail!(
            "endpoint '{}' does not resolve to GraphQL for subscription '{}'; detected {}",
            request.endpoint,
            operation_id,
            other.protocol_type().as_str()
        ),
    };

    Ok((
        adapter,
        operation_id.clone(),
        request.args.clone().unwrap_or_default(),
    ))
}

async fn run_graphql_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let (adapter, operation_id, args) =
        resolve_graphql_subscription_prepared_operation(request).await?;
    let prepared = adapter
        .prepare_operation(&request.endpoint, &operation_id, args)
        .await?;
    let endpoint = derive_graphql_websocket_endpoint(&request.endpoint)?;

    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let handler_config = GraphQLSubscriptionConfig {
        operation_id: operation_id.clone(),
        query: prepared.query,
        variables: prepared.variables,
    };
    let mut observer = DaemonWebSocketObserver {
        sink: &mut sink,
        view: &view,
        seq: &mut seq,
        source_kind: "graphql",
    };

    let mut profile = GraphQLWebSocketProfile::Modern;
    let mut profile_locked = false;
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;

    let result: Result<()> = loop {
        let mut handler = GraphQLSubscriptionHandler::new(handler_config.clone(), profile);
        let config = WebSocketRuntimeConfig {
            endpoint: endpoint.clone(),
            auth_profile: auth_profile.clone(),
            subprotocols: vec![profile.subprotocol().to_string()],
            initial_text_frames: vec![graphql_transport_init_message()],
            first_message_timeout_secs: Some(5),
            initial_reconnect_delay_secs: SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS,
            max_reconnect_delay_secs: SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS,
        };
        let mut attempt_observer = GraphQLFallbackObserver {
            inner: &mut observer,
            saw_data: false,
        };

        match subscription_websocket::run_websocket_subscription_session_once(
            &config,
            &mut handler,
            &mut attempt_observer,
            &mut stop_rx,
        )
        .await
        {
            Ok(()) if attempt_observer.saw_data || handler.has_received_data() => break Ok(()),
            Ok(()) => break Ok(()),
            Err(WebSocketRunError::Fatal(err)) => {
                if attempt_observer.saw_data || handler.has_received_data() {
                    profile_locked = true;
                }
                if !profile_locked {
                    if let Some(fallback) = err.downcast_ref::<GraphQLProfileFallback>() {
                        WebSocketRuntimeObserver::emit(
                            &mut observer,
                            "reconnect",
                            None,
                            Some(json!({
                                "reason": fallback.reason,
                                "from_profile": fallback.from.protocol_label(),
                                "to_profile": fallback.to.protocol_label(),
                                "compatibility_fallback": true,
                                "delay_secs": 0,
                            })),
                        )
                        .await?;
                        profile = fallback.to;
                        continue;
                    }
                }
                break Err(err);
            }
            Err(WebSocketRunError::Retry(err)) => {
                if attempt_observer.saw_data || handler.has_received_data() {
                    profile_locked = true;
                    delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
                } else if !profile_locked && graphql_retry_supports_fallback(profile, &err) {
                    WebSocketRuntimeObserver::emit(
                        &mut observer,
                        "reconnect",
                        None,
                        Some(json!({
                            "reason": err.to_string(),
                            "from_profile": profile.protocol_label(),
                            "to_profile": GraphQLWebSocketProfile::Legacy.protocol_label(),
                            "compatibility_fallback": true,
                            "delay_secs": 0,
                        })),
                    )
                    .await?;
                    profile = GraphQLWebSocketProfile::Legacy;
                    continue;
                }

                let message = err.to_string();
                WebSocketRuntimeObserver::emit(
                    &mut observer,
                    "error",
                    None,
                    Some(json!({ "message": message })),
                )
                .await?;
                WebSocketRuntimeObserver::update_status(
                    &mut observer,
                    Some("reconnecting"),
                    Some(message.clone()),
                    true,
                )
                .await?;
                WebSocketRuntimeObserver::emit(
                    &mut observer,
                    "reconnect",
                    None,
                    Some(json!({
                        "delay_secs": delay_secs,
                        "graphql_profile": profile.protocol_label(),
                    })),
                )
                .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "graphql").await?;
                    break Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    };

    if let Err(err) = result {
        append_subscription_event(
            &mut sink,
            &view,
            &mut seq,
            "graphql",
            "error",
            None,
            Some(json!({
                "message": err.to_string(),
                "operation_id": operation_id,
            })),
        )
        .await?;
        return Err(err);
    }

    Ok(())
}

async fn run_jsonrpc_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let config = resolve_jsonrpc_subscription_config(request)?;
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let subscribe_message = JsonRpcSubscriptionHandler::new(config.clone()).subscribe_message();
    let mut handler = JsonRpcSubscriptionHandler::new(config.clone());
    let mut observer = DaemonWebSocketObserver {
        sink: &mut sink,
        view: &view,
        seq: &mut seq,
        source_kind: "jsonrpc_pubsub",
    };

    let result = subscription_websocket::run_websocket_subscription_runtime(
        WebSocketRuntimeConfig {
            endpoint: request.endpoint.clone(),
            auth_profile,
            subprotocols: Vec::new(),
            initial_text_frames: vec![subscribe_message],
            first_message_timeout_secs: Some(5),
            initial_reconnect_delay_secs: SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS,
            max_reconnect_delay_secs: SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS,
        },
        &mut handler,
        &mut observer,
        &mut stop_rx,
    )
    .await;

    if let Err(err) = result {
        append_subscription_event(
            &mut sink,
            &view,
            &mut seq,
            "jsonrpc_pubsub",
            "error",
            None,
            Some(json!({
                "message": err.to_string(),
                "operation_id": config.operation_id,
            })),
        )
        .await?;
        return Err(err);
    }

    Ok(())
}

struct DaemonPollContext {
    runtime: DaemonRuntime,
    request: SubscribeStartRequest,
    checkpoint_path: PathBuf,
}

#[async_trait::async_trait]
impl PollRuntimeContext for DaemonPollContext {
    async fn load_checkpoint(
        &mut self,
    ) -> Result<Option<crate::subscription_poll::PollCheckpointState>> {
        match tokio::fs::read(&self.checkpoint_path).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    async fn store_checkpoint(
        &mut self,
        checkpoint: &crate::subscription_poll::PollCheckpointState,
    ) -> Result<()> {
        if let Some(parent) = self.checkpoint_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&self.checkpoint_path, serde_json::to_vec(checkpoint)?).await?;
        Ok(())
    }

    async fn fetch(
        &mut self,
        args: HashMap<String, Value>,
        checkpoint: &crate::subscription_poll::PollCheckpointState,
    ) -> Result<crate::subscription_poll::PollFetchResult> {
        let mut options = self.request.options.clone();
        if let Some(etag) = checkpoint.etag.as_ref() {
            options
                .request_headers
                .insert("if-none-match".to_string(), etag.clone());
        }
        let response = self
            .runtime
            .invoke(RuntimeInvokeRequest {
                request_id: format!("{}-poll-{}", self.request.request_id, now_unix_secs()),
                endpoint: self.request.endpoint.clone(),
                action: RuntimeAction::Execute,
                operation_id: self.request.operation_id.clone(),
                args: Some(args),
                options,
            })
            .await?;
        Ok(crate::subscription_poll::PollFetchResult {
            data: response.data,
            duration_ms: response.duration_ms,
            status_code: response.meta.response_status_code,
            response_headers: response.meta.response_headers.unwrap_or_default(),
        })
    }
}

fn subscription_checkpoint_path(job_id: &str) -> PathBuf {
    daemon_dir()
        .join("subscriptions")
        .join(format!("{job_id}.checkpoint.json"))
}

async fn cleanup_subscription_checkpoint(job_id: &str) {
    let _ = tokio::fs::remove_file(subscription_checkpoint_path(job_id)).await;
}

async fn run_poll_subscription_job(
    runtime: &DaemonRuntime,
    job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let config = resolve_poll_subscription_config(request)?;
    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut observer = DaemonWebSocketObserver {
        sink: &mut sink,
        view: &view,
        seq: &mut seq,
        source_kind: "poll",
    };
    let mut context = DaemonPollContext {
        runtime: runtime.clone(),
        request: request.clone(),
        checkpoint_path: subscription_checkpoint_path(job_id),
    };

    let result = crate::subscription_poll::run_poll_subscription_runtime(
        config,
        request.args.clone().unwrap_or_default(),
        &mut context,
        &mut observer,
        &mut stop_rx,
    )
    .await;

    if result.is_ok() {
        cleanup_subscription_checkpoint(job_id).await;
    }
    result
}

async fn run_mcp_subscription_job(
    runtime: &DaemonRuntime,
    _job_id: &str,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let resource_uri = request
        .resource_uri
        .as_ref()
        .ok_or_else(|| anyhow!("resource_uri is required for MCP subscriptions"))?;
    if adapters::mcp::McpAdapter::is_http_url(&request.endpoint) {
        return run_mcp_http_subscription_job(
            runtime,
            request,
            sink_path,
            view,
            resource_uri,
            stop_rx,
        )
        .await;
    }
    if !adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint) {
        bail!("MCP subscriptions require a stdio command or http(s) MCP endpoint");
    }
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let cache = runtime.build_cache(&request.options)?;
    let spawn_options =
        build_stdio_spawn_options(&request.endpoint, &request.options, auth_profile.as_ref())?
            .unwrap_or_default();
    let (cmd, cmd_args) = adapters::mcp::McpAdapter::parse_stdio_command(&request.endpoint)?;
    let session_key = stdio_session_key(
        &request.endpoint,
        auth_profile.as_ref(),
        &request.options.inject_env,
    )?;
    let (session, _) = runtime
        .mcp
        .get_or_create_stdio(
            &session_key,
            &cmd,
            &cmd_args,
            &spawn_options,
            request_timeout_duration(request.options.timeout_ms),
            StdioSessionRequestMetadata {
                idle_ttl_secs: request.options.daemon_idle_ttl,
                link_name: request.options.link_name.as_deref(),
                link_skill: request.options.link_skill.as_deref(),
                link_skill_doc: request.options.link_skill_doc.as_deref(),
                link_skill_path: request.options.link_skill_path.as_deref(),
                endpoint: &request.endpoint,
                exclusive_keys: &request.options.daemon_exclusive,
            },
        )
        .await?;
    {
        let mut guard = session.lock().await;
        if !guard.client.supports_resource_subscribe() {
            bail!("MCP server does not support resources.subscribe");
        }
        guard
            .ensure_resource_subscription(resource_uri, &request.endpoint, Some(&cache))
            .await?;
    }

    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut cursor = 0u64;
    append_subscription_event(
        &mut sink,
        &view,
        &mut seq,
        "mcp_resource",
        "open",
        None,
        Some(json!({ "resource_uri": resource_uri })),
    )
    .await?;
    if request.read_resource {
        let read_result = {
            let mut guard = session.lock().await;
            guard
                .read_resource(resource_uri, &request.endpoint, Some(&cache))
                .await
        };
        append_mcp_resource_read_result(
            &mut sink,
            &view,
            &mut seq,
            resource_uri,
            "initial_read",
            "failed to read initial resource snapshot",
            read_result,
        )
        .await?;
    }

    let run_result: Result<()> = 'run: loop {
        tokio::select! {
            stop_requested = subscription_stop_requested(&mut stop_rx) => {
                if stop_requested {
                    match close_subscription_as_stopped(&mut sink, &view, &mut seq, "mcp_resource").await {
                        Ok(()) => break 'run Ok(()),
                        Err(err) => break 'run Err(err),
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let (notifications, next_cursor) = {
                    let mut guard = session.lock().await;
                    guard
                        .notifications_since(cursor, &request.endpoint, Some(&cache))
                        .await
                };
                cursor = next_cursor;
                for notification in notifications {
                    if let Err(err) = append_subscription_event(
                        &mut sink,
                        &view,
                        &mut seq,
                        "mcp_resource",
                        "data",
                        notification.params.clone(),
                        Some(json!({"method": notification.method})),
                    ).await {
                        break 'run Err(err);
                    }
                    if request.read_resource && should_read_mcp_resource_snapshot(&notification) {
                        let read_result = {
                            let mut guard = session.lock().await;
                            guard
                                .read_resource(resource_uri, &request.endpoint, Some(&cache))
                                .await
                        };
                        if let Err(err) = append_mcp_resource_read_result(
                            &mut sink,
                            &view,
                            &mut seq,
                            resource_uri,
                            "resource_updated",
                            "failed to read resource after update",
                            read_result,
                        )
                        .await {
                            break 'run Err(err);
                        }
                    }
                }
            }
        }
    };

    let unsubscribe_result = {
        let mut guard = session.lock().await;
        guard
            .release_resource_subscription(resource_uri, &request.endpoint, Some(&cache))
            .await
    };
    if let Err(err) = unsubscribe_result {
        let msg = format!("failed to unsubscribe resource before shutdown: {}", err);
        append_subscription_event(
            &mut sink,
            &view,
            &mut seq,
            "mcp_resource",
            "error",
            None,
            Some(json!({ "message": msg })),
        )
        .await?;
        update_subscription_view(&view, None, Some(msg.clone()), false).await;
        if run_result.is_ok() {
            return Err(err);
        }
    }

    run_result
}

async fn run_mcp_http_subscription_job(
    runtime: &DaemonRuntime,
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    resource_uri: &str,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let lookup_key = http_session_lookup_key(&request.endpoint, auth_profile.as_ref());
    let (session, resolved_transport) =
        if let Some((session, _)) = runtime.mcp.get_http_by_lookup_key(&lookup_key).await {
            (session, None)
        } else {
            let resolved_transport =
                resolve_mcp_http_endpoint(&request.endpoint, auth_profile.clone()).await?;
            let session_key = format!(
                "http:{:?}:{}:{}",
                resolved_transport.mode,
                resolved_transport.connect_url,
                auth_fingerprint(auth_profile.as_ref())
            );
            let (session, _) = runtime
                .mcp
                .get_or_create_http(
                    &lookup_key,
                    &session_key,
                    &resolved_transport,
                    auth_profile,
                    request_timeout_duration(request.options.timeout_ms),
                )
                .await?;
            (session, Some(resolved_transport))
        };
    session.ensure_resource_subscription(resource_uri).await?;

    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    let mut cursor = 0u64;
    append_subscription_event(
        &mut sink,
        &view,
        &mut seq,
        "mcp_resource",
        "open",
        None,
        Some(json!({
            "resource_uri": resource_uri,
            "transport_mode": resolved_transport
                .as_ref()
                .map(|value| format!("{:?}", value.mode))
                .unwrap_or_else(|| "reused".to_string()),
            "connect_url": resolved_transport
                .as_ref()
                .map(|value| redact_endpoint(&value.connect_url))
                .unwrap_or_else(|| redact_endpoint(&request.endpoint)),
        })),
    )
    .await?;
    if request.read_resource {
        let read_result = session.read_resource(resource_uri).await;
        append_mcp_resource_read_result(
            &mut sink,
            &view,
            &mut seq,
            resource_uri,
            "initial_read",
            "failed to read initial resource snapshot",
            read_result,
        )
        .await?;
    }

    let run_result: Result<()> = 'run: loop {
        tokio::select! {
            stop_requested = subscription_stop_requested(&mut stop_rx) => {
                if stop_requested {
                    match close_subscription_as_stopped(&mut sink, &view, &mut seq, "mcp_resource").await {
                        Ok(()) => break 'run Ok(()),
                        Err(err) => break 'run Err(err),
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let (notifications, next_cursor, stream_error) = session.notifications_since(cursor).await;
                cursor = next_cursor;
                if let Some(err) = stream_error {
                    break 'run Err(anyhow!("MCP HTTP subscription stream failed: {}", err));
                }
                for notification in notifications {
                    if let Err(err) = append_subscription_event(
                        &mut sink,
                        &view,
                        &mut seq,
                        "mcp_resource",
                        "data",
                        notification.params.clone(),
                        Some(json!({"method": notification.method})),
                    ).await {
                        break 'run Err(err);
                    }
                    if request.read_resource && should_read_mcp_resource_snapshot(&notification) {
                        let read_result = session.read_resource(resource_uri).await;
                        if let Err(err) = append_mcp_resource_read_result(
                            &mut sink,
                            &view,
                            &mut seq,
                            resource_uri,
                            "resource_updated",
                            "failed to read resource after update",
                            read_result,
                        )
                        .await {
                            break 'run Err(err);
                        }
                    }
                }
            }
        }
    };

    let unsubscribe_result = session.release_resource_subscription(resource_uri).await;
    if let Err(err) = unsubscribe_result {
        let msg = format!("failed to unsubscribe resource before shutdown: {}", err);
        append_subscription_event(
            &mut sink,
            &view,
            &mut seq,
            "mcp_resource",
            "error",
            None,
            Some(json!({ "message": msg })),
        )
        .await?;
        update_subscription_view(&view, None, Some(msg.clone()), false).await;
        if run_result.is_ok() {
            return Err(err);
        }
    }

    run_result
}

#[cfg(unix)]
pub async fn daemon_status_client() -> Result<DaemonStatus> {
    let value = client_call("daemon.status", None).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn daemon_status_client() -> Result<DaemonStatus> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn daemon_sessions_client() -> Result<Vec<DaemonSessionView>> {
    let value = client_call("daemon.sessions", None).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn daemon_sessions_client() -> Result<Vec<DaemonSessionView>> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn daemon_stop_client() -> Result<()> {
    let _ = client_call("daemon.shutdown", None).await?;
    Ok(())
}

#[cfg(not(unix))]
pub async fn daemon_stop_client() -> Result<()> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn runtime_invoke_client(
    request: &RuntimeInvokeRequest,
) -> Result<RuntimeInvokeResponse> {
    let params = serde_json::to_value(request)?;
    let value = client_call("runtime.invoke", Some(params)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn runtime_invoke_client(
    _request: &RuntimeInvokeRequest,
) -> Result<RuntimeInvokeResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn subscribe_start_client(
    request: &SubscribeStartRequest,
) -> Result<SubscribeStartResponse> {
    let params = serde_json::to_value(request)?;
    let value = client_call("subscription.start", Some(params)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn subscribe_start_client(
    _request: &SubscribeStartRequest,
) -> Result<SubscribeStartResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn subscribe_list_client() -> Result<Vec<SubscriptionJobView>> {
    let value = client_call("subscription.list", None).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn subscribe_list_client() -> Result<Vec<SubscriptionJobView>> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn subscribe_status_client(job_id: &str) -> Result<SubscriptionJobView> {
    let value = client_call("subscription.status", Some(json!({ "job_id": job_id }))).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn subscribe_status_client(_job_id: &str) -> Result<SubscriptionJobView> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn subscribe_stop_client(job_id: &str) -> Result<SubscribeStopResponse> {
    let value = client_call("subscription.stop", Some(json!({ "job_id": job_id }))).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn subscribe_stop_client(_job_id: &str) -> Result<SubscribeStopResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
#[allow(dead_code)]
pub async fn subscribe_events_client(
    request: &SubscriptionEventsRequest,
) -> Result<SubscriptionEventsResponse> {
    let value = client_call("subscription.events", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub async fn subscribe_events_client(
    _request: &SubscriptionEventsRequest,
) -> Result<SubscriptionEventsResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn source_ensure_client(
    request: &ManagedSourceEnsureRequest,
) -> Result<ManagedSourceEnsureResponse> {
    let value = client_call("source.ensure", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn source_ensure_client(
    _request: &ManagedSourceEnsureRequest,
) -> Result<ManagedSourceEnsureResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn source_status_client(
    request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceView> {
    let value = client_call("source.status", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn source_status_client(
    _request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceView> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn source_stop_client(
    request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceStopResponse> {
    let value = client_call("source.stop", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn source_stop_client(
    _request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceStopResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn source_delete_client(
    request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceDeleteResponse> {
    let value = client_call("source.delete", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn source_delete_client(
    _request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceDeleteResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn stream_read_client(
    request: &ManagedStreamReadRequest,
) -> Result<ManagedStreamReadResponse> {
    let value = client_call("stream.read", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn stream_read_client(
    _request: &ManagedStreamReadRequest,
) -> Result<ManagedStreamReadResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn stream_info_client(stream_id: &str) -> Result<ManagedStreamInfo> {
    let value = client_call("stream.info", Some(json!({ "stream_id": stream_id }))).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn stream_info_client(_stream_id: &str) -> Result<ManagedStreamInfo> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn stream_trim_client(
    request: &ManagedStreamTrimRequest,
) -> Result<ManagedStreamTrimResponse> {
    let value = client_call("stream.trim", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn stream_trim_client(
    _request: &ManagedStreamTrimRequest,
) -> Result<ManagedStreamTrimResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
async fn start_daemon_process() -> Result<()> {
    let dir = daemon_dir();
    ensure_private_dir(&dir)?;
    let lock_path = dir.join("start.lock");
    let start_lock = try_acquire_start_lock(&lock_path)?;
    let got_lock = start_lock.is_some();

    if got_lock {
        let current_exe = std::env::current_exe().context("Cannot resolve current executable")?;
        let mut child_cmd = std::process::Command::new(current_exe);
        child_cmd
            .arg("daemon")
            .arg("_serve")
            // Avoid corrupting coverage artifacts when parent test runners
            // terminate long-lived daemon processes in CI.
            .env_remove("LLVM_PROFILE_FILE")
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: the child process does not touch shared Rust state after fork; it only
        // requests a new session id before exec so the daemon survives parent cleanup.
        unsafe {
            child_cmd.pre_exec(|| {
                // Detach from the invoking process group so parent shell cleanup after
                // the command exits does not also terminate the daemon.
                if setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let _child = child_cmd
            .spawn()
            .context("Failed to spawn daemon process")?;
    }

    for _ in 0..START_POLL_TRIES {
        tokio::time::sleep(Duration::from_millis(START_POLL_INTERVAL_MS)).await;
        if daemon_status_client().await.is_ok() {
            return Ok(());
        }
    }

    drop(start_lock);
    bail!("Daemon failed to start. Run `uxc daemon status` for diagnostics.")
}

#[cfg(not(unix))]
async fn start_daemon_process() -> Result<()> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
fn daemon_version_matches(status: &DaemonStatus) -> bool {
    status.version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
}

#[cfg(unix)]
pub async fn ensure_compatible_daemon_running() -> Result<EnsureDaemonOutcome> {
    match daemon_status_client().await {
        Ok(status) => {
            if daemon_version_matches(&status) {
                return Ok(EnsureDaemonOutcome {
                    started_now: false,
                    restarted_for_version_mismatch: false,
                    previous_version: None,
                });
            }

            let previous_version = status.version.clone();
            if let Err(err) = daemon_stop_local().await {
                return Err(UxcError::DaemonVersionMismatch(format!(
                    "Detected daemon version mismatch (daemon={}, cli={}) and failed to restart daemon automatically: {}. Run `uxc daemon restart`.",
                    previous_version.as_deref().unwrap_or("unknown"),
                    env!("CARGO_PKG_VERSION"),
                    err
                ))
                .into());
            }
            if let Err(err) = start_daemon_process().await {
                return Err(UxcError::DaemonVersionMismatch(format!(
                    "Detected daemon version mismatch (daemon={}, cli={}) and failed to restart daemon automatically: {}. Run `uxc daemon restart`.",
                    previous_version.as_deref().unwrap_or("unknown"),
                    env!("CARGO_PKG_VERSION"),
                    err
                ))
                .into());
            }

            Ok(EnsureDaemonOutcome {
                started_now: true,
                restarted_for_version_mismatch: true,
                previous_version,
            })
        }
        Err(_) => {
            start_daemon_process().await?;
            Ok(EnsureDaemonOutcome {
                started_now: true,
                restarted_for_version_mismatch: false,
                previous_version: None,
            })
        }
    }
}

#[cfg(not(unix))]
pub async fn ensure_compatible_daemon_running() -> Result<EnsureDaemonOutcome> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn run_daemon_server() -> Result<()> {
    let dir = daemon_dir();
    ensure_private_dir(&dir)?;
    let socket = socket_path();
    if socket.exists() {
        let _ = fs::remove_file(&socket);
    }

    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("Failed to bind daemon socket at {}", socket.display()))?;

    let runtime = Arc::new(DaemonRuntime::try_new()?);
    let resume_runtime = runtime.clone();
    tokio::spawn(async move {
        if let Err(err) = resume_runtime.resume_persisted_subscriptions().await {
            tracing::warn!("Failed to resume persisted subscriptions: {}", err);
        }
        if let Err(err) = resume_runtime.resume_managed_sources().await {
            tracing::warn!("Failed to resume managed sources: {}", err);
        }
    });

    // Log daemon start
    runtime
        .log(DaemonLogEntry::new(DaemonEventType::DaemonStart))
        .await;

    loop {
        let (stream, _) = listener.accept().await?;
        let runtime_for_conn = runtime.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, runtime_for_conn).await {
                tracing::debug!("daemon connection failed: {}", err);
            }
        });

        if runtime.should_stop().await {
            break;
        }
    }

    // Log daemon stop
    runtime
        .log(DaemonLogEntry::new(DaemonEventType::DaemonStop))
        .await;

    let _ = fs::remove_file(&socket);
    Ok(())
}

#[cfg(not(unix))]
pub async fn run_daemon_server() -> Result<()> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
async fn handle_connection(mut stream: UnixStream, runtime: Arc<DaemonRuntime>) -> Result<()> {
    let req_val = match read_frame(&mut stream).await {
        Ok(value) => value,
        Err(err) => {
            let _ = write_jsonrpc_error(
                &mut stream,
                Value::Null,
                -32700,
                format!("Parse error: {err}"),
            )
            .await;
            return Ok(());
        }
    };
    let req: JsonRpcRequest = match serde_json::from_value(req_val) {
        Ok(req) => req,
        Err(err) => {
            let _ = write_jsonrpc_error(
                &mut stream,
                Value::Null,
                -32600,
                format!("Invalid request: {err}"),
            )
            .await;
            return Ok(());
        }
    };

    if req.jsonrpc != JSONRPC_VERSION {
        write_jsonrpc_error(
            &mut stream,
            req.id,
            -32600,
            "Invalid jsonrpc version".to_string(),
        )
        .await?;
        return Ok(());
    }

    let response = match req.method.as_str() {
        "daemon.ping" => JsonRpcResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: req.id,
            result: Some(json!({"ok": true})),
            error: None,
        },
        "daemon.status" => {
            let status = runtime.status().await;
            JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: req.id,
                result: Some(serde_json::to_value(status)?),
                error: None,
            }
        }
        "daemon.sessions" => {
            let sessions = runtime.session_views().await;
            JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: req.id,
                result: Some(serde_json::to_value(sessions)?),
                error: None,
            }
        }
        "daemon.shutdown" => {
            runtime.request_stop().await;
            JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: req.id,
                result: Some(json!({"ok": true})),
                error: None,
            }
        }
        "runtime.invoke" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let invoke: RuntimeInvokeRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.invoke(invoke).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "subscription.start" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let start: SubscribeStartRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.subscribe_start(start).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "subscription.list" => JsonRpcResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: req.id,
            result: Some(serde_json::to_value(runtime.subscribe_list().await)?),
            error: None,
        },
        "subscription.status" => {
            let Some(job_id) = req
                .params
                .as_ref()
                .and_then(|v| v.get("job_id"))
                .and_then(Value::as_str)
            else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing job_id".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            match runtime.subscribe_status(job_id).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "subscription.stop" => {
            let Some(job_id) = req
                .params
                .as_ref()
                .and_then(|v| v.get("job_id"))
                .and_then(Value::as_str)
            else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing job_id".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            match runtime.subscribe_stop(job_id).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "subscription.events" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let events: SubscriptionEventsRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.subscribe_events(&events).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "source.ensure" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let ensure: ManagedSourceEnsureRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.source_ensure(ensure).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "source.status" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let status: ManagedSourceStatusRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.source_status(&status).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "source.stop" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let stop: ManagedSourceStatusRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.source_stop(&stop).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "source.delete" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let delete: ManagedSourceStatusRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.source_delete(&delete).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "stream.read" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let read: ManagedStreamReadRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.stream_read(&read).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "stream.info" => {
            let Some(stream_id) = req
                .params
                .as_ref()
                .and_then(|v| v.get("stream_id"))
                .and_then(Value::as_str)
            else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing stream_id".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            match runtime.stream_info(stream_id).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        "stream.trim" => {
            let Some(params) = req.params else {
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                };
                write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                return Ok(());
            };
            let trim: ManagedStreamTrimRequest = match serde_json::from_value(params) {
                Ok(value) => value,
                Err(err) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req.id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid params: {err}"),
                            data: None,
                        }),
                    };
                    write_frame(&mut stream, &serde_json::to_value(resp)?).await?;
                    return Ok(());
                }
            };
            match runtime.stream_trim(&trim).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: None,
                    error: Some(jsonrpc_error_from_anyhow(&err)),
                },
            }
        }
        _ => JsonRpcResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: req.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", req.method),
                data: None,
            }),
        },
    };

    write_frame(&mut stream, &serde_json::to_value(response)?).await?;
    Ok(())
}

pub async fn daemon_status_local() -> Result<DaemonStatus> {
    daemon_status_client().await
}

pub async fn daemon_sessions_local() -> Result<Vec<DaemonSessionView>> {
    daemon_sessions_client().await
}

pub async fn daemon_start_local() -> Result<EnsureDaemonOutcome> {
    ensure_compatible_daemon_running().await
}

pub async fn daemon_stop_local() -> Result<bool> {
    if daemon_status_client().await.is_err() {
        return Ok(false);
    }
    daemon_stop_client().await?;
    for _ in 0..STOP_POLL_TRIES {
        tokio::time::sleep(Duration::from_millis(STOP_POLL_INTERVAL_MS)).await;
        if daemon_status_client().await.is_err() {
            return Ok(true);
        }
    }
    bail!("Daemon did not stop in time. Run `uxc daemon status` for diagnostics.")
}

#[cfg(unix)]
async fn client_call(method: &str, params: Option<Value>) -> Result<Value> {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        UnixStream::connect(socket_path()),
    )
    .await
    .context("Timed out connecting to daemon socket")?
    .with_context(|| {
        format!(
            "Failed to connect daemon socket {}",
            socket_path().display()
        )
    })?;

    let request = json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": 1,
        "method": method,
        "params": params,
    });
    write_frame(&mut stream, &request).await?;

    let resp_val = read_frame(&mut stream).await?;
    let resp: JsonRpcResponse = serde_json::from_value(resp_val)?;
    if let Some(err) = resp.error {
        if let Some(data) = err.data.as_ref() {
            if let Ok(payload) = serde_json::from_value::<StructuredErrorPayload>(data.clone()) {
                return Err(
                    StructuredError::new(payload.code, payload.message, payload.details).into(),
                );
            }
            return Err(structured_error_from_jsonrpc_error(
                i64::from(err.code),
                &err.message,
                Some(data),
                "EXECUTION_FAILED",
            )
            .into());
        }
        if err.code == -32602 {
            return Err(UxcError::InvalidArguments(err.message).into());
        }
        if err.code == ERR_PROTOCOL_DETECTION {
            return Err(UxcError::ProtocolDetectionFailed(err.message).into());
        }
        if err.code == ERR_OPERATION_NOT_FOUND {
            return Err(UxcError::OperationNotFound(err.message).into());
        }
        if err.code == ERR_OAUTH_REQUIRED {
            return Err(UxcError::OAuthRequired(err.message).into());
        }
        if err.code == ERR_OAUTH_REFRESH_FAILED {
            return Err(UxcError::OAuthRefreshFailed(err.message).into());
        }
        if err.code == ERR_OAUTH_SCOPE_INSUFFICIENT {
            return Err(UxcError::OAuthScopeInsufficient(err.message).into());
        }
        bail!("{}", err.message);
    }
    resp.result
        .ok_or_else(|| anyhow!("Missing JSON-RPC result"))
}

#[cfg(unix)]
async fn write_frame(stream: &mut UnixStream, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    tokio::time::timeout(
        Duration::from_secs(FRAME_IO_TIMEOUT_SECS),
        stream.write_all(header.as_bytes()),
    )
    .await
    .context("Timed out writing frame header")??;
    tokio::time::timeout(
        Duration::from_secs(FRAME_IO_TIMEOUT_SECS),
        stream.write_all(&body),
    )
    .await
    .context("Timed out writing frame body")??;
    tokio::time::timeout(Duration::from_secs(FRAME_IO_TIMEOUT_SECS), stream.flush())
        .await
        .context("Timed out flushing frame")??;
    Ok(())
}

#[cfg(unix)]
async fn read_frame(stream: &mut UnixStream) -> Result<Value> {
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];

    loop {
        let n = tokio::time::timeout(
            Duration::from_secs(FRAME_IO_TIMEOUT_SECS),
            stream.read(&mut byte),
        )
        .await
        .context("Timed out reading frame header")??;
        if n == 0 {
            bail!("EOF while reading frame header");
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
        if header.len() > 8192 {
            bail!("Frame header too large");
        }
    }

    let header_str = String::from_utf8(header)?;
    let mut content_len = None;
    for line in header_str.split("\r\n") {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_len = Some(rest.trim().parse::<usize>()?);
        }
    }

    let len = content_len.ok_or_else(|| anyhow!("Missing Content-Length header"))?;
    if len > MAX_FRAME_BODY_BYTES {
        bail!(
            "Frame body too large: {} bytes (max {})",
            len,
            MAX_FRAME_BODY_BYTES
        );
    }
    let mut body = vec![0_u8; len];
    tokio::time::timeout(
        Duration::from_secs(FRAME_IO_TIMEOUT_SECS),
        stream.read_exact(&mut body),
    )
    .await
    .context("Timed out reading frame body")??;
    Ok(serde_json::from_slice(&body)?)
}

fn daemon_dir() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("uxc");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".uxc").join("daemon");
    }

    let mut dir = std::env::temp_dir();
    dir.push(format!("uxc-{}", best_effort_user_label()));
    dir.push("daemon");
    dir
}

pub fn socket_path() -> PathBuf {
    daemon_dir().join("uxc.sock")
}

fn subscription_store_path() -> PathBuf {
    daemon_dir().join("subscriptions.json")
}

fn parse_subscription_numeric_id(job_id: &str) -> Option<u64> {
    job_id.strip_prefix("sub_")?.parse().ok()
}

fn write_subscription_store(path: &Path, jobs: &[PersistedSubscriptionRecord]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create subscription store directory {}",
                parent.display()
            )
        })?;
    }
    let store = PersistedSubscriptionStore {
        version: "v1".to_string(),
        jobs: jobs.to_vec(),
    };
    let tmp_path = path.with_extension("json.tmp");
    let raw = serde_json::to_vec_pretty(&store)?;
    fs::write(&tmp_path, raw).with_context(|| {
        format!(
            "Failed to write temporary subscription store {}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to replace subscription store {}", path.display()))?;
    Ok(())
}

fn quarantine_subscription_store(path: &Path, suffix: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let quarantined = path.with_extension(format!("json.{suffix}.bak"));
    fs::rename(path, &quarantined).with_context(|| {
        format!(
            "Failed to quarantine subscription store {} to {}",
            path.display(),
            quarantined.display()
        )
    })?;
    Ok(())
}

fn best_effort_user_label() -> String {
    let raw = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    let filtered = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if filtered.is_empty() {
        "unknown".to_string()
    } else {
        filtered
    }
}

fn auth_fingerprint(profile: Option<&Profile>) -> String {
    let mut hasher = Sha256::new();
    if let Some(p) = profile {
        hasher.update(p.auth_type.to_string().as_bytes());
        hasher.update(p.api_key.as_bytes());
        if let Some(name) = &p.name {
            hasher.update(name.as_bytes());
        }
        for (field_name, source_kind) in p.field_source_kinds() {
            hasher.update(field_name.as_bytes());
            hasher.update(source_kind.as_bytes());
        }
        if let Some(source) = &p.secret_source {
            hasher.update(source.kind().as_bytes());
        }
        if let Some(signer) = &p.signer {
            if let Ok(serialized) = serde_json::to_vec(signer) {
                hasher.update(&serialized);
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

fn stdio_env_fingerprint(specs: &[InjectEnvSpec], profile: Option<&Profile>) -> Result<String> {
    if specs.is_empty() {
        return Ok("none".to_string());
    }
    let profile = profile.ok_or_else(|| {
        UxcError::InvalidArguments(
            "--inject-env requires a credential. Use --auth <credential_id> for direct stdio calls, or --credential <credential_id> when creating a link.".to_string(),
        )
    })?;
    fingerprint_injected_env(specs, profile)
}

fn stdio_session_key(
    endpoint: &str,
    profile: Option<&Profile>,
    inject_env: &[InjectEnvSpec],
) -> Result<String> {
    Ok(format!(
        "stdio:{}:{}:{}",
        endpoint,
        auth_fingerprint(profile),
        stdio_env_fingerprint(inject_env, profile)?
    ))
}

fn http_session_lookup_key(endpoint: &str, profile: Option<&Profile>) -> String {
    format!("http_lookup:{}:{}", endpoint, auth_fingerprint(profile))
}

fn display_session_key(session_key: &str) -> String {
    let digest = Sha256::digest(session_key.as_bytes());
    let short_hash = digest[..8]
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    format!("stdio:{}", short_hash)
}

fn build_stdio_spawn_options(
    endpoint: &str,
    options: &RuntimeInvokeOptions,
    profile: Option<&Profile>,
) -> Result<Option<adapters::mcp::StdioSpawnOptions>> {
    if options.inject_env.is_empty() {
        return Ok(None);
    }
    if !adapters::mcp::McpAdapter::is_stdio_command(endpoint) {
        return Err(UxcError::InvalidArguments(
            "--inject-env is only supported for stdio endpoints".to_string(),
        )
        .into());
    }
    let profile = profile.ok_or_else(|| {
        UxcError::InvalidArguments(
            "--inject-env requires a credential. Use --auth <credential_id> for direct stdio calls, or --credential <credential_id> when creating a link.".to_string(),
        )
    })?;
    let env_overrides = render_injected_env(&options.inject_env, profile)?;
    Ok(Some(adapters::mcp::StdioSpawnOptions { env_overrides }))
}

fn normalize_exclusive_keys(keys: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for k in keys {
        let trimmed = k.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push(expand_tilde_key(trimmed));
    }
    out.sort();
    out.dedup();
    out
}

fn expand_tilde_key(key: &str) -> String {
    let Some(home) = resolve_home_dir_for_tilde() else {
        return key.to_string();
    };
    if key == "~" {
        return home.to_string_lossy().to_string();
    }
    if let Some(rest) = key.strip_prefix("~/") {
        return home.join(rest).to_string_lossy().to_string();
    }
    if let Some(rest) = key.strip_prefix("~\\") {
        return home.join(rest).to_string_lossy().to_string();
    }
    key.to_string()
}

fn resolve_home_dir_for_tilde() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        let home_drive = std::env::var_os("HOMEDRIVE");
        let home_path = std::env::var_os("HOMEPATH");
        if let (Some(drive), Some(path)) = (home_drive, home_path) {
            let mut combined = PathBuf::from(drive);
            combined.push(path);
            return Some(combined);
        }
    }
    None
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cache_age_ms(fetched_at: u64) -> u64 {
    now_unix_secs()
        .saturating_sub(fetched_at)
        .saturating_mul(1000)
}

fn apply_runtime_artifact_compaction(
    kind: &str,
    data: &mut Value,
    meta: &mut RuntimeMeta,
) -> Result<()> {
    if !matches!(kind, "host_help" | "operation_detail" | "call_result") {
        return Ok(());
    }

    let full_payload = serde_json::to_vec(data)?;
    let full_bytes = full_payload.len();
    if full_bytes <= ARTIFACT_COMPACTION_THRESHOLD_BYTES {
        return Ok(());
    }

    match write_runtime_artifact(kind, &full_payload) {
        Ok((artifact_path, sha256_hex)) => {
            *data = build_preview_value(data, 0);
            meta.artifact_truncated = Some(true);
            meta.artifact_kind = Some(kind.to_string());
            meta.artifact_bytes = Some(full_bytes as u64);
            meta.artifact_path = Some(artifact_path.display().to_string());
            meta.artifact_sha256 = Some(sha256_hex);
        }
        Err(err) => {
            tracing::warn!(
                "artifact compaction write failed for kind {}: {} (falling back to inline payload)",
                kind,
                err
            );
        }
    }

    Ok(())
}

fn write_runtime_artifact(kind: &str, payload: &[u8]) -> Result<(PathBuf, String)> {
    let artifacts_dir = daemon_dir().join("artifacts");
    ensure_private_dir(&artifacts_dir)?;

    let mut hasher = Sha256::new();
    hasher.update(payload);
    let digest = hasher.finalize();
    let sha256_hex = format!("{:x}", digest);
    let short_hash = &sha256_hex[..16];
    let artifact_name = format!("{}-{}-{}.json", kind, now_unix_secs(), short_hash);
    let artifact_path = artifacts_dir.join(artifact_name);
    fs::write(&artifact_path, payload).with_context(|| {
        format!(
            "failed to write compacted artifact payload to {}",
            artifact_path.display()
        )
    })?;

    Ok((artifact_path, sha256_hex))
}

fn build_preview_value(value: &Value, depth: usize) -> Value {
    if depth >= ARTIFACT_PREVIEW_MAX_DEPTH {
        return match value {
            Value::Object(_) => json!({"_uxc_preview_truncated": true, "_uxc_type": "object"}),
            Value::Array(_) => json!({"_uxc_preview_truncated": true, "_uxc_type": "array"}),
            Value::String(s) => Value::String(truncate_preview_string(s)),
            _ => value.clone(),
        };
    }

    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (idx, (key, val)) in map.iter().enumerate() {
                if idx >= ARTIFACT_PREVIEW_MAX_OBJECT_KEYS {
                    break;
                }
                out.insert(key.clone(), build_preview_value(val, depth + 1));
            }
            if map.len() > ARTIFACT_PREVIEW_MAX_OBJECT_KEYS {
                out.insert("_uxc_preview_truncated".to_string(), Value::Bool(true));
                out.insert(
                    "_uxc_preview_remaining_keys".to_string(),
                    Value::Number(((map.len() - ARTIFACT_PREVIEW_MAX_OBJECT_KEYS) as u64).into()),
                );
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items.iter().take(ARTIFACT_PREVIEW_MAX_ARRAY_ITEMS) {
                out.push(build_preview_value(item, depth + 1));
            }
            if items.len() > ARTIFACT_PREVIEW_MAX_ARRAY_ITEMS {
                out.push(json!({
                    "_uxc_preview_truncated": true,
                    "_uxc_preview_remaining_items": items.len() - ARTIFACT_PREVIEW_MAX_ARRAY_ITEMS
                }));
            }
            Value::Array(out)
        }
        Value::String(s) => Value::String(truncate_preview_string(s)),
        _ => value.clone(),
    }
}

fn truncate_preview_string(input: &str) -> String {
    let mut chars = input.chars();
    let truncated: String = chars
        .by_ref()
        .take(ARTIFACT_PREVIEW_MAX_STRING_CHARS)
        .collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn protocol_from_cached_schema(schema: &Value) -> Option<ProtocolType> {
    if schema
        .get("protocol")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p.eq_ignore_ascii_case("MCP"))
    {
        return Some(ProtocolType::Mcp);
    }

    if schema.get("openapi").is_some() || schema.get("swagger").is_some() {
        return Some(ProtocolType::OpenAPI);
    }

    if schema.get("openrpc").is_some() {
        return Some(ProtocolType::JsonRpc);
    }

    if schema.get("data").and_then(|v| v.get("__schema")).is_some()
        || schema.get("__schema").is_some()
    {
        return Some(ProtocolType::GraphQL);
    }

    if schema
        .get("protocol")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p.eq_ignore_ascii_case("gRPC"))
        || schema.get("services").is_some()
    {
        return Some(ProtocolType::GRpc);
    }

    None
}

fn adapter_from_protocol(protocol: ProtocolType, options: &DetectionOptions) -> AdapterEnum {
    match protocol {
        ProtocolType::OpenAPI => AdapterEnum::OpenAPI(
            adapters::openapi::OpenAPIAdapter::new()
                .with_schema_url_override(options.schema_url.clone())
                .with_timeout(options.request_timeout),
        ),
        ProtocolType::GRpc => AdapterEnum::GRpc(
            adapters::grpc::GrpcAdapter::new().with_timeout(options.request_timeout),
        ),
        ProtocolType::JsonRpc => AdapterEnum::JsonRpc(
            adapters::jsonrpc::JsonRpcAdapter::new()
                .with_schema_url_override(options.schema_url.clone())
                .with_timeout(options.request_timeout),
        ),
        ProtocolType::Mcp => {
            let mut adapter =
                adapters::mcp::McpAdapter::new().with_timeout(options.request_timeout);
            if let Some(spawn_options) = options.stdio_spawn_options.clone() {
                adapter = adapter.with_stdio_spawn_options(spawn_options);
            }
            AdapterEnum::Mcp(adapter)
        }
        ProtocolType::GraphQL => AdapterEnum::GraphQL(
            adapters::graphql::GraphQLAdapter::new().with_timeout(options.request_timeout),
        ),
    }
}

fn inject_cache_if_supported(
    adapter: adapters::AdapterEnum,
    cache: Arc<dyn cache::Cache>,
) -> adapters::AdapterEnum {
    match adapter {
        adapters::AdapterEnum::OpenAPI(a) => adapters::AdapterEnum::OpenAPI(a.with_cache(cache)),
        adapters::AdapterEnum::GraphQL(a) => adapters::AdapterEnum::GraphQL(a.with_cache(cache)),
        adapters::AdapterEnum::GRpc(a) => adapters::AdapterEnum::GRpc(a.with_cache(cache)),
        adapters::AdapterEnum::JsonRpc(a) => adapters::AdapterEnum::JsonRpc(a.with_cache(cache)),
        adapters::AdapterEnum::Mcp(a) => adapters::AdapterEnum::Mcp(a.with_cache(cache)),
    }
}

fn inject_auth_if_supported(
    adapter: adapters::AdapterEnum,
    profile: Option<Profile>,
) -> adapters::AdapterEnum {
    match profile {
        Some(profile) => match adapter {
            adapters::AdapterEnum::OpenAPI(a) => {
                adapters::AdapterEnum::OpenAPI(a.with_auth(profile))
            }
            adapters::AdapterEnum::GraphQL(a) => {
                adapters::AdapterEnum::GraphQL(a.with_auth(profile))
            }
            adapters::AdapterEnum::GRpc(a) => adapters::AdapterEnum::GRpc(a.with_auth(profile)),
            adapters::AdapterEnum::JsonRpc(a) => {
                adapters::AdapterEnum::JsonRpc(a.with_auth(profile))
            }
            adapters::AdapterEnum::Mcp(a) => adapters::AdapterEnum::Mcp(a.with_auth(profile)),
        },
        None => adapter,
    }
}

fn inject_refresh_if_supported(
    adapter: adapters::AdapterEnum,
    refresh_schema: bool,
) -> adapters::AdapterEnum {
    match adapter {
        adapters::AdapterEnum::OpenAPI(a) => {
            adapters::AdapterEnum::OpenAPI(a.with_refresh_schema(refresh_schema))
        }
        adapters::AdapterEnum::GraphQL(a) => {
            adapters::AdapterEnum::GraphQL(a.with_refresh_schema(refresh_schema))
        }
        adapters::AdapterEnum::GRpc(a) => {
            adapters::AdapterEnum::GRpc(a.with_refresh_schema(refresh_schema))
        }
        adapters::AdapterEnum::JsonRpc(a) => {
            adapters::AdapterEnum::JsonRpc(a.with_refresh_schema(refresh_schema))
        }
        adapters::AdapterEnum::Mcp(a) => {
            adapters::AdapterEnum::Mcp(a.with_refresh_schema(refresh_schema))
        }
    }
}

fn inject_timeout_if_supported(
    adapter: adapters::AdapterEnum,
    timeout: Option<Duration>,
) -> adapters::AdapterEnum {
    match adapter {
        adapters::AdapterEnum::OpenAPI(a) => {
            adapters::AdapterEnum::OpenAPI(a.with_timeout(timeout))
        }
        adapters::AdapterEnum::GraphQL(a) => {
            adapters::AdapterEnum::GraphQL(a.with_timeout(timeout))
        }
        adapters::AdapterEnum::GRpc(a) => adapters::AdapterEnum::GRpc(a.with_timeout(timeout)),
        adapters::AdapterEnum::JsonRpc(a) => {
            adapters::AdapterEnum::JsonRpc(a.with_timeout(timeout))
        }
        adapters::AdapterEnum::Mcp(a) => adapters::AdapterEnum::Mcp(a.with_timeout(timeout)),
    }
}

fn inject_request_headers_if_supported(
    adapter: adapters::AdapterEnum,
    request_headers: HashMap<String, String>,
) -> adapters::AdapterEnum {
    if request_headers.is_empty() {
        return adapter;
    }
    match adapter {
        adapters::AdapterEnum::OpenAPI(a) => {
            adapters::AdapterEnum::OpenAPI(a.with_request_headers(request_headers))
        }
        other => other,
    }
}

fn request_timeout_duration(timeout_ms: Option<u64>) -> Option<Duration> {
    timeout_ms
        .and_then(|value| (value > 0).then_some(value))
        .map(Duration::from_millis)
}

fn openapi_runtime_endpoint(request: &RuntimeInvokeRequest) -> Option<String> {
    if !matches!(request.action, RuntimeAction::Execute) {
        return None;
    }

    let operation_id = request.operation_id.as_deref()?;
    let (_, path) = operation_id.split_once(':')?;
    if !path.starts_with('/') {
        return None;
    }

    let endpoint = request.endpoint.trim_end_matches('/');
    let endpoint = adapters::openapi::OpenAPIAdapter::SCHEMA_ENDPOINTS
        .iter()
        .find_map(|suffix| endpoint.strip_suffix(suffix))
        .unwrap_or(endpoint)
        .trim_end_matches('/');

    Some(format!("{}{}", endpoint, path))
}

fn effective_runtime_auth_profile(
    request: &RuntimeInvokeRequest,
    protocol: ProtocolType,
    root_auth_profile: Option<Profile>,
) -> Result<Option<Profile>> {
    if protocol == ProtocolType::OpenAPI {
        if let Some(endpoint) = openapi_runtime_endpoint(request) {
            return auth::resolve_auth_for_endpoint(&endpoint, request.options.auth.clone());
        }
    }

    Ok(root_auth_profile)
}

async fn resolve_adapter_with_schema_cache(
    url: &str,
    detection_options: &DetectionOptions,
    cache: Arc<dyn cache::Cache>,
    auth_profile: Option<Profile>,
    no_cache: bool,
    refresh_schema: bool,
) -> Result<ResolveAdapterResult> {
    if !no_cache && !refresh_schema {
        match cache.get_with_policy(url, cache::CacheReadPolicy::NormalTtl)? {
            cache::CacheLookup::Hit(hit) => {
                if let Some(protocol) = protocol_from_cached_schema(&hit.schema) {
                    let mut adapter = adapter_from_protocol(protocol, detection_options);
                    adapter = inject_cache_if_supported(adapter, cache.clone());
                    adapter = inject_auth_if_supported(adapter, auth_profile.clone());
                    adapter = inject_refresh_if_supported(adapter, refresh_schema);
                    return Ok(ResolveAdapterResult {
                        adapter,
                        cache_meta: Some(SchemaCacheMeta {
                            age_ms: cache_age_ms(hit.fetched_at),
                            stale: hit.stale,
                            fallback: false,
                        }),
                    });
                }
            }
            cache::CacheLookup::Miss | cache::CacheLookup::Bypassed => {}
        }
    }

    let detector = ProtocolDetector::new();
    match detector
        .detect_adapter_with_options(url, detection_options)
        .await
    {
        Ok(mut adapter) => {
            adapter = inject_cache_if_supported(adapter, cache);
            adapter = inject_auth_if_supported(adapter, auth_profile);
            adapter = inject_refresh_if_supported(adapter, refresh_schema);
            Ok(ResolveAdapterResult {
                adapter,
                cache_meta: None,
            })
        }
        Err(err) => {
            if !no_cache && !refresh_schema {
                if let cache::CacheLookup::Hit(hit) =
                    cache.get_with_policy(url, cache::CacheReadPolicy::AllowStale)?
                {
                    if let Some(protocol) = protocol_from_cached_schema(&hit.schema) {
                        let _ = cache.put(url, &hit.schema);
                        let mut adapter = adapter_from_protocol(protocol, detection_options);
                        adapter = inject_cache_if_supported(adapter, cache.clone());
                        adapter = inject_auth_if_supported(adapter, auth_profile.clone());
                        adapter = inject_refresh_if_supported(adapter, refresh_schema);
                        return Ok(ResolveAdapterResult {
                            adapter,
                            cache_meta: Some(SchemaCacheMeta {
                                age_ms: cache_age_ms(hit.fetched_at),
                                stale: hit.stale,
                                fallback: true,
                            }),
                        });
                    }
                }
            }
            Err(err)
        }
    }
}

async fn invoke_with_adapter(
    adapter: &AdapterEnum,
    request: &RuntimeInvokeRequest,
) -> Result<(
    String,
    Option<String>,
    Value,
    Option<adapters::ExecutionMetadata>,
)> {
    match request.action {
        RuntimeAction::HostHelp => {
            let operations = adapter.list_operations(&request.endpoint).await?;
            let protocol = adapter.protocol_type().as_str();
            let summaries = operations
                .iter()
                .map(|op| to_operation_summary(protocol, op))
                .collect::<Vec<_>>();
            let service = host_help_service_summary(adapter, &request.endpoint).await?;
            let mut payload = json!({
                "operations": summaries,
                "count": summaries.len(),
                "examples": host_help_examples(request.options.link_name.as_deref()),
            });
            if let Some(link_name) = request.options.link_name.as_deref() {
                payload["linked_command"] = Value::String(link_name.to_string());
            }
            if let Some(source_skill) = request.options.link_skill.as_deref() {
                payload["source_skill"] = Value::String(source_skill.to_string());
            }
            if let Some(source_docs) = request.options.link_skill_doc.as_deref() {
                payload["source_docs"] = Value::String(source_docs.to_string());
            }
            if let Some(source_path) = request.options.link_skill_path.as_deref() {
                payload["source_path"] = Value::String(source_path.to_string());
            }
            if let Some(service) = service {
                payload["service"] = serde_json::to_value(service)?;
            }
            Ok(("host_help".to_string(), None, payload, None))
        }
        RuntimeAction::CodegenSchema => {
            let operations = adapter.list_operations(&request.endpoint).await?;
            let mut details = HashMap::new();
            for op in &operations {
                let detail = adapter
                    .describe_operation(&request.endpoint, &op.operation_id)
                    .await?;
                details.insert(op.operation_id.clone(), detail);
            }
            let schema = build_codegen_host_schema(
                &request.endpoint,
                adapter.protocol_type().as_str(),
                request.options.link_name.as_deref(),
                &operations,
                &details,
                now_unix_secs(),
            );
            Ok((
                "codegen_host_schema".to_string(),
                None,
                serde_json::to_value(schema)?,
                None,
            ))
        }
        RuntimeAction::OperationHelp => {
            let op = request
                .operation_id
                .as_ref()
                .ok_or_else(|| anyhow!("operation_id is required"))?;
            let detail = adapter.describe_operation(&request.endpoint, op).await?;
            Ok((
                "operation_detail".to_string(),
                Some(op.clone()),
                serde_json::to_value(detail)?,
                None,
            ))
        }
        RuntimeAction::Execute => {
            let op = request
                .operation_id
                .as_ref()
                .ok_or_else(|| anyhow!("operation_id is required"))?;
            let args = prepare_runtime_execute_args(adapter, request).await?;
            let result = adapter.execute(&request.endpoint, op, args).await?;
            Ok((
                "call_result".to_string(),
                Some(op.clone()),
                result.data,
                Some(result.metadata),
            ))
        }
    }
}

async fn invoke_live_stdio_mcp_help(
    runtime: &DaemonRuntime,
    request: &RuntimeInvokeRequest,
    auth_profile: Option<&Profile>,
    cache: Arc<dyn Cache>,
) -> Result<Option<(String, Option<String>, Value)>> {
    if !matches!(
        request.action,
        RuntimeAction::HostHelp | RuntimeAction::CodegenSchema | RuntimeAction::OperationHelp
    ) {
        return Ok(None);
    }

    if !adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint) {
        return Ok(None);
    }

    let session_key =
        stdio_session_key(&request.endpoint, auth_profile, &request.options.inject_env)?;
    let Some(session) = runtime.mcp.get_stdio(&session_key).await else {
        return Ok(None);
    };

    runtime.mcp.mark_stdio_request_started(&session_key).await;
    let mut guard = session.lock().await;
    guard.apply_request_metadata(&StdioSessionRequestMetadata {
        idle_ttl_secs: request.options.daemon_idle_ttl,
        link_name: request.options.link_name.as_deref(),
        link_skill: request.options.link_skill.as_deref(),
        link_skill_doc: request.options.link_skill_doc.as_deref(),
        link_skill_path: request.options.link_skill_path.as_deref(),
        endpoint: &request.endpoint,
        exclusive_keys: &request.options.daemon_exclusive,
    });
    guard.last_used = Instant::now();
    guard.last_used_at_unix = now_unix_secs();
    let _ = guard
        .mark_tools_dirty_from_notifications(&request.endpoint, &cache)
        .await;
    let tools = guard
        .refresh_tools_if_needed(
            &request.endpoint,
            &cache,
            request_timeout_duration(request.options.timeout_ms).unwrap_or_else(
                adapters::mcp::transport::McpStdioTransport::default_request_timeout,
            ),
        )
        .await;
    let recent_stderr = guard.client.recent_stderr_lines(5).await;
    let child_exited = guard.client.child_has_exited().unwrap_or(false);
    let service = live_stdio_service_summary(&guard.client);

    let response = match tools {
        Ok(tools) => match request.action {
            RuntimeAction::HostHelp => {
                let operations = tools
                    .iter()
                    .map(operation_from_mcp_tool)
                    .collect::<Vec<_>>();
                let summaries = operations
                    .iter()
                    .map(|op| to_operation_summary("mcp", op))
                    .collect::<Vec<_>>();
                let mut payload = json!({
                    "operations": summaries,
                    "count": summaries.len(),
                    "examples": host_help_examples(request.options.link_name.as_deref()),
                });
                if let Some(link_name) = request.options.link_name.as_deref() {
                    payload["linked_command"] = Value::String(link_name.to_string());
                }
                if let Some(source_skill) = request.options.link_skill.as_deref() {
                    payload["source_skill"] = Value::String(source_skill.to_string());
                }
                if let Some(source_docs) = request.options.link_skill_doc.as_deref() {
                    payload["source_docs"] = Value::String(source_docs.to_string());
                }
                if let Some(source_path) = request.options.link_skill_path.as_deref() {
                    payload["source_path"] = Value::String(source_path.to_string());
                }
                if let Some(service) = service {
                    payload["service"] = serde_json::to_value(service)?;
                }
                Ok(Some(("host_help".to_string(), None, payload)))
            }
            RuntimeAction::CodegenSchema => {
                let operations = tools
                    .iter()
                    .map(operation_from_mcp_tool)
                    .collect::<Vec<_>>();
                let details = tools
                    .iter()
                    .map(|tool| (tool.name.clone(), operation_detail_from_mcp_tool(tool)))
                    .collect::<HashMap<_, _>>();
                let schema = build_codegen_host_schema(
                    &request.endpoint,
                    "mcp",
                    request.options.link_name.as_deref(),
                    &operations,
                    &details,
                    now_unix_secs(),
                );
                Ok(Some((
                    "codegen_host_schema".to_string(),
                    None,
                    serde_json::to_value(schema)?,
                )))
            }
            RuntimeAction::OperationHelp => {
                let op = request
                    .operation_id
                    .as_ref()
                    .ok_or_else(|| anyhow!("operation_id is required"))?;
                let tool = tools
                    .iter()
                    .find(|tool| tool.name == *op)
                    .ok_or_else(|| UxcError::OperationNotFound(op.clone()))?;
                Ok(Some((
                    "operation_detail".to_string(),
                    Some(op.clone()),
                    serde_json::to_value(operation_detail_from_mcp_tool(tool))?,
                )))
            }
            RuntimeAction::Execute => Ok(None),
        },
        Err(err) => Err(err),
    };
    drop(guard);
    let request_error = response.as_ref().err();
    runtime
        .mcp
        .mark_stdio_request_finished(&session_key, request_error, recent_stderr, child_exited)
        .await;
    response
}

async fn prepare_runtime_execute_args(
    adapter: &AdapterEnum,
    request: &RuntimeInvokeRequest,
) -> Result<HashMap<String, Value>> {
    if !matches!(request.action, RuntimeAction::Execute) {
        return Ok(request.args.clone().unwrap_or_default());
    }

    let op = request
        .operation_id
        .as_ref()
        .ok_or_else(|| anyhow!("operation_id is required"))?;

    prepare_execute_args(
        adapter,
        &request.endpoint,
        op,
        request.args.clone().unwrap_or_default(),
    )
    .await
}

async fn host_help_service_summary(
    adapter: &AdapterEnum,
    endpoint: &str,
) -> Result<Option<ServiceSummary>> {
    if !matches!(adapter.protocol_type(), ProtocolType::Mcp) {
        return Ok(None);
    }

    let schema = adapter.fetch_schema(endpoint).await?;
    let name = schema
        .get("serverInfo")
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let description = schema
        .get("instructions")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    if name.is_none() && description.is_none() {
        return Ok(None);
    }
    Ok(Some(ServiceSummary { name, description }))
}

fn live_stdio_service_summary(client: &adapters::mcp::McpStdioClient) -> Option<ServiceSummary> {
    let name = client.server_info().map(|info| info.name.clone());
    let description = client.instructions().map(ToString::to_string);
    if name.is_none() && description.is_none() {
        return None;
    }
    Some(ServiceSummary { name, description })
}

fn operation_from_mcp_tool(tool: &adapters::mcp::types::Tool) -> Operation {
    Operation {
        operation_id: tool.name.clone(),
        display_name: tool.name.clone(),
        description: Some(tool.description.clone()),
        parameters: tool
            .inputSchema
            .as_ref()
            .map(adapters::mcp::parse_schema_to_parameters_for_daemon)
            .unwrap_or_default(),
        return_type: Some("ToolContent".to_string()),
    }
}

fn operation_detail_from_mcp_tool(tool: &adapters::mcp::types::Tool) -> adapters::OperationDetail {
    adapters::OperationDetail {
        operation_id: tool.name.clone(),
        display_name: tool.name.clone(),
        description: Some(tool.description.clone()),
        parameters: tool
            .inputSchema
            .as_ref()
            .map(adapters::mcp::parse_schema_to_parameters_for_daemon)
            .unwrap_or_default(),
        return_type: Some("ToolContent".to_string()),
        input_schema: tool.inputSchema.clone(),
    }
}

fn to_operation_summary(protocol: &str, op: &Operation) -> OperationSummary {
    let required = op
        .parameters
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.clone())
        .collect::<Vec<_>>();

    let input_shape_hint = if op.parameters.is_empty() {
        "none".to_string()
    } else if required.is_empty() {
        "optional".to_string()
    } else {
        "required".to_string()
    };

    let protocol_kind = match protocol {
        "openapi" => {
            if op.operation_id.contains(':') {
                "http_operation"
            } else {
                "api_operation"
            }
        }
        "graphql" => {
            if op.operation_id.starts_with("query/") {
                "query"
            } else if op.operation_id.starts_with("mutation/") {
                "mutation"
            } else if op.operation_id.starts_with("subscription/") {
                "subscription"
            } else {
                "graphql_operation"
            }
        }
        "jsonrpc" => "rpc_method",
        "grpc" => "rpc_method",
        "mcp" => "tool",
        _ => "operation",
    }
    .to_string();

    OperationSummary {
        operation_id: op.operation_id.clone(),
        display_name: op.display_name.clone(),
        summary: op.description.clone(),
        required,
        input_shape_hint,
        protocol_kind,
    }
}

fn host_help_examples(link_name: Option<&str>) -> Vec<String> {
    if let Some(link_name) = link_name.map(str::trim).filter(|v| !v.is_empty()) {
        return vec![
            format!("{link_name} -h"),
            format!("{link_name} <operation_id> -h"),
            format!("{link_name} <operation_id> id=42"),
            format!("{link_name} <operation_id> '{{...}}'"),
        ];
    }

    vec![
        "uxc <host> -h".to_string(),
        "uxc <host> <operation_id> -h".to_string(),
        "uxc <host> <operation_id> id=42".to_string(),
        "uxc <host> <operation_id> '{...}'".to_string(),
    ]
}

struct SchemaMappingEnvGuard {
    prev: Option<OsString>,
}

impl SchemaMappingEnvGuard {
    fn new(schema_mapping_file: Option<String>) -> Self {
        let prev = std::env::var_os("UXC_SCHEMA_MAPPINGS_FILE");
        match schema_mapping_file {
            Some(path) if !path.is_empty() => std::env::set_var("UXC_SCHEMA_MAPPINGS_FILE", path),
            _ => std::env::remove_var("UXC_SCHEMA_MAPPINGS_FILE"),
        }
        Self { prev }
    }
}

impl Drop for SchemaMappingEnvGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(value) => std::env::set_var("UXC_SCHEMA_MAPPINGS_FILE", value),
            None => std::env::remove_var("UXC_SCHEMA_MAPPINGS_FILE"),
        }
    }
}

fn normalize_http_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn http_endpoint_candidates(url: &str) -> Vec<String> {
    let normalized = normalize_http_url(url);
    let mut candidates = vec![normalized.clone()];

    if let Ok(parsed) = url::Url::parse(&normalized) {
        let path = parsed.path();
        if path.is_empty() || path == "/" {
            candidates.push(format!("{}/mcp", normalized));
            candidates.push(format!("{}/.well-known/mcp", normalized));
        }
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

async fn resolve_mcp_http_endpoint(
    url: &str,
    auth_profile: Option<Profile>,
) -> Result<adapters::mcp::ResolvedMcpHttpTransport> {
    for candidate in http_endpoint_candidates(url) {
        match adapters::mcp::http_transport::McpHttpTransport::probe_initialize_with_reason(
            &candidate,
            auth_profile.clone(),
        )
        .await
        {
            Ok(adapters::mcp::http_transport::ProbeInitializeOutcome::Success(mode)) => {
                return Ok(adapters::mcp::ResolvedMcpHttpTransport::new(
                    mode, candidate,
                ));
            }
            Ok(adapters::mcp::http_transport::ProbeInitializeOutcome::AuthFailed(failure)) => {
                let detail = format!(
                    "MCP authentication probe failed for {}: {}",
                    candidate, failure.message
                );
                return match failure.code {
                    adapters::mcp::http_transport::ProbeAuthFailureCode::OAuthRequired => {
                        Err(UxcError::OAuthRequired(detail).into())
                    }
                    adapters::mcp::http_transport::ProbeAuthFailureCode::OAuthRefreshFailed => {
                        Err(UxcError::OAuthRefreshFailed(detail).into())
                    }
                };
            }
            Ok(adapters::mcp::http_transport::ProbeInitializeOutcome::NotMcp(_)) => {}
            Err(_) => {}
        }
    }

    bail!("Unable to discover MCP HTTP endpoint for {}", url)
}

fn map_runtime_error_code(err: &anyhow::Error) -> i32 {
    if let Some(uxc_err) = err.downcast_ref::<UxcError>() {
        return match uxc_err {
            UxcError::ProtocolDetectionFailed(_) | UxcError::UnsupportedProtocol(_) => {
                ERR_PROTOCOL_DETECTION
            }
            UxcError::InvalidArguments(message)
                if message.starts_with("subscription cursor expired") =>
            {
                ERR_SUBSCRIPTION_CURSOR_EXPIRED
            }
            UxcError::InvalidArguments(_) => -32602,
            UxcError::OperationNotFound(_) => ERR_OPERATION_NOT_FOUND,
            UxcError::OAuthRequired(_) => ERR_OAUTH_REQUIRED,
            UxcError::OAuthRefreshFailed(_) => ERR_OAUTH_REFRESH_FAILED,
            UxcError::OAuthScopeInsufficient(_) => ERR_OAUTH_SCOPE_INSUFFICIENT,
            _ => ERR_RUNTIME_GENERIC,
        };
    }
    ERR_RUNTIME_GENERIC
}

fn jsonrpc_error_from_anyhow(err: &anyhow::Error) -> JsonRpcError {
    JsonRpcError {
        code: map_runtime_error_code(err),
        message: err.to_string(),
        data: structured_error_from_anyhow(err)
            .and_then(|payload: StructuredErrorPayload| serde_json::to_value(payload).ok()),
    }
}

impl Default for DaemonRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use futures::SinkExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;
    use std::time::Duration as StdDuration;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn openapi_runtime_endpoint_appends_operation_path_for_execute() {
        let request = RuntimeInvokeRequest {
            request_id: "req-1".to_string(),
            endpoint: "https://testnet.binance.vision".to_string(),
            action: RuntimeAction::Execute,
            operation_id: Some("get:/api/v3/account".to_string()),
            args: None,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                schema_url: Some("https://example.com/schema.json".to_string()),
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
            },
        };

        assert_eq!(
            openapi_runtime_endpoint(&request).as_deref(),
            Some("https://testnet.binance.vision/api/v3/account")
        );
    }

    #[test]
    fn display_session_key_redacts_raw_endpoint_material() {
        let session_key = "stdio:https://example.com?token=secret:auth:env";
        let display = display_session_key(session_key);
        assert!(display.starts_with("stdio:"));
        assert!(!display.contains("example.com"));
        assert!(!display.contains("secret"));
    }

    #[test]
    fn resolve_stdio_request_metadata_resets_ttl_and_link_name_from_current_request() {
        let (
            idle_ttl_secs,
            link_name,
            link_skill,
            link_skill_doc,
            link_skill_path,
            endpoint,
            daemon_exclusive,
        ) = resolve_stdio_request_metadata(
            &StdioSessionRequestMetadata {
                idle_ttl_secs: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                endpoint: "https://new.example.com",
                exclusive_keys: &[],
            },
            &["/tmp/profile".to_string()],
        );

        assert_eq!(idle_ttl_secs, MCP_IDLE_TTL_SECS);
        assert_eq!(link_name, None);
        assert_eq!(link_skill, None);
        assert_eq!(link_skill_doc, None);
        assert_eq!(link_skill_path, None);
        assert_eq!(endpoint, "https://new.example.com");
        assert_eq!(daemon_exclusive, vec!["/tmp/profile".to_string()]);
    }

    #[test]
    fn resolve_stdio_request_metadata_accepts_zero_ttl_override() {
        let (
            idle_ttl_secs,
            link_name,
            link_skill,
            link_skill_doc,
            link_skill_path,
            endpoint,
            daemon_exclusive,
        ) = resolve_stdio_request_metadata(
            &StdioSessionRequestMetadata {
                idle_ttl_secs: Some(0),
                link_name: Some("board-link"),
                link_skill: Some("board-webmcp"),
                link_skill_doc: Some("https://uxc.holon.run/skills/board-webmcp/"),
                link_skill_path: Some("skills/board-webmcp/SKILL.md"),
                endpoint: "https://new.example.com",
                exclusive_keys: &["/tmp/new-profile".to_string()],
            },
            &[],
        );

        assert_eq!(idle_ttl_secs, 0);
        assert_eq!(link_name, Some("board-link".to_string()));
        assert_eq!(link_skill, Some("board-webmcp".to_string()));
        assert_eq!(
            link_skill_doc,
            Some("https://uxc.holon.run/skills/board-webmcp/".to_string())
        );
        assert_eq!(
            link_skill_path,
            Some("skills/board-webmcp/SKILL.md".to_string())
        );
        assert_eq!(endpoint, "https://new.example.com");
        assert_eq!(daemon_exclusive, vec!["/tmp/new-profile".to_string()]);
    }

    #[test]
    fn openapi_runtime_endpoint_ignores_non_execute_requests() {
        let request = RuntimeInvokeRequest {
            request_id: "req-1".to_string(),
            endpoint: "https://api.binance.com".to_string(),
            action: RuntimeAction::OperationHelp,
            operation_id: Some("post:/api/v3/order".to_string()),
            args: None,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                schema_url: Some("https://example.com/schema.json".to_string()),
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
            },
        };

        assert!(openapi_runtime_endpoint(&request).is_none());
    }

    #[test]
    fn parse_file_sink_requires_file_prefix() {
        let err = parse_file_sink("stdout").unwrap_err();
        assert!(err.to_string().contains("file:<path>"));
    }

    #[test]
    fn parse_file_sink_rejects_parent_relative_path() {
        let err = parse_file_sink("file:../events.ndjson").unwrap_err();
        assert!(err.to_string().contains("cannot contain '..'"));
    }

    #[test]
    fn parse_file_sink_accepts_arbitrary_absolute_path() {
        let path = parse_file_sink("file:/tmp/arbitrary/output.ndjson").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/arbitrary/output.ndjson"));
    }

    #[test]
    fn parse_file_sink_rejects_absolute_path_with_parent_component() {
        let err = parse_file_sink("file:/tmp/../events.ndjson").unwrap_err();
        assert!(err.to_string().contains("cannot contain '..'"));
    }

    #[test]
    fn drain_ndjson_events_parses_complete_lines_and_leaves_partial() {
        let mut buffer = "{\"a\":1}\n{\"b\":2}\n{\"c\":".to_string();
        let values = drain_ndjson_events(&mut buffer).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["a"], 1);
        assert_eq!(values[1]["b"], 2);
        assert_eq!(buffer, "{\"c\":");
    }

    #[test]
    fn drain_sse_json_events_parses_json_payloads() {
        let mut buffer =
            "event: message\ndata: {\"a\":1}\n\n:data-only comment\n\ndata: {\"b\":2}\n\n"
                .to_string();
        let values = drain_sse_json_events(&mut buffer).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["a"], 1);
        assert_eq!(values[1]["b"], 2);
        assert!(buffer.is_empty());
    }

    #[test]
    fn openapi_runtime_endpoint_strips_schema_suffix_before_appending_path() {
        let request = RuntimeInvokeRequest {
            request_id: "req-1".to_string(),
            endpoint: "https://petstore.swagger.io/v2/swagger.json".to_string(),
            action: RuntimeAction::Execute,
            operation_id: Some("post:/pet".to_string()),
            args: None,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
            },
        };

        assert_eq!(
            openapi_runtime_endpoint(&request).as_deref(),
            Some("https://petstore.swagger.io/v2/pet")
        );
    }

    #[test]
    fn openapi_runtime_endpoint_prefers_longest_schema_suffix_match() {
        let request = RuntimeInvokeRequest {
            request_id: "req-1".to_string(),
            endpoint: "https://example.com/swagger/v1/swagger.json".to_string(),
            action: RuntimeAction::Execute,
            operation_id: Some("get:/health".to_string()),
            args: None,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
            },
        };

        assert_eq!(
            openapi_runtime_endpoint(&request).as_deref(),
            Some("https://example.com/health")
        );
    }

    #[test]
    fn drain_ndjson_events_rejects_oversized_buffer() {
        let mut buffer = "x".repeat(SUBSCRIPTION_MAX_BUFFER_BYTES + 1);
        let err = drain_ndjson_events(&mut buffer).unwrap_err();
        assert!(err.to_string().contains("buffer exceeded"));
    }

    #[test]
    fn decode_utf8_prefix_handles_split_multibyte_sequence() {
        let mut buffer = vec![0xE4, 0xBD];
        assert!(decode_utf8_prefix(&mut buffer).unwrap().is_none());
        buffer.push(0xA0);
        assert_eq!(
            decode_utf8_prefix(&mut buffer).unwrap(),
            Some("你".to_string())
        );
        assert!(buffer.is_empty());
    }

    #[derive(Clone)]
    enum TestWsFrame {
        Text(&'static str),
        Binary(Vec<u8>),
    }

    #[derive(Clone)]
    struct TestWsConnectionPlan {
        frames: Vec<TestWsFrame>,
        hold_open_after_send: bool,
    }

    async fn start_test_websocket_server(
        plans: Vec<TestWsConnectionPlan>,
    ) -> (String, StdArc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connects = StdArc::new(AtomicUsize::new(0));
        let connects_clone = connects.clone();
        let task = tokio::spawn(async move {
            for plan in plans {
                let (stream, _) = listener.accept().await.unwrap();
                let connects_for_cb = connects_clone.clone();
                let mut websocket =
                    accept_hdr_async(stream, move |_request: &Request, response: Response| {
                        connects_for_cb.fetch_add(1, Ordering::SeqCst);
                        Ok(response)
                    })
                    .await
                    .unwrap();

                for frame in plan.frames {
                    match frame {
                        TestWsFrame::Text(text) => {
                            websocket
                                .send(Message::Text(text.to_string()))
                                .await
                                .unwrap();
                        }
                        TestWsFrame::Binary(bytes) => {
                            websocket.send(Message::Binary(bytes)).await.unwrap();
                        }
                    }
                    tokio::time::sleep(StdDuration::from_millis(50)).await;
                }

                if plan.hold_open_after_send {
                    tokio::time::sleep(StdDuration::from_secs(2)).await;
                }
                let _ = websocket.close(None).await;
            }
        });

        (format!("ws://{}", addr), connects, task)
    }

    fn subscription_request(endpoint: &str, sink: &str) -> SubscribeStartRequest {
        SubscribeStartRequest {
            request_id: "test-request".to_string(),
            endpoint: endpoint.to_string(),
            sink: sink.to_string(),
            operation_id: None,
            args: None,
            resource_uri: None,
            read_resource: false,
            transport_hint: Some(SubscriptionTransportHint::Websocket),
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
            mode: SubscriptionMode::Stream,
            poll_config: None,
            ephemeral: false,
            internal: false,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
            },
        }
    }

    fn managed_source_spec(endpoint: &str) -> ManagedSourceSpec {
        ManagedSourceSpec {
            endpoint: endpoint.to_string(),
            operation_id: None,
            args: None,
            resource_uri: None,
            read_resource: false,
            transport_hint: Some(SubscriptionTransportHint::Websocket),
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
            mode: SubscriptionMode::Stream,
            poll_config: None,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
            },
        }
    }

    async fn wait_for_file_contains(path: &Path, needle: &str, timeout: StdDuration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                if content.contains(needle) {
                    return true;
                }
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
        false
    }

    fn test_runtime_with_store(temp: &tempfile::TempDir) -> DaemonRuntime {
        DaemonRuntime::try_new_with_subscription_store_path(temp.path().join("subscriptions.json"))
            .expect("test daemon runtime should initialize")
    }

    #[tokio::test]
    async fn daemon_runtime_init_skips_persisted_memory_sink_records() {
        let temp = tempdir().unwrap();
        let store_path = temp.path().join("subscriptions.json");
        let record = PersistedSubscriptionRecord {
            request: subscription_request("https://example.com/stream", "memory:"),
            view: SubscriptionJobView {
                job_id: "sub_1".to_string(),
                mode: SubscriptionMode::Stream,
                endpoint: "https://example.com/stream".to_string(),
                protocol: "websocket".to_string(),
                sink: "memory:".to_string(),
                resource_uri: None,
                status: "running".to_string(),
                durable: false,
                auto_resume: false,
                resume_strategy: "none".to_string(),
                created_at_unix: now_unix_secs(),
                started_at_unix: Some(now_unix_secs()),
                stopped_at_unix: None,
                last_event_at_unix: None,
                last_error: None,
                restart_count: 0,
                last_resume_at_unix: None,
                last_resume_error: None,
                reconnect_count: 0,
                written_events: 0,
            },
        };
        write_subscription_store(&store_path, &[record]).unwrap();

        let runtime = DaemonRuntime::try_new_with_subscription_store_path(store_path.clone())
            .expect("daemon runtime should tolerate memory sink records");
        assert!(runtime.subscribe_list().await.is_empty());

        let raw = std::fs::read_to_string(&store_path).unwrap();
        let store: PersistedSubscriptionStore = serde_json::from_str(&raw).unwrap();
        assert!(store.jobs.is_empty());
    }

    #[tokio::test]
    async fn daemon_runtime_init_skips_persisted_records_with_invalid_file_sink() {
        let temp = tempdir().unwrap();
        let store_path = temp.path().join("subscriptions.json");
        let record = PersistedSubscriptionRecord {
            request: subscription_request("https://example.com/stream", "bad-sink"),
            view: SubscriptionJobView {
                job_id: "sub_2".to_string(),
                mode: SubscriptionMode::Stream,
                endpoint: "https://example.com/stream".to_string(),
                protocol: "websocket".to_string(),
                sink: "bad-sink".to_string(),
                resource_uri: None,
                status: "running".to_string(),
                durable: true,
                auto_resume: true,
                resume_strategy: "reconnect".to_string(),
                created_at_unix: now_unix_secs(),
                started_at_unix: Some(now_unix_secs()),
                stopped_at_unix: None,
                last_event_at_unix: None,
                last_error: None,
                restart_count: 0,
                last_resume_at_unix: None,
                last_resume_error: None,
                reconnect_count: 0,
                written_events: 0,
            },
        };
        write_subscription_store(&store_path, &[record]).unwrap();

        let runtime = DaemonRuntime::try_new_with_subscription_store_path(store_path.clone())
            .expect("daemon runtime should tolerate invalid sink records");
        assert!(runtime.subscribe_list().await.is_empty());

        let raw = std::fs::read_to_string(&store_path).unwrap();
        let store: PersistedSubscriptionStore = serde_json::from_str(&raw).unwrap();
        assert!(store.jobs.is_empty());
    }

    #[tokio::test]
    async fn websocket_subscription_runtime_reconnects_and_stops_cleanly() {
        let temp = tempdir().unwrap();
        let sink_path = temp.path().join("websocket.ndjson");
        let sink_spec = format!("file:{}", sink_path.display());
        let (endpoint, connects, server_task) = start_test_websocket_server(vec![
            TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":1}"#)],
                hold_open_after_send: false,
            },
            TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":2}"#)],
                hold_open_after_send: true,
            },
        ])
        .await;

        let runtime = test_runtime_with_store(&temp);
        let response = runtime
            .subscribe_start(subscription_request(&endpoint, &sink_spec))
            .await
            .unwrap();

        assert_eq!(response.protocol, "websocket");
        assert!(
            wait_for_file_contains(&sink_path, r#""value":2"#, StdDuration::from_secs(5)).await,
            "websocket sink did not receive second event"
        );
        assert!(
            wait_for_file_contains(
                &sink_path,
                r#""event_kind":"reconnect""#,
                StdDuration::from_secs(5)
            )
            .await,
            "websocket sink did not record reconnect"
        );

        let status = runtime.subscribe_status(&response.job_id).await.unwrap();
        assert_eq!(status.protocol, "websocket");
        assert!(status.reconnect_count >= 1);
        assert!(connects.load(Ordering::SeqCst) >= 2);

        let stop = runtime.subscribe_stop(&response.job_id).await.unwrap();
        assert!(stop.stopped);
        assert!(
            wait_for_file_contains(
                &sink_path,
                r#""reason":"stopped""#,
                StdDuration::from_secs(5)
            )
            .await,
            "websocket sink did not record stop"
        );

        server_task.abort();
    }

    #[tokio::test]
    async fn websocket_subscription_runtime_preserves_text_and_binary_frames() {
        let temp = tempdir().unwrap();
        let sink_path = temp.path().join("websocket-frames.ndjson");
        let sink_spec = format!("file:{}", sink_path.display());
        let (endpoint, _connects, server_task) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![
                    TestWsFrame::Text("tick"),
                    TestWsFrame::Binary(vec![1, 2, 3]),
                ],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        let response = runtime
            .subscribe_start(subscription_request(&endpoint, &sink_spec))
            .await
            .unwrap();

        assert!(
            wait_for_file_contains(&sink_path, r#""text":"tick""#, StdDuration::from_secs(5)).await,
            "websocket sink did not record plain text frame"
        );
        assert!(
            wait_for_file_contains(&sink_path, r#""base64":"AQID""#, StdDuration::from_secs(5))
                .await,
            "websocket sink did not record binary frame"
        );

        runtime.subscribe_stop(&response.job_id).await.unwrap();
        server_task.abort();
    }

    #[tokio::test]
    async fn memory_sink_subscription_events_stream_without_user_file() {
        let temp = tempdir().unwrap();
        let (endpoint, _connects, server_task) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":7}"#)],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        let response = runtime
            .subscribe_start(subscription_request(&endpoint, "memory:"))
            .await
            .unwrap();

        let events = runtime
            .subscribe_events(&SubscriptionEventsRequest {
                job_id: response.job_id.clone(),
                after_seq: 0,
                limit: 10,
                wait_ms: 5_000,
            })
            .await
            .unwrap();

        assert_eq!(events.job_id, response.job_id);
        assert!(!events.events.is_empty(), "expected subscription events");
        assert!(events.events.iter().any(|event| event.event_kind == "open"));
        assert!(
            events.events.iter().any(|event| event
                .data
                .as_ref()
                .and_then(|value| value.get("value"))
                == Some(&json!(7))),
            "expected streamed websocket payload"
        );

        runtime.subscribe_stop(&response.job_id).await.unwrap();
        server_task.abort();
    }

    #[tokio::test]
    async fn stopped_subscription_status_and_tail_events_remain_temporarily_available() {
        let temp = tempdir().unwrap();
        let sink_path = temp.path().join("stopped-tail-events.ndjson");
        let sink_spec = format!("file:{}", sink_path.display());
        let (endpoint, _connects, server_task) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":9}"#)],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        let response = runtime
            .subscribe_start(subscription_request(&endpoint, &sink_spec))
            .await
            .unwrap();

        assert!(
            wait_for_file_contains(&sink_path, r#""value":9"#, StdDuration::from_secs(5)).await,
            "expected streamed websocket payload before stop"
        );

        let snapshot = runtime
            .subscribe_events(&SubscriptionEventsRequest {
                job_id: response.job_id.clone(),
                after_seq: 0,
                limit: 100,
                wait_ms: 0,
            })
            .await
            .unwrap();
        let after_seq = snapshot.next_after_seq;

        runtime.subscribe_stop(&response.job_id).await.unwrap();

        let status = runtime.subscribe_status(&response.job_id).await.unwrap();
        assert_eq!(status.status, "stopped");

        let mut saw_closed = false;
        for _ in 0..10 {
            let tail = runtime
                .subscribe_events(&SubscriptionEventsRequest {
                    job_id: response.job_id.clone(),
                    after_seq,
                    limit: 10,
                    wait_ms: 100,
                })
                .await
                .unwrap();
            assert_eq!(tail.status, "stopped");
            if tail.events.iter().any(|event| {
                event.event_kind == "closed"
                    && event.meta.as_ref().and_then(|meta| meta.get("reason"))
                        == Some(&json!("stopped"))
            }) {
                saw_closed = true;
                break;
            }
        }
        assert!(saw_closed, "expected closed event after stop");

        server_task.abort();
    }

    #[tokio::test]
    async fn managed_source_ensure_mirrors_payloads_into_stream() {
        let temp = tempdir().unwrap();
        let (endpoint, _connects, server_task) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":42}"#)],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        let ensured = runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "test".to_string(),
                source_key: "websocket:demo".to_string(),
                spec: managed_source_spec(&endpoint),
            })
            .await
            .unwrap();

        assert_eq!(
            ensured.stream_id,
            managed_stream_id("test", "websocket:demo")
        );
        assert!(runtime.subscribe_list().await.is_empty());

        let mut seen = None;
        for _ in 0..30 {
            let page = runtime
                .stream_read(&ManagedStreamReadRequest {
                    stream_id: ensured.stream_id.clone(),
                    after_offset: 0,
                    limit: 10,
                })
                .await
                .unwrap();
            if let Some(first) = page.events.first() {
                seen = Some(first.raw_payload.clone());
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }

        assert_eq!(seen, Some(json!({"value":42})));

        let stopped = runtime
            .source_stop(&ManagedSourceStatusRequest {
                namespace: "test".to_string(),
                source_key: "websocket:demo".to_string(),
            })
            .await
            .unwrap();
        assert!(stopped.stopped);

        server_task.abort();
    }

    #[tokio::test]
    async fn managed_source_spec_change_replaces_run_but_keeps_stream() {
        let temp = tempdir().unwrap();
        let (endpoint_one, _connects_one, server_task_one) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":"one"}"#)],
                hold_open_after_send: true,
            }])
            .await;
        let (endpoint_two, _connects_two, server_task_two) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":"two"}"#)],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        let first = runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "test".to_string(),
                source_key: "replaceable".to_string(),
                spec: managed_source_spec(&endpoint_one),
            })
            .await
            .unwrap();

        for _ in 0..20 {
            let page = runtime
                .stream_read(&ManagedStreamReadRequest {
                    stream_id: first.stream_id.clone(),
                    after_offset: 0,
                    limit: 10,
                })
                .await
                .unwrap();
            if !page.events.is_empty() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }

        let second = runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "test".to_string(),
                source_key: "replaceable".to_string(),
                spec: managed_source_spec(&endpoint_two),
            })
            .await
            .unwrap();

        assert_eq!(first.stream_id, second.stream_id);
        assert_ne!(first.run_id, second.run_id);
        assert!(second.replaced_previous);

        let mut seen_second = false;
        for _ in 0..30 {
            let page = runtime
                .stream_read(&ManagedStreamReadRequest {
                    stream_id: second.stream_id.clone(),
                    after_offset: 1,
                    limit: 10,
                })
                .await
                .unwrap();
            if page
                .events
                .iter()
                .any(|event| event.raw_payload == json!({"value":"two"}))
            {
                seen_second = true;
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }
        assert!(seen_second);

        runtime
            .source_stop(&ManagedSourceStatusRequest {
                namespace: "test".to_string(),
                source_key: "replaceable".to_string(),
            })
            .await
            .unwrap();
        server_task_one.abort();
        server_task_two.abort();
    }

    #[tokio::test]
    async fn managed_source_delete_keeps_stream_rows() {
        let temp = tempdir().unwrap();
        let (endpoint, _connects, server_task) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":"persisted"}"#)],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        let ensured = runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "test".to_string(),
                source_key: "delete-me".to_string(),
                spec: managed_source_spec(&endpoint),
            })
            .await
            .unwrap();

        for _ in 0..30 {
            let info = runtime.stream_info(&ensured.stream_id).await.unwrap();
            if info.event_count > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }

        runtime
            .source_delete(&ManagedSourceStatusRequest {
                namespace: "test".to_string(),
                source_key: "delete-me".to_string(),
            })
            .await
            .unwrap();

        let page = runtime
            .stream_read(&ManagedStreamReadRequest {
                stream_id: ensured.stream_id.clone(),
                after_offset: 0,
                limit: 10,
            })
            .await
            .unwrap();
        assert!(page
            .events
            .iter()
            .any(|event| event.raw_payload == json!({"value":"persisted"})));

        server_task.abort();
    }
}

#[cfg(unix)]
async fn write_jsonrpc_error(
    stream: &mut UnixStream,
    id: Value,
    code: i32,
    message: String,
) -> Result<()> {
    let resp = JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
    };
    write_frame(stream, &serde_json::to_value(resp)?).await
}

struct StartLockGuard {
    path: PathBuf,
}

impl Drop for StartLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn try_acquire_start_lock(path: &Path) -> Result<Option<StartLockGuard>> {
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
    {
        Ok(_) => Ok(Some(StartLockGuard {
            path: path.to_path_buf(),
        })),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            if lock_is_stale(path, Duration::from_secs(START_LOCK_STALE_SECS))? {
                let _ = fs::remove_file(path);
                return try_acquire_start_lock(path);
            }
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

fn lock_is_stale(path: &Path, max_age: Duration) -> Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let modified = metadata
        .modified()
        .context("Failed reading start.lock mtime")?;
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    Ok(age > max_age)
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}
