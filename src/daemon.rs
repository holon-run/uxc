use crate::adapters::mcp::types::{JsonRpcNotification, ResourceContents};
use crate::adapters::{
    self, Adapter, AdapterEnum, DetectionOptions, Operation, ProtocolDetector, ProtocolType,
};
use crate::arg_coercion::{prepare_execute_args, prepare_execute_args_from_detail};
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
    ManagedSourceListRecord, ManagedSourceRecord, ManagedSourceStore, PendingStreamEvent,
    SourceRuntimeUpdate, StreamEventRecord, StreamInfoRecord,
};
use crate::subscription_discord::{
    derive_gateway_bot_endpoint, parse_gateway_bot_response, prepare_gateway_websocket_url,
    DiscordGatewayBotResponse, DiscordGatewayHandler, DiscordGatewayRuntimeConfig,
    DiscordIdentifyProperties, DISCORD_DEFAULT_MESSAGE_INTENTS,
};
use crate::subscription_email::{
    resolve_email_imap_idle_runtime_config, run_email_imap_idle_subscription_runtime,
    EmailImapIdleRuntimeConfig,
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
use crate::subscription_poll::PollSubscriptionConfig;
use crate::subscription_slack::{
    derive_socket_mode_open_endpoint, parse_socket_mode_open_response, SlackSocketModeHandler,
};
use crate::subscription_websocket::{
    self, RawFrameHandler, WebSocketRunError, WebSocketRuntimeConfig, WebSocketRuntimeObserver,
};
use anyhow::{anyhow, bail, Context, Result};
use fs2::FileExt;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream as StdUnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::task::JoinHandle;

const JSONRPC_VERSION: &str = "2.0";
const START_POLL_TRIES: usize = 30;
const START_POLL_INTERVAL_MS: u64 = 100;
const STOP_POLL_TRIES: usize = 50;
const STOP_POLL_INTERVAL_MS: u64 = 100;
const START_LOCK_STALE_SECS: u64 = 30;
const DAEMON_OWNER_TERM_SIGNAL: i32 = 15;
const STDIO_INIT_LOCK_STALE_SECS: u64 = 30;
const MCP_IDLE_TTL_DEFAULT_SECS: u64 = 3600;
const MCP_IDLE_TTL_ENV: &str = "UXC_DAEMON_MCP_IDLE_TTL_SECS";
const MCP_IDLE_TTL_MAX_SECS: u64 = 24 * 60 * 60;
const MCP_IDLE_CLEANUP_INTERVAL_MS: u64 = 500;
// Five seconds is long enough for cooperative stdio servers to notice stdin EOF
// and release external resources, while still bounding daemon-side eviction stalls.
const MCP_STDIO_EXIT_TIMEOUT_SECS: u64 = 5;
const CONNECT_TIMEOUT_SECS: u64 = 2;
const FRAME_IO_TIMEOUT_SECS: u64 = 120;
const MAX_FRAME_BODY_BYTES: usize = 8 * 1024 * 1024;
const SUBSCRIPTION_HTTP_TIMEOUT_SECS: u64 = 300;
const SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS: u64 = 1;
const SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS: u64 = 30;
const SUBSCRIPTION_MAX_BUFFER_BYTES: usize = 1024 * 1024;
const SUBSCRIPTION_EVENTS_MAX_LIMIT: usize = 500;
const MANAGED_SOURCE_INITIAL_RESTART_DELAY_SECS: u64 = 1;
const MANAGED_SOURCE_MAX_RESTART_DELAY_SECS: u64 = 30;
const MANAGED_STREAM_EVENTS_DEFAULT_LIMIT: usize = 100;
const MANAGED_STREAM_EVENTS_MAX_LIMIT: usize = 500;
const MCP_NOTIFICATION_HISTORY_LIMIT: usize = 256;
const ARTIFACT_COMPACTION_THRESHOLD_BYTES: usize = 64 * 1024;
const ARTIFACT_PREVIEW_MAX_OBJECT_KEYS: usize = 20;
const ARTIFACT_PREVIEW_MAX_ARRAY_ITEMS: usize = 20;
const ARTIFACT_PREVIEW_MAX_STRING_CHARS: usize = 512;
const ARTIFACT_PREVIEW_MAX_DEPTH: usize = 3;
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
    fn kill(pid: i32, sig: i32) -> std::ffi::c_int;
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
    #[serde(default)]
    pub suppress_routine_logs: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_compaction: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionTransportHint {
    Websocket,
    DiscordGateway,
    SlackSocketMode,
    FeishuLongConnection,
    EmailImapIdle,
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
    pub last_success_at_unix: Option<u64>,
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
pub struct ManagedSourceCheckpointSummary {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tie_breaker: Option<Value>,
    #[serde(default)]
    pub seen_window_len: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceStreamSummary {
    pub event_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_at_unix: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SubscriptionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at_unix: Option<u64>,
    #[serde(default)]
    pub reconnect_count: u64,
    #[serde(default)]
    pub written_events: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<ManagedSourceCheckpointSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<ManagedSourceStreamSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceListEntry {
    pub namespace: String,
    pub source_key: String,
    pub status: String,
    pub run_id: String,
    pub stream_id: String,
    pub updated_at_unix: u64,
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
pub struct ManagedSourceDoctorIssue {
    pub severity: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceDoctorResponse {
    pub namespace: String,
    pub source_key: String,
    pub observed_at_unix: u64,
    pub status: String,
    pub runner_active: bool,
    pub stream_exists: bool,
    pub legacy_checkpoint_file_present: bool,
    pub legacy_cursor_file_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_since_last_success: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_since_last_event: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall_threshold_secs: Option<u64>,
    pub source: ManagedSourceView,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ManagedSourceDoctorIssue>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_at_unix: Option<u64>,
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

fn default_managed_stream_events_limit() -> usize {
    MANAGED_STREAM_EVENTS_DEFAULT_LIMIT
}

fn should_read_mcp_resource_snapshot(notification: &JsonRpcNotification) -> bool {
    notification.method == "notifications/resources/updated"
}

async fn append_mcp_resource_snapshot(
    recorder: &mut impl SubscriptionEventRecorder,
    reason: &str,
    resource_contents: ResourceContents,
) -> Result<()> {
    recorder
        .emit(
            "mcp_resource",
            "snapshot",
            Some(serde_json::to_value(resource_contents)?),
            Some(json!({ "reason": reason })),
        )
        .await
}

async fn append_mcp_resource_read_result(
    recorder: &mut impl SubscriptionEventRecorder,
    resource_uri: &str,
    reason: &str,
    error_context: &str,
    read_result: Result<ResourceContents>,
) -> Result<()> {
    match read_result {
        Ok(contents) => append_mcp_resource_snapshot(recorder, reason, contents).await,
        Err(err) => {
            let msg = format!("{}: {}", error_context, err);
            recorder
                .emit(
                    "mcp_resource",
                    "error",
                    None,
                    Some(json!({ "message": msg, "resource_uri": resource_uri })),
                )
                .await?;
            recorder.update_status(None, Some(msg), false).await?;
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
    #[serde(default)]
    pub managed_sources: usize,
    #[serde(default)]
    pub managed_sources_running: usize,
    #[serde(default)]
    pub managed_streams: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_lock_held: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_pid_alive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_socket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_started_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket_exists: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonOwnerMetadata {
    pid: u32,
    version: String,
    socket: String,
    started_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonLocalDiagnostics {
    pub socket: String,
    pub socket_exists: bool,
    pub owner_lock_held: bool,
    pub owner_pid: Option<u32>,
    pub owner_pid_alive: bool,
    pub owner_version: Option<String>,
    pub owner_socket: Option<String>,
    pub owner_started_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonDoctorResponse {
    pub status: String,
    pub repaired: bool,
    pub socket_removed: bool,
    pub owner_metadata_cleared: bool,
    pub socket: String,
    pub diagnostics: DaemonLocalDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_contract: Option<LifecycleContractView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_lifecycle_update_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_lifecycle_snapshot: Option<LifecycleSnapshotView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_summary: Option<String>,
    #[serde(default)]
    pub recent_stderr: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSessionKillRequest {
    pub session_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSessionKillResponse {
    pub session_key: String,
    pub child_pid: Option<u32>,
    pub killed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleContractView {
    pub reap_policy: adapters::mcp::LifecycleReapPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleSnapshotView {
    pub auto_reap_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleContractFetchState {
    Unsupported,
    Available,
    Unknown,
}

fn lifecycle_contract_fetch_state(err: &anyhow::Error) -> LifecycleContractFetchState {
    let jsonrpc_code = structured_error_from_anyhow(err)
        .and_then(|payload| payload.details)
        .and_then(|details| details.get("jsonrpc_code").and_then(Value::as_i64));
    if jsonrpc_code == Some(-32601) {
        LifecycleContractFetchState::Unsupported
    } else {
        LifecycleContractFetchState::Unknown
    }
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
    reuse_eligible: bool,
    lifecycle_contract: Option<LifecycleContractView>,
    lifecycle_contract_fetch_state: LifecycleContractFetchState,
    last_lifecycle_update_at_unix: Option<u64>,
    last_lifecycle_snapshot: Option<LifecycleSnapshotView>,
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
    lifecycle_contract: Option<LifecycleContractView>,
    lifecycle_contract_fetch_state: LifecycleContractFetchState,
    last_lifecycle_update_at_unix: Option<u64>,
    last_lifecycle_snapshot: Option<LifecycleSnapshotView>,
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

struct ResolvedStdioRequestMetadata {
    idle_ttl_secs: u64,
    link_name: Option<String>,
    link_skill: Option<String>,
    link_skill_doc: Option<String>,
    link_skill_path: Option<String>,
    endpoint: String,
    daemon_exclusive: Vec<String>,
}

fn default_mcp_idle_ttl_secs() -> u64 {
    std::env::var(MCP_IDLE_TTL_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|ttl| *ttl > 0)
        .map(|ttl| ttl.min(MCP_IDLE_TTL_MAX_SECS))
        .unwrap_or(MCP_IDLE_TTL_DEFAULT_SECS)
}

fn instant_cutoff(now: Instant, age_secs: u64) -> Option<Instant> {
    now.checked_sub(Duration::from_secs(age_secs))
}

fn resolve_stdio_request_metadata(
    metadata: &StdioSessionRequestMetadata<'_>,
    existing_exclusive_keys: &[String],
) -> ResolvedStdioRequestMetadata {
    let daemon_exclusive = if metadata.exclusive_keys.is_empty() {
        existing_exclusive_keys.to_vec()
    } else {
        metadata.exclusive_keys.to_vec()
    };
    ResolvedStdioRequestMetadata {
        idle_ttl_secs: metadata
            .idle_ttl_secs
            .unwrap_or_else(default_mcp_idle_ttl_secs),
        link_name: metadata.link_name.map(str::to_string),
        link_skill: metadata.link_skill.map(str::to_string),
        link_skill_doc: metadata.link_skill_doc.map(str::to_string),
        link_skill_path: metadata.link_skill_path.map(str::to_string),
        endpoint: metadata.endpoint.to_string(),
        daemon_exclusive,
    }
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
    runtime_view: Mutex<Option<Arc<Mutex<SubscriptionJobView>>>>,
    stop_tx: Mutex<watch::Sender<bool>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedSourceRuntimeStateSnapshot {
    status: String,
    started_at_unix: Option<u64>,
    stopped_at_unix: Option<u64>,
    last_error: Option<String>,
    last_success_at_unix: Option<u64>,
    last_event_at_unix: Option<u64>,
    reconnect_count: u64,
    written_events: u64,
}

impl McpStdioSession {
    fn apply_lifecycle_notification(&mut self, notification: &JsonRpcNotification) -> Result<bool> {
        if notification.method != "notifications/uxc.lifecycle_changed" {
            return Ok(false);
        }
        let params = notification.params.clone().unwrap_or_else(|| json!({}));
        let snapshot: LifecycleSnapshotView =
            serde_json::from_value(params).context("Failed to parse lifecycle snapshot")?;
        self.last_lifecycle_update_at_unix = Some(snapshot.updated_at_unix);
        self.last_lifecycle_snapshot = Some(snapshot);
        Ok(true)
    }

    fn apply_request_metadata(&mut self, metadata: &StdioSessionRequestMetadata<'_>) {
        let resolved = resolve_stdio_request_metadata(metadata, &self.daemon_exclusive);
        self.idle_ttl_secs = resolved.idle_ttl_secs;
        self.link_name = resolved.link_name;
        self.link_skill = resolved.link_skill;
        self.link_skill_doc = resolved.link_skill_doc;
        self.link_skill_path = resolved.link_skill_path;
        self.endpoint = resolved.endpoint;
        self.daemon_exclusive = resolved.daemon_exclusive;
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
        let mut public_notifications = Vec::new();
        for notification in notifications {
            if let Err(err) = self.apply_lifecycle_notification(&notification) {
                tracing::warn!(endpoint = %endpoint, error = %err, "Failed to apply MCP lifecycle notification");
                continue;
            }
            if notification.method == "notifications/uxc.lifecycle_changed" {
                continue;
            }
            if notification.method == "notifications/tools/list_changed" {
                self.tools_dirty = true;
                if let Some(cache) = cache {
                    let _ = cache.invalidate(endpoint);
                }
            }
            public_notifications.push(notification);
        }
        self.notifications.extend(public_notifications.clone());
        public_notifications
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
            lifecycle_contract: self.lifecycle_contract.clone(),
            last_lifecycle_update_at_unix: self.last_lifecycle_update_at_unix,
            last_lifecycle_snapshot: self.last_lifecycle_snapshot.clone(),
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
            "lifecycle_contract": snapshot.lifecycle_contract.clone(),
            "last_lifecycle_update_at_unix": snapshot.last_lifecycle_update_at_unix,
            "last_lifecycle_snapshot": snapshot.last_lifecycle_snapshot.clone(),
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
        let default_idle_ttl_secs = default_mcp_idle_ttl_secs();
        let now = Instant::now();
        let http_cutoff = instant_cutoff(now, default_idle_ttl_secs);
        let stdio_entries: Vec<(String, Arc<Mutex<McpStdioSession>>)> = {
            let map = self.stdio.lock().await;
            map.iter().map(|(k, s)| (k.clone(), s.clone())).collect()
        };
        let mut stdio_remove = Vec::new();
        for (key, session) in &stdio_entries {
            // Use try_lock to avoid blocking on sessions that may be held across .await in invoke_mcp.
            // If a session is busy, we'll check it again in the next cleanup cycle.
            if let Ok(mut guard) = session.try_lock() {
                let endpoint = guard.endpoint.clone();
                let _ = guard.sync_notifications(&endpoint, None).await;
                if guard.idle_ttl_secs == 0 {
                    continue;
                }
                let should_reap = instant_cutoff(Instant::now(), guard.idle_ttl_secs)
                    .is_some_and(|cutoff| guard.last_used < cutoff);
                if should_reap {
                    let lifecycle_allows_reap = match (
                        guard.lifecycle_contract_fetch_state,
                        guard
                            .lifecycle_contract
                            .as_ref()
                            .map(|contract| contract.reap_policy),
                    ) {
                        (_, Some(adapters::mcp::LifecycleReapPolicy::SafeIdleReap)) => true,
                        (LifecycleContractFetchState::Unsupported, None) => true,
                        (
                            LifecycleContractFetchState::Unsupported
                            | LifecycleContractFetchState::Available,
                            Some(adapters::mcp::LifecycleReapPolicy::Stateful),
                        ) => guard
                            .last_lifecycle_snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.auto_reap_allowed),
                        (LifecycleContractFetchState::Unknown, _) => false,
                        (LifecycleContractFetchState::Available, None) => false,
                    };
                    if !lifecycle_allows_reap {
                        continue;
                    }
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

        let now = Instant::now();
        if let Some(init_lock_cutoff) = instant_cutoff(now, STDIO_INIT_LOCK_STALE_SECS) {
            let mut lock_map = self.stdio_init_locks.lock().await;
            // Retain locks that are:
            // 1. Still in use (strong_count > 1 means someone is holding the lock), or
            // 2. Were touched recently (not stale)
            // This avoids dropping an init lock during an ongoing initialization,
            // which could otherwise allow a concurrent cold call to create a duplicate
            // lock and spawn another MCP process, breaking the singleflight guarantee.
            lock_map
                .retain(|_, v| Arc::strong_count(&v.lock) > 1 || v.touched_at >= init_lock_cutoff);

            let mut exclusive_lock_map = self.stdio_exclusive_locks.lock().await;
            exclusive_lock_map
                .retain(|_, v| Arc::strong_count(&v.lock) > 1 || v.touched_at >= init_lock_cutoff);
        }

        let mut http_remove = Vec::new();
        if let Some(http_cutoff) = http_cutoff {
            let http_entries: Vec<(String, Arc<McpHttpSession>)> = {
                let map = self.http.lock().await;
                map.iter().map(|(k, s)| (k.clone(), s.clone())).collect()
            };
            for (key, session) in &http_entries {
                let last_used = *session.last_used.lock().await;
                if last_used < http_cutoff {
                    http_remove.push(key.clone());
                }
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

        let mut client = adapters::mcp::McpStdioClient::connect_with_options_and_timeout(
            command,
            args,
            spawn_options.clone(),
            request_timeout.unwrap_or_else(
                adapters::mcp::transport::McpStdioTransport::default_request_timeout,
            ),
        )
        .await?;
        let (lifecycle_contract, lifecycle_contract_fetch_state) = match client
            .lifecycle_contract(request_timeout.unwrap_or_else(
                adapters::mcp::transport::McpStdioTransport::default_request_timeout,
            ))
            .await
        {
            Ok(contract) => (
                Some(LifecycleContractView {
                    reap_policy: contract.reap_policy,
                }),
                LifecycleContractFetchState::Available,
            ),
            Err(err) => {
                let fetch_state = lifecycle_contract_fetch_state(&err);
                match fetch_state {
                    LifecycleContractFetchState::Unsupported => {
                        tracing::debug!(
                            session_key = %session_key,
                            error = %err,
                            "MCP stdio child does not declare lifecycle contract"
                        );
                    }
                    LifecycleContractFetchState::Unknown => {
                        tracing::warn!(
                            session_key = %session_key,
                            error = %err,
                            "Failed to fetch MCP stdio lifecycle contract; automatic idle reap will stay disabled"
                        );
                    }
                    LifecycleContractFetchState::Available => {}
                }
                (None, fetch_state)
            }
        };
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
            idle_ttl_secs: metadata
                .idle_ttl_secs
                .unwrap_or_else(default_mcp_idle_ttl_secs),
            link_name: metadata.link_name.map(str::to_string),
            link_skill: metadata.link_skill.map(str::to_string),
            link_skill_doc: metadata.link_skill_doc.map(str::to_string),
            link_skill_path: metadata.link_skill_path.map(str::to_string),
            endpoint: metadata.endpoint.to_string(),
            daemon_exclusive: exclusive_keys.clone(),
            reuse_eligible: true,
            lifecycle_contract: lifecycle_contract.clone(),
            lifecycle_contract_fetch_state,
            last_lifecycle_update_at_unix: None,
            last_lifecycle_snapshot: None,
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
                idle_ttl_secs: metadata
                    .idle_ttl_secs
                    .unwrap_or_else(default_mcp_idle_ttl_secs),
                daemon_exclusive: exclusive_keys.clone(),
                in_flight_requests: 0,
                reuse_eligible: true,
                lifecycle_contract,
                lifecycle_contract_fetch_state,
                last_lifecycle_update_at_unix: None,
                last_lifecycle_snapshot: None,
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
        if !guard.reuse_eligible {
            let display_key = display_session_key(session_key);
            return Err(UxcError::InvalidArguments(format!(
                "Daemon-backed MCP session {} is unavailable after a failed explicit kill; retry `uxc daemon session kill {}` or run `uxc daemon stop`.",
                display_key, display_key
            ))
            .into());
        }
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
        let reuse_eligible = guard.reuse_eligible;
        let lifecycle_contract = guard.lifecycle_contract.clone();
        let lifecycle_contract_fetch_state = guard.lifecycle_contract_fetch_state;
        let last_lifecycle_update_at_unix = guard.last_lifecycle_update_at_unix;
        let last_lifecycle_snapshot = guard.last_lifecycle_snapshot.clone();
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
            snapshot.reuse_eligible = reuse_eligible;
            snapshot.lifecycle_contract = lifecycle_contract;
            snapshot.lifecycle_contract_fetch_state = lifecycle_contract_fetch_state;
            snapshot.last_lifecycle_update_at_unix = last_lifecycle_update_at_unix;
            snapshot.last_lifecycle_snapshot = last_lifecycle_snapshot;
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

                    let (owner_endpoint, owner_auth_fp, owner_env_fp, _owner_cwd) =
                        parse_stdio_session_key(&owner_session_key);
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
        transport.initialized().await?;
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
                let endpoint = guard.endpoint.clone();
                let _ = guard.sync_notifications(&endpoint, None).await;
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
                    snapshot.lifecycle_contract = guard.lifecycle_contract.clone();
                    snapshot.lifecycle_contract_fetch_state = guard.lifecycle_contract_fetch_state;
                    snapshot.last_lifecycle_update_at_unix = guard.last_lifecycle_update_at_unix;
                    snapshot.last_lifecycle_snapshot = guard.last_lifecycle_snapshot.clone();
                    snapshot.recent_stderr = recent_stderr;
                    snapshot.reuse_eligible = guard.reuse_eligible;
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

    async fn resolve_stdio_session_key(&self, requested_session_key: &str) -> Result<String> {
        let map = self.stdio.lock().await;
        let matches = map
            .keys()
            .filter(|session_key| {
                session_key.as_str() == requested_session_key
                    || display_session_key(session_key) == requested_session_key
            })
            .cloned()
            .collect::<Vec<_>>();
        drop(map);

        match matches.as_slice() {
            [session_key] => Ok(session_key.clone()),
            [] => Err(UxcError::OperationNotFound(format!(
                "daemon session not found: {}",
                requested_session_key
            ))
            .into()),
            _ => Err(UxcError::InvalidArguments(format!(
                "daemon session identifier is ambiguous: {}",
                requested_session_key
            ))
            .into()),
        }
    }

    async fn kill_stdio_session(
        &self,
        requested_session_key: &str,
    ) -> Result<DaemonSessionKillResponse> {
        let session_key = self
            .resolve_stdio_session_key(requested_session_key)
            .await?;
        let session = {
            let map = self.stdio.lock().await;
            map.get(&session_key).cloned()
        }
        .ok_or_else(|| {
            UxcError::OperationNotFound(format!(
                "daemon session not found: {}",
                requested_session_key
            ))
        })?;

        let mut guard = session.lock().await;
        let child_pid = guard.child_pid;
        guard.reuse_eligible = false;
        let recent_stderr = redact_recent_stderr(guard.client.recent_stderr_lines(5).await);
        self.upsert_stdio_snapshot(&session_key, |snapshot| {
            snapshot.reuse_eligible = false;
        })
        .await;
        let kill_result = guard
            .client
            .kill_and_wait(Duration::from_secs(MCP_STDIO_EXIT_TIMEOUT_SECS))
            .await;
        drop(guard);

        match kill_result {
            Ok(()) => {
                {
                    let mut map = self.stdio.lock().await;
                    map.remove(&session_key);
                }
                self.cleanup_stdio_exclusive_for_session_key(&session_key)
                    .await;
                self.remove_stdio_snapshot(&session_key, "explicit_killed", None)
                    .await;
                Ok(DaemonSessionKillResponse {
                    session_key: display_session_key(&session_key),
                    child_pid,
                    killed: true,
                })
            }
            Err(err) => {
                let error_message = format!("failed to kill daemon session: {}", err);
                self.upsert_stdio_snapshot(&session_key, |snapshot| {
                    snapshot.reuse_eligible = false;
                    snapshot.last_error_summary = Some(error_message.clone());
                    snapshot.recent_stderr = recent_stderr.clone();
                })
                .await;
                Err(err.context(format!(
                    "Failed to kill daemon session {}",
                    display_session_key(&session_key)
                )))
            }
        }
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
            SubscriptionTransportHint::EmailImapIdle => {
                if !lower.starts_with("imap://") && !lower.starts_with("imaps://") {
                    bail!("email-imap-idle transport requires an imap:// or imaps:// endpoint");
                }
                return Ok("email_imap_idle".to_string());
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

    bail!("subscription execution requires an http(s) endpoint or --resource-uri for MCP subscriptions")
}

const MANAGED_SOURCE_POLL_JITTER_MAX_MS: u64 = 30_000;

fn stable_managed_source_hash(namespace: &str, source_key: &str, poll_round: u64) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in namespace.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash ^= 0;
    hash = hash.wrapping_mul(FNV_PRIME);
    for byte in source_key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    for byte in poll_round.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn managed_source_poll_jitter_window_ms(interval_secs: u64) -> u64 {
    let interval_ms = interval_secs.max(1).saturating_mul(1000);
    (interval_ms / 4).clamp(1, MANAGED_SOURCE_POLL_JITTER_MAX_MS)
}

fn managed_source_initial_poll_delay_duration(
    namespace: &str,
    source_key: &str,
    interval_secs: u64,
) -> Duration {
    let jitter_window_ms = managed_source_poll_jitter_window_ms(interval_secs);
    Duration::from_millis(stable_managed_source_hash(namespace, source_key, 0) % jitter_window_ms)
}

fn managed_source_poll_wait_duration(
    namespace: &str,
    source_key: &str,
    interval_secs: u64,
    poll_round: u64,
) -> Duration {
    let interval_ms = interval_secs.max(1).saturating_mul(1000);
    let jitter_window_ms = managed_source_poll_jitter_window_ms(interval_secs);
    let half_window_ms = jitter_window_ms / 2;
    let jitter_ms =
        stable_managed_source_hash(namespace, source_key, poll_round) % (jitter_window_ms + 1);
    let wait_ms = if jitter_ms >= half_window_ms {
        interval_ms.saturating_add(jitter_ms - half_window_ms)
    } else {
        interval_ms.saturating_sub(half_window_ms - jitter_ms)
    };
    Duration::from_millis(wait_ms.max(1))
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
        let existing = self
            .store
            .get_source(&request.namespace, &request.source_key)
            .await?;

        let mut replaced_previous = false;
        let record = if let Some(existing) = existing {
            if existing.spec_key == spec_key {
                self.ensure_runner_for_record(runtime, existing.clone(), request.spec.clone())
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
            } else {
                let _ = self
                    .stop_internal(runtime, &request.namespace, &request.source_key)
                    .await;
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
                    last_success_at_unix: None,
                    last_event_at_unix: None,
                    reconnect_count: 0,
                    written_events: 0,
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
                last_success_at_unix: None,
                last_event_at_unix: None,
                reconnect_count: 0,
                written_events: 0,
            }
        };

        self.start_managed_source(runtime, record, request.spec.clone())
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
        self.build_status_view(&record).await
    }

    async fn list(&self) -> Result<Vec<ManagedSourceListEntry>> {
        let records = self.store.load_source_list_records().await?;
        Ok(records.iter().map(list_entry_from_list_record).collect())
    }

    async fn summary(&self) -> Result<(usize, usize, usize)> {
        let summary = self.store.summary().await?;
        Ok((
            summary.source_count,
            summary.running_source_count,
            summary.stream_count,
        ))
    }

    async fn build_status_view(&self, record: &ManagedSourceRecord) -> Result<ManagedSourceView> {
        let mut view = view_from_record(record);
        let identity_key = managed_source_identity_key(&record.namespace, &record.source_key);
        let active_entry = { self.entries.lock().await.get(&identity_key).cloned() };
        if let Some(entry) = active_entry {
            let current = entry.state.lock().await.clone();
            view.status = current.status;
            view.updated_at_unix = current.updated_at_unix;
            view.started_at_unix = current.started_at_unix;
            view.stopped_at_unix = current.stopped_at_unix;
            view.last_error = current.last_error;
            view.last_success_at_unix = current.last_success_at_unix;
            view.last_event_at_unix = current.last_event_at_unix;
            view.reconnect_count = current.reconnect_count;
            view.written_events = current.written_events;
            if let Some(runtime_view) = entry.runtime_view.lock().await.clone() {
                let snapshot = runtime_view.lock().await.clone();
                view.status = snapshot.status;
                view.started_at_unix = snapshot.started_at_unix;
                view.stopped_at_unix = snapshot.stopped_at_unix;
                view.last_error = snapshot.last_error;
                view.last_success_at_unix = snapshot.last_success_at_unix;
                view.last_event_at_unix = snapshot.last_event_at_unix;
                view.reconnect_count = snapshot.reconnect_count;
                view.written_events = snapshot.written_events;
            }
        }

        enrich_managed_source_view_from_spec(&mut view, &record.spec_json);
        if let Some(stream_info) = self.store.stream_info(&record.stream_id).await? {
            let latest_event_at_unix = stream_info.latest_event_at_unix;
            view.stream = Some(ManagedSourceStreamSummary {
                event_count: stream_info.event_count,
                earliest_offset: stream_info.earliest_offset,
                latest_offset: stream_info.latest_offset,
                latest_event_at_unix,
            });
            if let Some(latest_event_at_unix) = latest_event_at_unix {
                view.last_event_at_unix = view
                    .last_event_at_unix
                    .map(|current| current.max(latest_event_at_unix))
                    .or(Some(latest_event_at_unix));
            }
        }
        if view.mode == Some(SubscriptionMode::Poll) {
            if let Some(checkpoint) = self
                .store
                .load_poll_checkpoint(&record.namespace, &record.source_key, &record.run_id)
                .await?
            {
                view.checkpoint = Some(checkpoint_summary_from_state(
                    managed_source_checkpoint_kind(&record.spec_json),
                    checkpoint,
                ));
            }
        }

        Ok(view)
    }

    async fn doctor(
        &self,
        runtime: &DaemonRuntime,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceDoctorResponse> {
        validate_managed_source_identity(&request.namespace, &request.source_key)?;
        let record = self
            .store
            .get_source(&request.namespace, &request.source_key)
            .await?
            .ok_or_else(|| {
                UxcError::OperationNotFound(format!(
                    "managed source not found: {}/{}",
                    request.namespace, request.source_key
                ))
            })?;
        let source = self.build_status_view(&record).await?;
        let runner_active = self
            .entries
            .lock()
            .await
            .contains_key(&managed_source_identity_key(
                &request.namespace,
                &request.source_key,
            ));
        let stream_exists = self.store.stream_info(&record.stream_id).await?.is_some();
        let legacy_checkpoint_file_present = runtime
            .managed_source_checkpoint_path(&record.run_id)
            .exists();
        let legacy_cursor_file_present =
            runtime.managed_source_cursor_path(&record.run_id).exists();
        let observed_at_unix = now_unix_secs();
        let seconds_since_last_success = source
            .last_success_at_unix
            .map(|ts| observed_at_unix.saturating_sub(ts));
        let seconds_since_last_event = source
            .last_event_at_unix
            .map(|ts| observed_at_unix.saturating_sub(ts));
        let stall_threshold_secs = source
            .poll_interval_secs
            .map(|interval| interval.saturating_mul(3).max(60))
            .or_else(|| {
                matches!(
                    source.status.as_str(),
                    "running" | "reconnecting" | "starting"
                )
                .then_some(300)
            });

        let mut issues = Vec::new();
        if !stream_exists {
            issues.push(ManagedSourceDoctorIssue {
                severity: "error".to_string(),
                code: "stream_missing".to_string(),
                message: format!(
                    "Managed source {} / {} points to missing stream {}",
                    request.namespace, request.source_key, record.stream_id
                ),
            });
        }
        if matches!(
            source.status.as_str(),
            "running" | "reconnecting" | "starting"
        ) && !runner_active
        {
            issues.push(ManagedSourceDoctorIssue {
                severity: "warn".to_string(),
                code: "runner_inactive".to_string(),
                message: "Source record reports an active runtime state, but no live runner is registered in the daemon.".to_string(),
            });
        }
        if legacy_checkpoint_file_present {
            issues.push(ManagedSourceDoctorIssue {
                severity: "warn".to_string(),
                code: "legacy_checkpoint_file_present".to_string(),
                message:
                    "A legacy managed source checkpoint file is still present for the current run."
                        .to_string(),
            });
        }
        if legacy_cursor_file_present {
            issues.push(ManagedSourceDoctorIssue {
                severity: "warn".to_string(),
                code: "legacy_cursor_file_present".to_string(),
                message:
                    "A legacy managed source cursor file is still present for the current run."
                        .to_string(),
            });
        }
        if source.mode == Some(SubscriptionMode::Poll)
            && source
                .stream
                .as_ref()
                .is_some_and(|stream| stream.event_count > 0)
            && source.checkpoint.is_none()
            && !legacy_checkpoint_file_present
        {
            issues.push(ManagedSourceDoctorIssue {
                severity: "warn".to_string(),
                code: "checkpoint_missing".to_string(),
                message: "Poll source has durable events but no persisted checkpoint summary."
                    .to_string(),
            });
        }
        if matches!(source.status.as_str(), "running" | "reconnecting")
            && stall_threshold_secs.is_some()
        {
            let progress_age = seconds_since_last_success
                .or(seconds_since_last_event)
                .or_else(|| {
                    source
                        .started_at_unix
                        .map(|ts| observed_at_unix.saturating_sub(ts))
                });
            if let (Some(age), Some(threshold)) = (progress_age, stall_threshold_secs) {
                if age > threshold {
                    issues.push(ManagedSourceDoctorIssue {
                        severity: "warn".to_string(),
                        code: "stalled".to_string(),
                        message: format!(
                            "Source has been {} seconds without observed progress; threshold is {} seconds.",
                            age, threshold
                        ),
                    });
                }
            }
        }

        let status = if issues.iter().any(|issue| issue.severity == "error") {
            "error"
        } else if issues.iter().any(|issue| issue.severity == "warn") {
            "warn"
        } else {
            "healthy"
        }
        .to_string();

        Ok(ManagedSourceDoctorResponse {
            namespace: request.namespace.clone(),
            source_key: request.source_key.clone(),
            observed_at_unix,
            status,
            runner_active,
            stream_exists,
            legacy_checkpoint_file_present,
            legacy_cursor_file_present,
            seconds_since_last_success,
            seconds_since_last_event,
            stall_threshold_secs,
            source,
            issues,
        })
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
            let spec: ManagedSourceSpec = match serde_json::from_value(record.spec_json.clone()) {
                Ok(spec) => spec,
                Err(err) => {
                    tracing::warn!(
                        "failed to decode managed source spec for {}/{}: {}",
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
                    continue;
                }
            };
            if let Err(err) = self
                .ensure_runner_for_record(runtime, record.clone(), spec)
                .await
            {
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
        reset_managed_source_runtime_files(runtime, &record.run_id).await?;
        record.status = "starting".to_string();
        record.updated_at_unix = now_unix_secs();
        record.started_at_unix = Some(now_unix_secs());
        record.stopped_at_unix = None;
        record.last_error = None;
        self.store.upsert_source(&record, true).await?;
        self.ensure_runner_for_record(runtime, record.clone(), spec)
            .await?;
        Ok(record)
    }

    async fn ensure_runner_for_record(
        &self,
        runtime: &DaemonRuntime,
        record: ManagedSourceRecord,
        spec: ManagedSourceSpec,
    ) -> Result<()> {
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
            runtime_view: Mutex::new(None),
            stop_tx: Mutex::new(stop_tx),
            task: Mutex::new(None),
        });
        self.entries
            .lock()
            .await
            .insert(identity_key.clone(), entry.clone());
        let manager = self.clone();
        let runtime = runtime.clone();
        let runner_entry = entry.clone();
        let task = tokio::spawn(async move {
            manager
                .run_managed_source(runtime, identity_key, runner_entry, record, spec, stop_rx)
                .await;
        });
        *entry.task.lock().await = Some(task);
        Ok(())
    }

    async fn run_managed_source(
        &self,
        runtime: DaemonRuntime,
        identity_key: String,
        entry: Arc<ManagedSourceEntry>,
        record: ManagedSourceRecord,
        spec: ManagedSourceSpec,
        stop_rx: watch::Receiver<bool>,
    ) {
        let request = managed_source_subscription_request(&record, &spec);
        let runtime_view = Arc::new(Mutex::new(subscription_view_for_managed_source(
            &record, &request,
        )));
        *entry.runtime_view.lock().await = Some(runtime_view.clone());
        if spec.mode == SubscriptionMode::Poll {
            self.run_managed_source_poll(
                &runtime,
                &entry,
                &record,
                &request,
                &runtime_view,
                stop_rx,
            )
            .await;
            *entry.runtime_view.lock().await = None;
            self.entries.lock().await.remove(&identity_key);
            return;
        }

        let mut restart_delay_secs = MANAGED_SOURCE_INITIAL_RESTART_DELAY_SECS;
        loop {
            let (event_tx, mut event_rx) = mpsc::channel(SUBSCRIPTION_EVENTS_MAX_LIMIT);
            let runner_task = spawn_managed_source_stream_task(
                runtime.clone(),
                &record.run_id,
                request.clone(),
                runtime_view.clone(),
                stop_rx.clone(),
                event_tx,
            );
            let mut last_synced_state: Option<ManagedSourceRuntimeStateSnapshot> = None;

            loop {
                let should_sync = tokio::select! {
                    maybe_event = event_rx.recv() => {
                        match maybe_event {
                            Some(event) => {
                                if matches!(event.event_kind.as_str(), "data" | "snapshot") {
                                    let payload = event.data.as_ref().or(event.meta.as_ref());
                                    if let Some(payload) = payload {
                                        if let Err(err) = self.store.append_event(&entry.stream_id, event.timestamp_unix, payload).await {
                                            self.fail_managed_source(&entry, err.to_string()).await;
                                            let _ = entry.stop_tx.lock().await.send(true);
                                            break;
                                        }
                                    }
                                }
                                true
                            }
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        let fallback_status = runtime_view.lock().await.status.clone();
                        let next_state =
                            managed_source_runtime_state_snapshot(&runtime_view, &fallback_status)
                                .await;
                        last_synced_state.as_ref() != Some(&next_state)
                    }
                };

                if should_sync {
                    let fallback_status = runtime_view.lock().await.status.clone();
                    if let Err(err) = sync_managed_source_state(
                        &self.store,
                        &entry,
                        &runtime_view,
                        &fallback_status,
                    )
                    .await
                    {
                        self.fail_managed_source(&entry, err.to_string()).await;
                        let _ = entry.stop_tx.lock().await.send(true);
                        break;
                    }
                    last_synced_state = Some(
                        managed_source_runtime_state_snapshot(&runtime_view, &fallback_status)
                            .await,
                    );
                    if fallback_status == "running" {
                        restart_delay_secs = MANAGED_SOURCE_INITIAL_RESTART_DELAY_SECS;
                    }
                }
            }

            let _ = runner_task.await;
            let final_status = runtime_view.lock().await.status.clone();
            if let Err(err) =
                sync_managed_source_state(&self.store, &entry, &runtime_view, &final_status).await
            {
                self.fail_managed_source(&entry, err.to_string()).await;
                let _ = entry.stop_tx.lock().await.send(true);
            }

            if *stop_rx.borrow() || final_status == "stopped" {
                break;
            }

            {
                let mut guard = runtime_view.lock().await;
                guard.status = "reconnecting".to_string();
                guard.restart_count = guard.restart_count.saturating_add(1);
                guard.stopped_at_unix = None;
                guard.last_error.get_or_insert_with(|| {
                    "managed source runner exited unexpectedly; restarting".to_string()
                });
            }
            let fallback_status = runtime_view.lock().await.status.clone();
            if let Err(err) =
                sync_managed_source_state(&self.store, &entry, &runtime_view, &fallback_status)
                    .await
            {
                self.fail_managed_source(&entry, err.to_string()).await;
                let _ = entry.stop_tx.lock().await.send(true);
                break;
            }
            let mut restart_stop_rx = stop_rx.clone();
            if wait_for_stop_or_timeout(
                &mut restart_stop_rx,
                Duration::from_secs(restart_delay_secs),
            )
            .await
            {
                let mut guard = runtime_view.lock().await;
                guard.status = "stopped".to_string();
                guard.stopped_at_unix = Some(now_unix_secs());
                let _ =
                    sync_managed_source_state(&self.store, &entry, &runtime_view, "stopped").await;
                break;
            }
            restart_delay_secs =
                (restart_delay_secs.saturating_mul(2)).min(MANAGED_SOURCE_MAX_RESTART_DELAY_SECS);
        }
        *entry.runtime_view.lock().await = None;
        self.entries.lock().await.remove(&identity_key);
    }

    async fn run_managed_source_poll(
        &self,
        runtime: &DaemonRuntime,
        entry: &Arc<ManagedSourceEntry>,
        record: &ManagedSourceRecord,
        request: &SubscribeStartRequest,
        runtime_view: &Arc<Mutex<SubscriptionJobView>>,
        mut stop_rx: watch::Receiver<bool>,
    ) {
        let config = match resolve_poll_subscription_config(request) {
            Ok(config) => config,
            Err(err) => {
                self.fail_managed_source(entry, err.to_string()).await;
                return;
            }
        };
        let mut runner = match crate::subscription_poll::PollSubscriptionRunner::new(config.clone())
        {
            Ok(runner) => runner,
            Err(err) => {
                self.fail_managed_source(entry, err.to_string()).await;
                return;
            }
        };
        match self
            .load_or_import_legacy_managed_source_checkpoint(runtime, record)
            .await
        {
            Ok(Some(checkpoint)) => runner.restore_checkpoint(checkpoint),
            Ok(None) => {}
            Err(err) => {
                self.fail_managed_source(entry, err.to_string()).await;
                return;
            }
        }
        update_subscription_view(runtime_view, Some("running"), None, false).await;

        let default_interval_secs = config.interval_secs.max(1);
        let base_args = request.args.clone().unwrap_or_default();
        let initial_delay = managed_source_initial_poll_delay_duration(
            &entry.namespace,
            &entry.source_key,
            default_interval_secs,
        );
        if !initial_delay.is_zero() && wait_for_stop_or_timeout(&mut stop_rx, initial_delay).await {
            update_subscription_view(runtime_view, Some("stopped"), None, false).await;
            let snapshot = runtime_view.lock().await.clone();
            let _ =
                sync_managed_source_state(&self.store, entry, runtime_view, &snapshot.status).await;
            return;
        }
        let mut poll_round = 0_u64;
        loop {
            let poll_started = Instant::now();
            if *stop_rx.borrow() {
                update_subscription_view(runtime_view, Some("stopped"), None, false).await;
                let snapshot = runtime_view.lock().await.clone();
                let _ =
                    sync_managed_source_state(&self.store, entry, runtime_view, &snapshot.status)
                        .await;
                return;
            }

            let args = runner.build_request_args(&base_args);
            let previous_checkpoint = runner.checkpoint().clone();
            let fetch =
                fetch_managed_source_poll(runtime, request, args, runner.checkpoint()).await;
            let next_interval_secs;

            match fetch {
                Ok(result) => {
                    let fetch_duration_ms = result.duration_ms;
                    let status_code = result.status_code;
                    if let Some(etag) = crate::subscription_poll::extract_header_value(
                        &result.response_headers,
                        "etag",
                    ) {
                        let mut checkpoint = runner.checkpoint().clone();
                        checkpoint.etag = Some(etag.to_string());
                        runner.restore_checkpoint(checkpoint);
                    }
                    next_interval_secs = crate::subscription_poll::parse_poll_interval_secs(
                        &result.response_headers,
                    )
                    .unwrap_or(default_interval_secs);

                    if result.status_code == Some(304) {
                        note_subscription_success(runtime_view, now_unix_secs()).await;
                        update_subscription_view(runtime_view, Some("running"), None, false).await;
                        let mut db_write_duration_ms = 0;
                        if runner.checkpoint() != &previous_checkpoint {
                            let db_write_started = Instant::now();
                            if let Err(err) = self
                                .store
                                .append_events_and_store_poll_checkpoint(
                                    &entry.namespace,
                                    &entry.source_key,
                                    &record.run_id,
                                    &entry.stream_id,
                                    &[],
                                    runner.checkpoint(),
                                )
                                .await
                            {
                                self.fail_managed_source(entry, err.to_string()).await;
                                return;
                            }
                            db_write_duration_ms = db_write_started.elapsed().as_millis() as u64;
                        }
                        log_managed_source_poll_summary(
                            runtime,
                            entry,
                            ManagedSourcePollSummaryLog {
                                status_code,
                                fetch_duration_ms,
                                process_duration_ms: 0,
                                db_write_duration_ms,
                                event_count: 0,
                                total_duration_ms: poll_started.elapsed().as_millis() as u64,
                                error: None,
                            },
                        )
                        .await;
                    } else {
                        let process_started = Instant::now();
                        let output = match runner.process_response(result.data, result.duration_ms)
                        {
                            Ok(output) => output,
                            Err(err) => {
                                self.fail_managed_source(entry, err.to_string()).await;
                                return;
                            }
                        };
                        let process_duration_ms = process_started.elapsed().as_millis() as u64;
                        let ingested_at_unix = now_unix_secs();
                        note_subscription_success(runtime_view, ingested_at_unix).await;
                        update_subscription_view(runtime_view, Some("running"), None, false).await;
                        let events = output
                            .emitted_items
                            .into_iter()
                            .map(|payload| PendingStreamEvent {
                                ingested_at_unix,
                                payload,
                            })
                            .collect::<Vec<_>>();
                        let event_count = events.len();
                        let mut db_write_duration_ms = 0;
                        if !events.is_empty() || runner.checkpoint() != &previous_checkpoint {
                            let db_write_started = Instant::now();
                            if let Err(err) = self
                                .store
                                .append_events_and_store_poll_checkpoint(
                                    &entry.namespace,
                                    &entry.source_key,
                                    &record.run_id,
                                    &entry.stream_id,
                                    &events,
                                    runner.checkpoint(),
                                )
                                .await
                            {
                                self.fail_managed_source(entry, err.to_string()).await;
                                return;
                            }
                            db_write_duration_ms = db_write_started.elapsed().as_millis() as u64;
                            if !events.is_empty() {
                                note_subscription_events_written(
                                    runtime_view,
                                    ingested_at_unix,
                                    event_count as u64,
                                )
                                .await;
                            }
                        }
                        log_managed_source_poll_summary(
                            runtime,
                            entry,
                            ManagedSourcePollSummaryLog {
                                status_code,
                                fetch_duration_ms,
                                process_duration_ms,
                                db_write_duration_ms,
                                event_count,
                                total_duration_ms: poll_started.elapsed().as_millis() as u64,
                                error: None,
                            },
                        )
                        .await;
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    log_managed_source_poll_summary(
                        runtime,
                        entry,
                        ManagedSourcePollSummaryLog {
                            status_code: None,
                            fetch_duration_ms: None,
                            process_duration_ms: 0,
                            db_write_duration_ms: 0,
                            event_count: 0,
                            total_duration_ms: poll_started.elapsed().as_millis() as u64,
                            error: Some(message.clone()),
                        },
                    )
                    .await;
                    update_subscription_view(
                        runtime_view,
                        Some("reconnecting"),
                        Some(message.clone()),
                        true,
                    )
                    .await;
                    if let Err(sync_err) =
                        sync_managed_source_state(&self.store, entry, runtime_view, "reconnecting")
                            .await
                    {
                        self.fail_managed_source(entry, sync_err.to_string()).await;
                        return;
                    }
                    if wait_for_stop_or_timeout(
                        &mut stop_rx,
                        managed_source_poll_wait_duration(
                            &entry.namespace,
                            &entry.source_key,
                            default_interval_secs,
                            poll_round,
                        ),
                    )
                    .await
                    {
                        update_subscription_view(runtime_view, Some("stopped"), None, false).await;
                        let snapshot = runtime_view.lock().await.clone();
                        let _ = sync_managed_source_state(
                            &self.store,
                            entry,
                            runtime_view,
                            &snapshot.status,
                        )
                        .await;
                        return;
                    }
                    poll_round = poll_round.wrapping_add(1);
                    continue;
                }
            }

            let fallback_status = runtime_view.lock().await.status.clone();
            if let Err(err) =
                sync_managed_source_state(&self.store, entry, runtime_view, &fallback_status).await
            {
                self.fail_managed_source(entry, err.to_string()).await;
                return;
            }

            if wait_for_stop_or_timeout(
                &mut stop_rx,
                managed_source_poll_wait_duration(
                    &entry.namespace,
                    &entry.source_key,
                    next_interval_secs,
                    poll_round,
                ),
            )
            .await
            {
                update_subscription_view(runtime_view, Some("stopped"), None, false).await;
                let snapshot = runtime_view.lock().await.clone();
                let _ =
                    sync_managed_source_state(&self.store, entry, runtime_view, &snapshot.status)
                        .await;
                return;
            }
            poll_round = poll_round.wrapping_add(1);
        }
    }

    async fn load_or_import_legacy_managed_source_checkpoint(
        &self,
        runtime: &DaemonRuntime,
        record: &ManagedSourceRecord,
    ) -> Result<Option<crate::subscription_poll::PollCheckpointState>> {
        if let Some(checkpoint) = self
            .store
            .load_poll_checkpoint(&record.namespace, &record.source_key, &record.run_id)
            .await?
        {
            return Ok(Some(checkpoint));
        }

        let checkpoint_path = runtime.managed_source_checkpoint_path(&record.run_id);
        match tokio::fs::read(&checkpoint_path).await {
            Ok(bytes) => {
                let checkpoint = serde_json::from_slice::<
                    crate::subscription_poll::PollCheckpointState,
                >(&bytes)?;
                let imported = self
                    .store
                    .store_poll_checkpoint_if_missing(
                        &record.namespace,
                        &record.source_key,
                        &record.run_id,
                        &checkpoint,
                    )
                    .await?;
                if imported {
                    let _ = tokio::fs::remove_file(&checkpoint_path).await;
                }
                Ok(Some(checkpoint))
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| {
                format!(
                    "Failed to read legacy checkpoint {}",
                    checkpoint_path.display()
                )
            }),
        }
    }

    async fn fail_managed_source(&self, entry: &Arc<ManagedSourceEntry>, message: String) {
        let now = now_unix_secs();
        let _ = self
            .store
            .clear_source_job(
                &entry.namespace,
                &entry.source_key,
                "failed",
                now,
                Some(now),
                Some(message.clone()),
            )
            .await;
        let mut state = entry.state.lock().await;
        state.status = "failed".to_string();
        state.updated_at_unix = now;
        state.stopped_at_unix = Some(now);
        state.last_error = Some(message);
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
            if let Some(task) = entry.task.lock().await.take() {
                let task = task;
                let _ = task.await;
            }
            self.entries.lock().await.remove(&identity_key);
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
        self.store
            .clear_poll_checkpoint(namespace, source_key, &stored.run_id)
            .await?;
        let _ = tokio::fs::remove_file(runtime.managed_source_sink_path(&stored.run_id)).await;
        let _ =
            tokio::fs::remove_file(runtime.managed_source_checkpoint_path(&stored.run_id)).await;
        let _ = tokio::fs::remove_file(runtime.managed_source_cursor_path(&stored.run_id)).await;
        Ok(())
    }
}

struct ManagedSourcePollSummaryLog {
    status_code: Option<u16>,
    fetch_duration_ms: Option<u64>,
    process_duration_ms: u64,
    db_write_duration_ms: u64,
    event_count: usize,
    total_duration_ms: u64,
    error: Option<String>,
}

async fn log_managed_source_poll_summary(
    runtime: &DaemonRuntime,
    entry: &ManagedSourceEntry,
    summary: ManagedSourcePollSummaryLog,
) {
    let mut log_entry = DaemonLogEntry::new(DaemonEventType::ManagedSourcePollSummary)
        .with_request_id(format!(
            "managed-source:{}:{}",
            entry.namespace, entry.source_key
        ))
        .with_meta(json!({
            "namespace": entry.namespace,
            "source_key": entry.source_key,
            "stream_id": entry.stream_id,
            "status_code": summary.status_code,
            "fetch_duration_ms": summary.fetch_duration_ms,
            "process_duration_ms": summary.process_duration_ms,
            "db_write_duration_ms": summary.db_write_duration_ms,
            "event_count": summary.event_count,
            "total_duration_ms": summary.total_duration_ms,
        }));
    if let Some(error) = summary.error {
        log_entry = log_entry.with_error(error);
    }
    runtime.log(log_entry).await;
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

fn managed_source_checkpoint_kind(spec_json: &Value) -> String {
    serde_json::from_value::<ManagedSourceSpec>(spec_json.clone())
        .ok()
        .and_then(|spec| spec.poll_config)
        .and_then(|value| {
            serde_json::from_value::<crate::subscription_poll::PollSubscriptionConfig>(value).ok()
        })
        .map(|config| match config.checkpoint_strategy {
            crate::subscription_poll::PollCheckpointStrategy::CursorOnly => {
                "cursor_only".to_string()
            }
            crate::subscription_poll::PollCheckpointStrategy::ItemKey { .. } => {
                "item_key".to_string()
            }
            crate::subscription_poll::PollCheckpointStrategy::Watermark { .. } => {
                "watermark".to_string()
            }
            crate::subscription_poll::PollCheckpointStrategy::ContentHash { .. } => {
                "content_hash".to_string()
            }
        })
        .unwrap_or_else(|| "poll".to_string())
}

fn checkpoint_summary_from_state(
    kind: String,
    checkpoint: crate::subscription_poll::PollCheckpointState,
) -> ManagedSourceCheckpointSummary {
    ManagedSourceCheckpointSummary {
        kind,
        cursor: checkpoint.cursor,
        watermark: checkpoint.watermark,
        tie_breaker: checkpoint.tie_breaker,
        seen_window_len: checkpoint.seen_keys.len(),
        etag: checkpoint.etag,
    }
}

fn enrich_managed_source_view_from_spec(view: &mut ManagedSourceView, spec_json: &Value) {
    let Ok(spec) = serde_json::from_value::<ManagedSourceSpec>(spec_json.clone()) else {
        return;
    };
    view.mode = Some(spec.mode);
    view.endpoint = Some(redact_endpoint(&spec.endpoint));
    view.operation_id = spec.operation_id;
    view.resource_uri = spec.resource_uri;
    view.poll_interval_secs = spec
        .poll_config
        .and_then(|value| {
            serde_json::from_value::<crate::subscription_poll::PollSubscriptionConfig>(value).ok()
        })
        .map(|config| config.interval_secs);
}

fn managed_source_streams_db_path(base_dir: &Path) -> PathBuf {
    base_dir.join("managed-source-streams.db")
}

fn managed_source_sink_path(base_dir: &Path, run_id: &str) -> PathBuf {
    base_dir
        .join("managed-source-sinks")
        .join(format!("{run_id}.ndjson"))
}

fn managed_source_checkpoint_path(base_dir: &Path, run_id: &str) -> PathBuf {
    base_dir
        .join("managed-source-checkpoints")
        .join(format!("{run_id}.checkpoint.json"))
}

fn managed_source_cursor_path(base_dir: &Path, run_id: &str) -> PathBuf {
    base_dir
        .join("managed-source-cursors")
        .join(format!("{run_id}.cursor.json"))
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
        "schema_url": spec.options.schema_url,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{:x}", digest))
}

fn managed_source_subscription_request(
    record: &ManagedSourceRecord,
    spec: &ManagedSourceSpec,
) -> SubscribeStartRequest {
    SubscribeStartRequest {
        request_id: format!("managed-source:{}:{}", record.namespace, record.source_key),
        endpoint: spec.endpoint.clone(),
        sink: "memory:".to_string(),
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

fn subscription_view_for_managed_source(
    record: &ManagedSourceRecord,
    request: &SubscribeStartRequest,
) -> SubscriptionJobView {
    let protocol = match request.mode {
        SubscriptionMode::Stream => {
            resolve_stream_subscription_protocol(request).unwrap_or_else(|_| "stream".to_string())
        }
        SubscriptionMode::Poll => "poll".to_string(),
    };
    SubscriptionJobView {
        job_id: format!("managed-source-{}", record.run_id),
        mode: request.mode,
        endpoint: request.endpoint.clone(),
        protocol,
        sink: request.sink.clone(),
        resource_uri: request.resource_uri.clone(),
        status: "starting".to_string(),
        durable: true,
        auto_resume: false,
        resume_strategy: "managed_source".to_string(),
        created_at_unix: record.created_at_unix,
        started_at_unix: record.started_at_unix,
        stopped_at_unix: record.stopped_at_unix,
        last_event_at_unix: record.last_event_at_unix,
        last_error: record.last_error.clone(),
        restart_count: 0,
        last_resume_at_unix: None,
        last_resume_error: None,
        reconnect_count: record.reconnect_count,
        written_events: record.written_events,
        last_success_at_unix: record.last_success_at_unix,
    }
}

async fn reset_managed_source_runtime_files(runtime: &DaemonRuntime, run_id: &str) -> Result<()> {
    let _ = tokio::fs::remove_file(runtime.managed_source_sink_path(run_id)).await;
    let _ = tokio::fs::remove_file(runtime.managed_source_checkpoint_path(run_id)).await;
    let _ = tokio::fs::remove_file(runtime.managed_source_cursor_path(run_id)).await;
    Ok(())
}

fn spawn_managed_source_stream_task(
    runtime: DaemonRuntime,
    run_id: &str,
    request: SubscribeStartRequest,
    view: Arc<Mutex<SubscriptionJobView>>,
    stop_rx: watch::Receiver<bool>,
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
) -> JoinHandle<()> {
    let run_id = run_id.to_string();
    tokio::spawn(async move {
        let result = run_stream_subscription_job(
            &runtime,
            &run_id,
            &request,
            event_tx,
            view.clone(),
            stop_rx,
        )
        .await;

        let mut guard = view.lock().await;
        if guard.status != "stopped" {
            match result {
                Ok(()) => guard.status = "stopped".to_string(),
                Err(err) => {
                    guard.status = "failed".to_string();
                    guard.last_error = Some(err.to_string());
                }
            }
        }
        guard.stopped_at_unix = Some(now_unix_secs());
    })
}

async fn sync_managed_source_state(
    store: &ManagedSourceStore,
    entry: &Arc<ManagedSourceEntry>,
    runtime_view: &Arc<Mutex<SubscriptionJobView>>,
    fallback_status: &str,
) -> Result<()> {
    let snapshot = runtime_view.lock().await.clone();
    let now = now_unix_secs();
    let status = if snapshot.status.is_empty() {
        fallback_status.to_string()
    } else {
        snapshot.status.clone()
    };
    let stopped_at_unix = if status == "running" || status == "reconnecting" || status == "starting"
    {
        None
    } else {
        snapshot.stopped_at_unix.or(Some(now))
    };
    store
        .update_source_runtime(
            &entry.namespace,
            &entry.source_key,
            SourceRuntimeUpdate {
                status: status.clone(),
                updated_at_unix: now,
                started_at_unix: snapshot.started_at_unix,
                stopped_at_unix,
                last_error: snapshot.last_error.clone(),
                last_success_at_unix: snapshot.last_success_at_unix,
                last_event_at_unix: snapshot.last_event_at_unix,
                reconnect_count: snapshot.reconnect_count,
                written_events: snapshot.written_events,
            },
        )
        .await?;
    let mut state = entry.state.lock().await;
    state.status = status;
    state.updated_at_unix = now;
    state.started_at_unix = snapshot.started_at_unix;
    state.stopped_at_unix = stopped_at_unix;
    state.last_error = snapshot.last_error;
    state.last_success_at_unix = snapshot.last_success_at_unix;
    state.last_event_at_unix = snapshot.last_event_at_unix;
    state.reconnect_count = snapshot.reconnect_count;
    state.written_events = snapshot.written_events;
    Ok(())
}

async fn managed_source_runtime_state_snapshot(
    runtime_view: &Arc<Mutex<SubscriptionJobView>>,
    fallback_status: &str,
) -> ManagedSourceRuntimeStateSnapshot {
    let snapshot = runtime_view.lock().await.clone();
    let status = if snapshot.status.is_empty() {
        fallback_status.to_string()
    } else {
        snapshot.status
    };
    let stopped_at_unix = if status == "running" || status == "reconnecting" || status == "starting"
    {
        None
    } else {
        snapshot.stopped_at_unix
    };
    ManagedSourceRuntimeStateSnapshot {
        status,
        started_at_unix: snapshot.started_at_unix,
        stopped_at_unix,
        last_error: snapshot.last_error,
        last_success_at_unix: snapshot.last_success_at_unix,
        last_event_at_unix: snapshot.last_event_at_unix,
        reconnect_count: snapshot.reconnect_count,
        written_events: snapshot.written_events,
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
        mode: None,
        endpoint: None,
        operation_id: None,
        resource_uri: None,
        poll_interval_secs: None,
        last_success_at_unix: record.last_success_at_unix,
        last_event_at_unix: record.last_event_at_unix,
        reconnect_count: record.reconnect_count,
        written_events: record.written_events,
        checkpoint: None,
        stream: None,
    }
}

fn list_entry_from_list_record(record: &ManagedSourceListRecord) -> ManagedSourceListEntry {
    ManagedSourceListEntry {
        namespace: record.namespace.clone(),
        source_key: record.source_key.clone(),
        status: record.status.clone(),
        run_id: record.run_id.clone(),
        stream_id: record.stream_id.clone(),
        updated_at_unix: record.updated_at_unix,
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
        latest_event_at_unix: record.latest_event_at_unix,
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
        Self::try_new_with_managed_source_base_dir(daemon_dir())
    }

    fn try_new_with_managed_source_base_dir(managed_source_base_dir: PathBuf) -> Result<Self> {
        let logger = Self::initialize_logger();
        let managed_source_store =
            ManagedSourceStore::new(managed_source_streams_db_path(&managed_source_base_dir))?;
        Ok(Self {
            state: Arc::new(Mutex::new(ServerState {
                started_at_unix: now_unix_secs(),
                request_count: 0,
            })),
            mcp: McpSessionManager::new(logger.clone()),
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

    async fn flush_logs(&self) {
        if let Some(ref logger) = self.logger {
            if let Err(e) = logger.flush().await {
                tracing::debug!("Failed to flush daemon log: {}", e);
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
        {
            let mut st = self.state.lock().await;
            st.request_count = st.request_count.saturating_add(1);
        }

        let start = Instant::now();
        let log_routine_events = !request.suppress_routine_logs;

        // Log runtime invoke start
        if log_routine_events {
            self.log(
                DaemonLogEntry::new(DaemonEventType::RuntimeInvokeStart)
                    .with_request_id(request.request_id.clone())
                    .with_endpoint(request.endpoint.clone())
                    .with_operation_id(request.operation_id.clone().unwrap_or_default()),
            )
            .await;
        }

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
            if reused && log_routine_events {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::DaemonSessionReused)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone()),
                )
                .await;
            }
            if log_routine_events {
                self.log(
                    DaemonLogEntry::new(DaemonEventType::RuntimeInvokeSuccess)
                        .with_request_id(request.request_id.clone())
                        .with_endpoint(request.endpoint.clone())
                        .with_operation_id(request.operation_id.clone().unwrap_or_default())
                        .with_protocol("mcp".to_string())
                        .with_duration_ms(duration_ms),
                )
                .await;
            }
            let mut response_meta = RuntimeMeta {
                schema_involved: Some(true),
                daemon_session_reused: Some(reused),
                ..Default::default()
            };
            let mut response_data = data;
            apply_runtime_artifact_compaction(
                &kind,
                &mut response_data,
                &mut response_meta,
                request.options.artifact_compaction.unwrap_or(true),
            )?;
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
            } else if log_routine_events {
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
            let prepared_args = if adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint) {
                request.args.clone().unwrap_or_default()
            } else {
                prepare_runtime_execute_args(&resolved.adapter, &request).await?
            };
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

            if reused && log_routine_events {
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
                                if adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint) {
                                    request.args.clone().unwrap_or_default()
                                } else {
                                    prepare_runtime_execute_args(&adapter, &request).await?
                                };
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
                apply_runtime_artifact_compaction(
                    &kind,
                    &mut data,
                    &mut meta,
                    request.options.artifact_compaction.unwrap_or(true),
                )?;
                if log_routine_events {
                    self.log(
                        DaemonLogEntry::new(DaemonEventType::RuntimeInvokeSuccess)
                            .with_request_id(request.request_id.clone())
                            .with_endpoint(request.endpoint.clone())
                            .with_operation_id(request.operation_id.clone().unwrap_or_default())
                            .with_protocol(protocol.clone())
                            .with_duration_ms(duration_ms),
                    )
                    .await;
                }

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
        let (managed_sources, managed_sources_running, managed_streams) =
            match self.managed_sources.summary().await {
                Ok(summary) => summary,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "Failed to summarize managed sources for daemon status"
                    );
                    (0, 0, 0)
                }
            };
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
            managed_sources,
            managed_sources_running,
            managed_streams,
            log_file,
            owner_lock_held: Some(true),
            owner_pid: Some(std::process::id()),
            owner_pid_alive: Some(true),
            owner_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            owner_socket: Some(socket_path().display().to_string()),
            owner_started_at_unix: Some(state.started_at_unix),
            socket_exists: Some(socket_path().exists()),
        }
    }

    pub async fn session_views(&self) -> Vec<DaemonSessionView> {
        self.mcp.session_views().await
    }

    pub async fn session_kill(
        &self,
        request: &DaemonSessionKillRequest,
    ) -> Result<DaemonSessionKillResponse> {
        self.mcp.kill_stdio_session(&request.session_key).await
    }

    pub async fn source_list(&self) -> Result<Vec<ManagedSourceListEntry>> {
        self.managed_sources.list().await
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

    pub async fn source_doctor(
        &self,
        request: &ManagedSourceStatusRequest,
    ) -> Result<ManagedSourceDoctorResponse> {
        self.managed_sources.doctor(self, request).await
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

    fn managed_source_checkpoint_path(&self, run_id: &str) -> PathBuf {
        managed_source_checkpoint_path(&self.managed_source_base_dir, run_id)
    }

    fn managed_source_cursor_path(&self, run_id: &str) -> PathBuf {
        managed_source_cursor_path(&self.managed_source_base_dir, run_id)
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
                request.options.cwd.as_deref(),
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
        raw_args: HashMap<String, Value>,
        auth_profile: Option<Profile>,
        precomputed_stdio_spawn_options: Option<adapters::mcp::StdioSpawnOptions>,
        cache: Arc<dyn Cache>,
    ) -> Result<(String, Option<String>, Value, bool)> {
        let endpoint = &request.endpoint;
        let op = request
            .operation_id
            .as_ref()
            .ok_or_else(|| anyhow!("operation_id is required"))?;

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
            let key = stdio_session_key(
                endpoint,
                auth_profile.as_ref(),
                &request.options.inject_env,
                request.options.cwd.as_deref(),
            )?;
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
            let timeout = request_timeout_duration(request.options.timeout_ms).unwrap_or_else(
                adapters::mcp::transport::McpStdioTransport::default_request_timeout,
            );
            let args = prepare_live_stdio_mcp_execute_args(
                &mut guard, endpoint, &cache, op, raw_args, timeout,
            )
            .await?;
            let arguments = Some(Value::Object(args.into_iter().collect()));
            let result = guard
                .client
                .call_tool_with_timeout(op, arguments, timeout)
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
            let arguments = Some(Value::Object(raw_args.into_iter().collect()));
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

#[async_trait::async_trait]
pub(crate) trait SubscriptionEventRecorder: Send {
    async fn emit(
        &mut self,
        source_kind: &str,
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

struct ChannelSubscriptionRecorder<'a> {
    tx: &'a mpsc::Sender<SubscriptionEventEnvelope>,
    view: &'a Arc<Mutex<SubscriptionJobView>>,
    seq: &'a mut u64,
}

async fn build_subscription_event_record(
    view: &Arc<Mutex<SubscriptionJobView>>,
    seq: &mut u64,
    source_kind: &str,
    event_kind: &str,
    data: Option<Value>,
    meta: Option<Value>,
) -> SubscriptionEventEnvelope {
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
    *seq = next_seq;
    record
}

async fn note_subscription_event_delivered(
    view: &Arc<Mutex<SubscriptionJobView>>,
    timestamp_unix: u64,
) {
    note_subscription_events_written(view, timestamp_unix, 1).await;
}

async fn note_subscription_events_written(
    view: &Arc<Mutex<SubscriptionJobView>>,
    timestamp_unix: u64,
    count: u64,
) {
    let mut guard = view.lock().await;
    guard.written_events = guard.written_events.saturating_add(count);
    guard.last_success_at_unix = Some(timestamp_unix);
    guard.last_event_at_unix = Some(timestamp_unix);
}

async fn note_subscription_success(view: &Arc<Mutex<SubscriptionJobView>>, timestamp_unix: u64) {
    let mut guard = view.lock().await;
    guard.last_success_at_unix = Some(timestamp_unix);
}

#[async_trait::async_trait]
impl SubscriptionEventRecorder for ChannelSubscriptionRecorder<'_> {
    async fn emit(
        &mut self,
        source_kind: &str,
        event_kind: &str,
        data: Option<Value>,
        meta: Option<Value>,
    ) -> Result<()> {
        let record = build_subscription_event_record(
            self.view,
            self.seq,
            source_kind,
            event_kind,
            data,
            meta,
        )
        .await;
        self.tx
            .send(record.clone())
            .await
            .map_err(|_| anyhow!("managed source event channel closed"))?;
        note_subscription_event_delivered(self.view, record.timestamp_unix).await;
        Ok(())
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
    recorder: &mut impl SubscriptionEventRecorder,
    source_kind: &str,
) -> Result<()> {
    recorder
        .emit(
            source_kind,
            "closed",
            None,
            Some(json!({"reason":"stopped"})),
        )
        .await?;
    recorder.update_status(Some("stopped"), None, false).await?;
    Ok(())
}

async fn execute_http_stream_once(
    request: &SubscribeStartRequest,
    recorder: &mut impl SubscriptionEventRecorder,
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
        close_subscription_as_stopped(recorder, "http")
            .await
            .map_err(SubscriptionRunError::Fatal)?;
        return Ok(());
    }
    let response = tokio::select! {
        changed = stop_rx.changed() => {
            if changed.is_ok() && *stop_rx.borrow() {
                close_subscription_as_stopped(recorder, "http").await
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

    recorder
        .emit(
            source_kind,
            "open",
            None,
            Some(json!({ "content_type": content_type, "url": redact_endpoint(target_url) })),
        )
        .await
        .map_err(SubscriptionRunError::Fatal)?;
    recorder
        .update_status(Some("running"), None, false)
        .await
        .map_err(SubscriptionRunError::Fatal)?;

    let mut stream = response.bytes_stream();
    let mut raw_buffer = Vec::new();
    let mut text_buffer = String::new();
    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(recorder, source_kind)
                .await
                .map_err(SubscriptionRunError::Fatal)?;
            return Ok(());
        }
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_ok() && *stop_rx.borrow() {
                    close_subscription_as_stopped(recorder, source_kind).await
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
                            recorder
                                .emit(source_kind, "data", Some(value), None)
                                .await
                                .map_err(SubscriptionRunError::Fatal)?;
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
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut recorder, "http").await?;
            return Ok(());
        }
        match execute_http_stream_once(request, &mut recorder, &mut stop_rx).await {
            Ok(()) => return Ok(()),
            Err(SubscriptionRunError::Fatal(err)) => return Err(err),
            Err(SubscriptionRunError::Retry(err)) => {
                if view.lock().await.status == "running" {
                    delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
                }
                let msg = err.to_string();
                recorder
                    .emit("http", "error", None, Some(json!({ "message": msg })))
                    .await?;
                recorder
                    .update_status(Some("reconnecting"), Some(msg.clone()), true)
                    .await?;
                recorder
                    .emit(
                        "http",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs })),
                    )
                    .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut recorder, "http").await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

struct SubscriptionRecorderObserver<'a, R> {
    recorder: &'a mut R,
    source_kind: &'a str,
}

#[async_trait::async_trait]
impl<R: SubscriptionEventRecorder> WebSocketRuntimeObserver
    for SubscriptionRecorderObserver<'_, R>
{
    async fn emit(
        &mut self,
        event_kind: &str,
        data: Option<Value>,
        meta: Option<Value>,
    ) -> Result<()> {
        self.recorder
            .emit(self.source_kind, event_kind, data, meta)
            .await
    }

    async fn update_status(
        &mut self,
        status: Option<&str>,
        last_error: Option<String>,
        increment_reconnect: bool,
    ) -> Result<()> {
        self.recorder
            .update_status(status, last_error, increment_reconnect)
            .await
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
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::Websocket)
    ) {
        return run_websocket_subscription_job(job_id, request, event_tx, view, stop_rx).await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::DiscordGateway)
    ) {
        return run_discord_gateway_subscription_job(job_id, request, event_tx, view, stop_rx)
            .await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::SlackSocketMode)
    ) {
        return run_slack_socket_mode_subscription_job(job_id, request, event_tx, view, stop_rx)
            .await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::FeishuLongConnection)
    ) {
        return run_feishu_long_connection_subscription_job(
            job_id, request, event_tx, view, stop_rx,
        )
        .await;
    }
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::EmailImapIdle)
    ) {
        return run_email_imap_idle_subscription_job(job_id, request, event_tx, view, stop_rx)
            .await;
    }

    if request.operation_id.is_some() {
        if request
            .operation_id
            .as_deref()
            .is_some_and(|operation_id| operation_id.starts_with("subscription/"))
        {
            return run_graphql_subscription_job(job_id, request, event_tx, view, stop_rx).await;
        }
        return run_jsonrpc_subscription_job(job_id, request, event_tx, view, stop_rx).await;
    }

    if request.resource_uri.is_some() {
        return run_mcp_subscription_job(runtime, job_id, request, event_tx, view, stop_rx).await;
    }

    run_http_subscription_job(job_id, request, event_tx, view, stop_rx).await
}

async fn run_websocket_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut handler = RawFrameHandler;
    let mut observer = SubscriptionRecorderObserver {
        recorder: &mut recorder,
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
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let resolved = resolve_discord_gateway_runtime_config(request)?;
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
    let mut handler = DiscordGatewayHandler::new(resolved.session);

    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut recorder, "discord_gateway").await?;
            return Ok(());
        }

        let websocket_url = if let Some(url) = handler.preferred_gateway_websocket_url() {
            url
        } else {
            match open_discord_gateway_websocket_url(request, &resolved.auth_profile).await {
                Ok((url, _open_meta)) => url,
                Err(err) => {
                    let message = err.to_string();
                    recorder
                        .emit(
                            "discord_gateway",
                            "error",
                            None,
                            Some(json!({ "message": message })),
                        )
                        .await?;
                    recorder
                        .update_status(Some("reconnecting"), Some(message), true)
                        .await?;
                    recorder
                        .emit(
                            "discord_gateway",
                            "reconnect",
                            None,
                            Some(json!({ "delay_secs": delay_secs, "phase": "gateway_open" })),
                        )
                        .await?;
                    if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await
                    {
                        close_subscription_as_stopped(&mut recorder, "discord_gateway").await?;
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
            let mut observer = SubscriptionRecorderObserver {
                recorder: &mut recorder,
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
                recorder
                    .emit(
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
                recorder
                    .emit(
                        "discord_gateway",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                recorder
                    .update_status(Some("reconnecting"), Some(err.to_string()), true)
                    .await?;
                recorder
                    .emit(
                        "discord_gateway",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs })),
                    )
                    .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut recorder, "discord_gateway").await?;
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
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;

    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut recorder, "slack_socket_mode").await?;
            return Ok(());
        }

        let websocket_url = match open_slack_socket_mode_websocket_url(request).await {
            Ok(url) => url,
            Err(err) => {
                let message = err.to_string();
                recorder
                    .emit(
                        "slack_socket_mode",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                recorder
                    .update_status(Some("reconnecting"), Some(message), true)
                    .await?;
                recorder
                    .emit(
                        "slack_socket_mode",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs, "phase": "open_url" })),
                    )
                    .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut recorder, "slack_socket_mode").await?;
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
            let mut observer = SubscriptionRecorderObserver {
                recorder: &mut recorder,
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
                recorder
                    .emit(
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
                recorder
                    .emit(
                        "slack_socket_mode",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                recorder
                    .update_status(Some("reconnecting"), Some(err.to_string()), true)
                    .await?;
                recorder
                    .emit(
                        "slack_socket_mode",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs })),
                    )
                    .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut recorder, "slack_socket_mode").await?;
                    return Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    }
}

async fn run_email_imap_idle_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?
            .ok_or_else(|| {
                anyhow!("email-imap-idle requires an auth profile with username/password fields")
            })?;
    let config: EmailImapIdleRuntimeConfig =
        resolve_email_imap_idle_runtime_config(request, &auth_profile)?;
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    run_email_imap_idle_subscription_runtime(config, &mut recorder, &mut stop_rx).await
}

async fn run_feishu_long_connection_subscription_job(
    _job_id: &str,
    request: &SubscribeStartRequest,
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;

    loop {
        if *stop_rx.borrow() {
            close_subscription_as_stopped(&mut recorder, "feishu_long_connection").await?;
            return Ok(());
        }

        let open = match open_feishu_long_connection_websocket_url(request).await {
            Ok(open) => open,
            Err(err) => {
                let message = err.to_string();
                recorder
                    .emit(
                        "feishu_long_connection",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                recorder
                    .update_status(Some("reconnecting"), Some(message), true)
                    .await?;
                recorder
                    .emit(
                        "feishu_long_connection",
                        "reconnect",
                        None,
                        Some(json!({ "delay_secs": delay_secs, "phase": "open_url" })),
                    )
                    .await?;
                if wait_for_stop_or_timeout(&mut stop_rx, Duration::from_secs(delay_secs)).await {
                    close_subscription_as_stopped(&mut recorder, "feishu_long_connection").await?;
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
            let mut observer = SubscriptionRecorderObserver {
                recorder: &mut recorder,
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
                recorder
                    .emit(
                        "feishu_long_connection",
                        "error",
                        None,
                        Some(json!({ "message": err.to_string() })),
                    )
                    .await?;
                return Err(err);
            }
            Err(WebSocketRunError::Retry(err)) => {
                if view.lock().await.status == "running" {
                    delay_secs = SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS;
                }
                let message = err.to_string();
                recorder
                    .emit(
                        "feishu_long_connection",
                        "error",
                        None,
                        Some(json!({ "message": message })),
                    )
                    .await?;
                recorder
                    .update_status(Some("reconnecting"), Some(err.to_string()), true)
                    .await?;
                recorder
                    .emit(
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
                    close_subscription_as_stopped(&mut recorder, "feishu_long_connection").await?;
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
        bail!("subscription runtime currently supports only GraphQL subscription/<field> operation IDs");
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
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
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
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let handler_config = GraphQLSubscriptionConfig {
        operation_id: operation_id.clone(),
        query: prepared.query,
        variables: prepared.variables,
    };
    let mut observer = SubscriptionRecorderObserver {
        recorder: &mut recorder,
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
                    close_subscription_as_stopped(&mut recorder, "graphql").await?;
                    break Ok(());
                }
                delay_secs =
                    (delay_secs.saturating_mul(2)).min(SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS);
            }
        }
    };

    if let Err(err) = result {
        recorder
            .emit(
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
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
    view: Arc<Mutex<SubscriptionJobView>>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let config = resolve_jsonrpc_subscription_config(request)?;
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let subscribe_message = JsonRpcSubscriptionHandler::new(config.clone()).subscribe_message();
    let mut handler = JsonRpcSubscriptionHandler::new(config.clone());
    let mut observer = SubscriptionRecorderObserver {
        recorder: &mut recorder,
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
        recorder
            .emit(
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

async fn fetch_managed_source_poll(
    runtime: &DaemonRuntime,
    request: &SubscribeStartRequest,
    args: HashMap<String, Value>,
    checkpoint: &crate::subscription_poll::PollCheckpointState,
) -> Result<crate::subscription_poll::PollFetchResult> {
    let mut options = request.options.clone();
    if let Some(etag) = checkpoint.etag.as_ref() {
        options
            .request_headers
            .insert("if-none-match".to_string(), etag.clone());
    }
    let response = runtime
        .invoke(RuntimeInvokeRequest {
            request_id: format!("{}-poll-{}", request.request_id, now_unix_secs()),
            endpoint: request.endpoint.clone(),
            action: RuntimeAction::Execute,
            operation_id: request.operation_id.clone(),
            args: Some(args),
            suppress_routine_logs: true,
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

async fn run_mcp_subscription_job(
    runtime: &DaemonRuntime,
    _job_id: &str,
    request: &SubscribeStartRequest,
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
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
            event_tx,
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
        request.options.cwd.as_deref(),
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

    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut cursor = 0u64;
    recorder
        .emit(
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
            &mut recorder,
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
                    match close_subscription_as_stopped(&mut recorder, "mcp_resource").await {
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
                    if let Err(err) = recorder
                        .emit(
                            "mcp_resource",
                            "data",
                            notification.params.clone(),
                            Some(json!({"method": notification.method})),
                        )
                        .await
                    {
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
                            &mut recorder,
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
        recorder
            .emit(
                "mcp_resource",
                "error",
                None,
                Some(json!({ "message": msg })),
            )
            .await?;
        recorder
            .update_status(None, Some(msg.clone()), false)
            .await?;
        if run_result.is_ok() {
            return Err(err);
        }
    }

    run_result
}

async fn run_mcp_http_subscription_job(
    runtime: &DaemonRuntime,
    request: &SubscribeStartRequest,
    event_tx: mpsc::Sender<SubscriptionEventEnvelope>,
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

    let mut seq = 0u64;
    let mut recorder = ChannelSubscriptionRecorder {
        tx: &event_tx,
        view: &view,
        seq: &mut seq,
    };
    let mut cursor = 0u64;
    recorder
        .emit(
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
            &mut recorder,
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
                    match close_subscription_as_stopped(&mut recorder, "mcp_resource").await {
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
                    if let Err(err) = recorder
                        .emit(
                            "mcp_resource",
                            "data",
                            notification.params.clone(),
                            Some(json!({"method": notification.method})),
                        )
                        .await
                    {
                        break 'run Err(err);
                    }
                    if request.read_resource && should_read_mcp_resource_snapshot(&notification) {
                        let read_result = session.read_resource(resource_uri).await;
                        if let Err(err) = append_mcp_resource_read_result(
                            &mut recorder,
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
        recorder
            .emit(
                "mcp_resource",
                "error",
                None,
                Some(json!({ "message": msg })),
            )
            .await?;
        recorder
            .update_status(None, Some(msg.clone()), false)
            .await?;
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
pub async fn daemon_session_kill_client(
    request: &DaemonSessionKillRequest,
) -> Result<DaemonSessionKillResponse> {
    let value = client_call("daemon.session.kill", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn daemon_session_kill_client(
    _request: &DaemonSessionKillRequest,
) -> Result<DaemonSessionKillResponse> {
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
pub async fn source_doctor_client(
    request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceDoctorResponse> {
    let value = client_call("source.doctor", Some(serde_json::to_value(request)?)).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn source_doctor_client(
    _request: &ManagedSourceStatusRequest,
) -> Result<ManagedSourceDoctorResponse> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
pub async fn source_list_client() -> Result<Vec<ManagedSourceListEntry>> {
    let value = client_call("source.list", None).await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(not(unix))]
pub async fn source_list_client() -> Result<Vec<ManagedSourceListEntry>> {
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

    let diagnostics = inspect_daemon_local_diagnostics()?;
    if diagnostics.owner_lock_held {
        let code = if diagnostics.owner_pid_alive {
            "DAEMON_OWNER_UNREACHABLE"
        } else {
            "DAEMON_OWNER_HELD"
        };
        let message = if diagnostics.owner_pid_alive {
            format!(
                "A live daemon owner already exists (pid={}) but is unreachable. Refusing to start a second daemon. Run `uxc daemon doctor`.",
                diagnostics
                    .owner_pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )
        } else {
            "A daemon owner lock is still held. Refusing to start a second daemon. Run `uxc daemon doctor`."
                .to_string()
        };
        return Err(
            StructuredError::new(code, message, Some(serde_json::to_value(&diagnostics)?)).into(),
        );
    }

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

    let diagnostics = inspect_daemon_local_diagnostics()?;
    drop(start_lock);
    if diagnostics.owner_lock_held {
        return Err(StructuredError::new(
            "DAEMON_OWNER_UNREACHABLE",
            "Daemon owner exists but did not become reachable in time. Run `uxc daemon doctor`."
                .to_string(),
            Some(serde_json::to_value(diagnostics)?),
        )
        .into());
    }
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
    let mut owner_lock = DaemonOwnerLockGuard::acquire(
        &daemon_lock_path(),
        &DaemonOwnerMetadata {
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            socket: socket.display().to_string(),
            started_at_unix: now_unix_secs(),
        },
    )?;
    if socket.exists() {
        let _ = fs::remove_file(&socket);
    }

    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("Failed to bind daemon socket at {}", socket.display()))?;

    let runtime = Arc::new(DaemonRuntime::try_new()?);
    let resume_runtime = runtime.clone();
    tokio::spawn(async move {
        if let Err(err) = resume_runtime.resume_managed_sources().await {
            tracing::warn!("Failed to resume managed sources: {}", err);
        }
    });
    let cleanup_runtime = runtime.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_millis(MCP_IDLE_CLEANUP_INTERVAL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if cleanup_runtime.should_stop().await {
                break;
            }
            cleanup_runtime.mcp.cleanup_idle().await;
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
    runtime.flush_logs().await;

    owner_lock.clear_metadata();
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
        "daemon.session.kill" => {
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
            let request: DaemonSessionKillRequest = match serde_json::from_value(params) {
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
            match runtime.session_kill(&request).await {
                Ok(killed) => JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: req.id,
                    result: Some(serde_json::to_value(killed)?),
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
        "source.doctor" => {
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
            match runtime.source_doctor(&status).await {
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
        "source.list" => match runtime.source_list().await {
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
        },
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

pub async fn daemon_session_kill_local(
    request: &DaemonSessionKillRequest,
) -> Result<DaemonSessionKillResponse> {
    daemon_session_kill_client(request).await
}

pub async fn daemon_doctor_local_result() -> Result<DaemonDoctorResponse> {
    tokio::task::spawn_blocking(daemon_doctor_local_blocking)
        .await
        .map_err(|err| anyhow!("daemon doctor task failed: {err}"))?
}

pub async fn daemon_start_local() -> Result<EnsureDaemonOutcome> {
    ensure_compatible_daemon_running().await
}

pub async fn daemon_stop_local() -> Result<bool> {
    if daemon_status_client().await.is_err() {
        let diagnostics = inspect_daemon_local_diagnostics()?;
        if !diagnostics.owner_lock_held {
            return Ok(false);
        }

        let pid = diagnostics.owner_pid.ok_or_else(|| {
            StructuredError::new(
                "DAEMON_OWNER_METADATA_MISSING",
                "Daemon owner lock is held but owner pid metadata is missing. Run `uxc daemon doctor`."
                    .to_string(),
                Some(serde_json::to_value(&diagnostics).unwrap_or(Value::Null)),
            )
        })?;

        if !diagnostics.owner_pid_alive {
            return Err(StructuredError::new(
                "DAEMON_OWNER_STALE",
                "Daemon owner metadata is stale. Run `uxc daemon doctor`.".to_string(),
                Some(serde_json::to_value(&diagnostics)?),
            )
            .into());
        }

        #[cfg(unix)]
        {
            // SAFETY: kill with SIGTERM targets the owner pid discovered from local metadata.
            if unsafe { kill(pid as i32, DAEMON_OWNER_TERM_SIGNAL) } == -1 {
                return Err(std::io::Error::last_os_error().into());
            }

            for _ in 0..STOP_POLL_TRIES {
                tokio::time::sleep(Duration::from_millis(STOP_POLL_INTERVAL_MS)).await;
                let diagnostics = inspect_daemon_local_diagnostics()?;
                if !diagnostics.owner_lock_held {
                    let _ = clear_daemon_owner_metadata_path(&daemon_lock_path());
                    if socket_path().exists() {
                        let _ = fs::remove_file(socket_path());
                    }
                    return Ok(true);
                }
            }

            return Err(StructuredError::new(
                "DAEMON_STOP_TIMEOUT",
                format!(
                    "Daemon owner pid {} did not stop in time. Run `uxc daemon doctor`.",
                    pid
                ),
                Some(serde_json::to_value(diagnostics)?),
            )
            .into());
        }

        #[cfg(not(unix))]
        {
            return Err(StructuredError::new(
                "DAEMON_STOP_UNSUPPORTED_PLATFORM",
                format!(
                    "Daemon owner pid {} cannot be stopped via Unix signals on this platform.",
                    pid
                ),
                Some(serde_json::to_value(&diagnostics)?),
            )
            .into());
        }
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
            return Err(structured_error_from_jsonrpc_error(
                i64::from(err.code),
                &err.message,
                None,
                "OAUTH_REQUIRED",
            )
            .into());
        }
        if err.code == ERR_OAUTH_REFRESH_FAILED {
            return Err(structured_error_from_jsonrpc_error(
                i64::from(err.code),
                &err.message,
                None,
                "OAUTH_REFRESH_FAILED",
            )
            .into());
        }
        if err.code == ERR_OAUTH_SCOPE_INSUFFICIENT {
            return Err(structured_error_from_jsonrpc_error(
                i64::from(err.code),
                &err.message,
                None,
                "OAUTH_SCOPE_INSUFFICIENT",
            )
            .into());
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

fn daemon_lock_path() -> PathBuf {
    daemon_dir().join("daemon.lock")
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
    cwd: Option<&str>,
) -> Result<String> {
    Ok(format!(
        "stdio:{}:{}:{}:{}",
        endpoint,
        auth_fingerprint(profile),
        stdio_env_fingerprint(inject_env, profile)?,
        cwd.unwrap_or("")
    ))
}

fn parse_stdio_session_key(
    session_key: &str,
) -> (Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    let Some(rest) = session_key.strip_prefix("stdio:") else {
        return (None, None, None, None);
    };
    let Some((before_cwd, cwd)) = rest.rsplit_once(':') else {
        return (Some(rest), None, None, None);
    };
    let Some((before_env, env_fp)) = before_cwd.rsplit_once(':') else {
        return (Some(before_cwd), Some(cwd), None, None);
    };
    let Some((endpoint, auth_fp)) = before_env.rsplit_once(':') else {
        return (Some(before_env), Some(env_fp), Some(cwd), None);
    };
    (Some(endpoint), Some(auth_fp), Some(env_fp), Some(cwd))
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
    if !adapters::mcp::McpAdapter::is_stdio_command(endpoint) {
        if options.inject_env.is_empty() {
            return Ok(None);
        }
        return Err(UxcError::InvalidArguments(
            "--inject-env is only supported for stdio endpoints".to_string(),
        )
        .into());
    }
    let cwd = options.cwd.as_deref().map(PathBuf::from);
    if options.inject_env.is_empty() && cwd.is_none() {
        return Ok(None);
    }
    let env_overrides = if options.inject_env.is_empty() {
        Vec::new()
    } else {
        let profile = profile.ok_or_else(|| {
            UxcError::InvalidArguments(
                "--inject-env requires a credential. Use --auth <credential_id> for direct stdio calls, or --credential <credential_id> when creating a link.".to_string(),
            )
        })?;
        render_injected_env(&options.inject_env, profile)?
    };
    Ok(Some(adapters::mcp::StdioSpawnOptions {
        env_overrides,
        cwd,
    }))
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
    artifact_compaction: bool,
) -> Result<()> {
    if !artifact_compaction {
        return Ok(());
    }

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

    let session_key = stdio_session_key(
        &request.endpoint,
        auth_profile,
        &request.options.inject_env,
        request.options.cwd.as_deref(),
    )?;
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

async fn prepare_live_stdio_mcp_execute_args(
    session: &mut McpStdioSession,
    endpoint: &str,
    cache: &Arc<dyn Cache>,
    operation_id: &str,
    raw_args: HashMap<String, Value>,
    timeout: Duration,
) -> Result<HashMap<String, Value>> {
    if raw_args.is_empty() {
        return Ok(raw_args);
    }

    let _ = session
        .mark_tools_dirty_from_notifications(endpoint, cache)
        .await;
    let tools = match session
        .refresh_tools_if_needed(endpoint, cache, timeout)
        .await
    {
        Ok(tools) => tools,
        Err(err) => {
            tracing::debug!(
                endpoint = %endpoint,
                operation_id = %operation_id,
                error = %err,
                "Failed to refresh live MCP stdio tool catalog for arg coercion; using raw args"
            );
            return Ok(raw_args);
        }
    };

    let Some(tool) = tools.iter().find(|tool| tool.name == *operation_id) else {
        tracing::debug!(
            endpoint = %endpoint,
            operation_id = %operation_id,
            "Live MCP stdio tool catalog did not contain requested operation; using raw args"
        );
        return Ok(raw_args);
    };

    prepare_execute_args_from_detail(
        ProtocolType::Mcp,
        operation_id,
        &operation_detail_from_mcp_tool(tool),
        raw_args,
    )
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
    use rusqlite::Connection;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;
    use std::time::Duration as StdDuration;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn managed_source_poll_jitter_preserves_interval_cadence() {
        let initial_one = managed_source_initial_poll_delay_duration(
            "agentinbox",
            "github_repo:holon-run/uxc",
            30,
        );
        let initial_two = managed_source_initial_poll_delay_duration(
            "agentinbox",
            "github_repo:holon-run/uxc",
            30,
        );

        assert_eq!(initial_one, initial_two);
        assert!(initial_one < StdDuration::from_millis(7_500));

        for round in 0..32 {
            let one = managed_source_poll_wait_duration(
                "agentinbox",
                "github_repo:holon-run/uxc",
                30,
                round,
            );
            let two = managed_source_poll_wait_duration(
                "agentinbox",
                "github_repo:holon-run/uxc",
                30,
                round,
            );

            assert_eq!(one, two);
            assert!(one >= StdDuration::from_millis(26_250));
            assert!(one <= StdDuration::from_millis(33_750));
        }
    }

    #[test]
    fn openapi_runtime_endpoint_appends_operation_path_for_execute() {
        let request = RuntimeInvokeRequest {
            request_id: "req-1".to_string(),
            endpoint: "https://testnet.binance.vision".to_string(),
            action: RuntimeAction::Execute,
            operation_id: Some("get:/api/v3/account".to_string()),
            args: None,
            suppress_routine_logs: false,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                artifact_compaction: None,
                schema_url: Some("https://example.com/schema.json".to_string()),
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
                cwd: None,
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
    fn parse_stdio_session_key_supports_cwd_segment() {
        let (endpoint, auth_fp, env_fp, cwd) =
            parse_stdio_session_key("stdio:./mcp-stdio-rel ok:auth123:env456:/tmp/workdir");
        assert_eq!(endpoint, Some("./mcp-stdio-rel ok"));
        assert_eq!(auth_fp, Some("auth123"));
        assert_eq!(env_fp, Some("env456"));
        assert_eq!(cwd, Some("/tmp/workdir"));
    }

    #[test]
    fn build_stdio_spawn_options_ignores_cwd_for_non_stdio_endpoints() {
        let options = RuntimeInvokeOptions {
            auth: None,
            inject_env: Vec::new(),
            no_cache: false,
            cache_ttl: None,
            timeout_ms: None,
            refresh_schema: false,
            artifact_compaction: None,
            schema_url: None,
            link_name: None,
            link_skill: None,
            link_skill_doc: None,
            link_skill_path: None,
            schema_mapping_file: None,
            daemon_exclusive: Vec::new(),
            daemon_idle_ttl: None,
            request_headers: HashMap::new(),
            cwd: Some("/tmp/workdir".to_string()),
        };

        let options = build_stdio_spawn_options("https://api.example.com", &options, None)
            .expect("non-stdio requests should ignore cwd");
        assert!(options.is_none());
    }

    #[test]
    fn build_stdio_spawn_options_still_rejects_inject_env_for_non_stdio_endpoints() {
        let options = RuntimeInvokeOptions {
            auth: None,
            inject_env: vec![InjectEnvSpec::parse("TOKEN={{secret}}").expect("valid inject env")],
            no_cache: false,
            cache_ttl: None,
            timeout_ms: None,
            refresh_schema: false,
            artifact_compaction: None,
            schema_url: None,
            link_name: None,
            link_skill: None,
            link_skill_doc: None,
            link_skill_path: None,
            schema_mapping_file: None,
            daemon_exclusive: Vec::new(),
            daemon_idle_ttl: None,
            request_headers: HashMap::new(),
            cwd: Some("/tmp/workdir".to_string()),
        };

        let err = build_stdio_spawn_options("https://api.example.com", &options, None)
            .expect_err("inject-env should still be rejected for non-stdio endpoints");
        assert!(err
            .to_string()
            .contains("--inject-env is only supported for stdio endpoints"));
    }

    #[test]
    fn runtime_artifact_compaction_runs_when_enabled() {
        let mut data = json!({
            "blob": "x".repeat(80_000),
            "ok": true
        });
        let mut meta = RuntimeMeta::default();

        apply_runtime_artifact_compaction("call_result", &mut data, &mut meta, true)
            .expect("compaction should succeed");

        assert_eq!(meta.artifact_truncated, Some(true));
        assert_eq!(meta.artifact_kind.as_deref(), Some("call_result"));
        assert!(meta.artifact_path.is_some());
        assert!(data["blob"]
            .as_str()
            .is_some_and(|preview| preview.len() < 80_000));
    }

    #[test]
    fn runtime_artifact_compaction_can_be_disabled_per_request() {
        let mut data = json!({
            "blob": "x".repeat(80_000),
            "ok": true
        });
        let original = data.clone();
        let mut meta = RuntimeMeta::default();

        apply_runtime_artifact_compaction("call_result", &mut data, &mut meta, false)
            .expect("disabled compaction should leave payload inline");

        assert_eq!(data, original);
        assert!(meta.artifact_truncated.is_none());
        assert!(meta.artifact_kind.is_none());
        assert!(meta.artifact_path.is_none());
    }

    fn with_mcp_idle_ttl_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let previous = std::env::var_os(MCP_IDLE_TTL_ENV);
        match value {
            Some(value) => std::env::set_var(MCP_IDLE_TTL_ENV, value),
            None => std::env::remove_var(MCP_IDLE_TTL_ENV),
        }

        let result = f();

        match previous {
            Some(previous) => std::env::set_var(MCP_IDLE_TTL_ENV, previous),
            None => std::env::remove_var(MCP_IDLE_TTL_ENV),
        }
        result
    }

    #[test]
    fn default_mcp_idle_ttl_secs_uses_default_without_env() {
        with_mcp_idle_ttl_env(None, || {
            assert_eq!(default_mcp_idle_ttl_secs(), MCP_IDLE_TTL_DEFAULT_SECS);
        });
    }

    #[test]
    fn default_mcp_idle_ttl_secs_accepts_positive_env_override() {
        with_mcp_idle_ttl_env(Some("7200"), || {
            assert_eq!(default_mcp_idle_ttl_secs(), 7200);
        });
    }

    #[test]
    fn default_mcp_idle_ttl_secs_rejects_invalid_zero_and_clamps_extreme_env_overrides() {
        with_mcp_idle_ttl_env(Some("not-a-number"), || {
            assert_eq!(default_mcp_idle_ttl_secs(), MCP_IDLE_TTL_DEFAULT_SECS);
        });
        with_mcp_idle_ttl_env(Some("0"), || {
            assert_eq!(default_mcp_idle_ttl_secs(), MCP_IDLE_TTL_DEFAULT_SECS);
        });
        with_mcp_idle_ttl_env(Some("-1"), || {
            assert_eq!(default_mcp_idle_ttl_secs(), MCP_IDLE_TTL_DEFAULT_SECS);
        });
        with_mcp_idle_ttl_env(Some("9999999999999999999"), || {
            assert_eq!(default_mcp_idle_ttl_secs(), MCP_IDLE_TTL_MAX_SECS);
        });
    }

    #[test]
    fn instant_cutoff_returns_none_for_unrepresentable_age() {
        let now = Instant::now();

        assert!(instant_cutoff(now, 0).is_some());
        assert!(instant_cutoff(now, u64::MAX).is_none());
    }

    #[test]
    fn resolve_stdio_request_metadata_resets_ttl_and_link_name_from_current_request() {
        with_mcp_idle_ttl_env(None, || {
            let resolved = resolve_stdio_request_metadata(
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

            assert_eq!(resolved.idle_ttl_secs, default_mcp_idle_ttl_secs());
            assert_eq!(resolved.link_name, None);
            assert_eq!(resolved.link_skill, None);
            assert_eq!(resolved.link_skill_doc, None);
            assert_eq!(resolved.link_skill_path, None);
            assert_eq!(resolved.endpoint, "https://new.example.com");
            assert_eq!(resolved.daemon_exclusive, vec!["/tmp/profile".to_string()]);
        });
    }

    #[test]
    fn resolve_stdio_request_metadata_accepts_zero_ttl_override() {
        let resolved = resolve_stdio_request_metadata(
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

        assert_eq!(resolved.idle_ttl_secs, 0);
        assert_eq!(resolved.link_name, Some("board-link".to_string()));
        assert_eq!(resolved.link_skill, Some("board-webmcp".to_string()));
        assert_eq!(
            resolved.link_skill_doc,
            Some("https://uxc.holon.run/skills/board-webmcp/".to_string())
        );
        assert_eq!(
            resolved.link_skill_path,
            Some("skills/board-webmcp/SKILL.md".to_string())
        );
        assert_eq!(resolved.endpoint, "https://new.example.com");
        assert_eq!(
            resolved.daemon_exclusive,
            vec!["/tmp/new-profile".to_string()]
        );
    }

    #[test]
    fn openapi_runtime_endpoint_ignores_non_execute_requests() {
        let request = RuntimeInvokeRequest {
            request_id: "req-1".to_string(),
            endpoint: "https://api.binance.com".to_string(),
            action: RuntimeAction::OperationHelp,
            operation_id: Some("post:/api/v3/order".to_string()),
            args: None,
            suppress_routine_logs: false,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                artifact_compaction: None,
                schema_url: Some("https://example.com/schema.json".to_string()),
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
                cwd: None,
            },
        };

        assert!(openapi_runtime_endpoint(&request).is_none());
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
            suppress_routine_logs: false,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                artifact_compaction: None,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
                cwd: None,
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
            suppress_routine_logs: false,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                timeout_ms: None,
                refresh_schema: false,
                artifact_compaction: None,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
                cwd: None,
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
                artifact_compaction: None,
                schema_url: None,
                link_name: None,
                link_skill: None,
                link_skill_doc: None,
                link_skill_path: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
                daemon_idle_ttl: None,
                request_headers: HashMap::new(),
                cwd: None,
            },
        }
    }

    fn poll_checkpoint_test_record(
        namespace: &str,
        source_key: &str,
        run_id: &str,
    ) -> ManagedSourceRecord {
        let now = now_unix_secs();
        ManagedSourceRecord {
            namespace: namespace.to_string(),
            source_key: source_key.to_string(),
            spec_json: json!({
                "endpoint": "https://example.com/events",
                "operation_id": "get:/events",
                "mode": "poll",
                "poll_config": {
                    "interval_secs": 1,
                    "extract_items_pointer": "/items",
                    "checkpoint_strategy": {
                        "type": "item_key",
                        "item_key_pointer": "/id"
                    }
                }
            }),
            spec_key: "spec-key".to_string(),
            run_id: run_id.to_string(),
            stream_id: managed_stream_id(namespace, source_key),
            status: "starting".to_string(),
            created_at_unix: now,
            updated_at_unix: now,
            started_at_unix: Some(now),
            stopped_at_unix: None,
            last_error: None,
            last_success_at_unix: None,
            last_event_at_unix: None,
            reconnect_count: 0,
            written_events: 0,
        }
    }

    fn assert_no_legacy_managed_source_files(base_dir: &Path, run_id: &str) {
        assert!(!managed_source_sink_path(base_dir, run_id).exists());
        assert!(!managed_source_checkpoint_path(base_dir, run_id).exists());
        assert!(!managed_source_cursor_path(base_dir, run_id).exists());
    }

    fn test_runtime_with_store(temp: &tempfile::TempDir) -> DaemonRuntime {
        DaemonRuntime::try_new_with_managed_source_base_dir(temp.path().to_path_buf())
            .expect("test daemon runtime should initialize")
    }

    #[test]
    fn client_call_error_path_does_not_double_prefix_oauth_messages() {
        let err = structured_error_from_jsonrpc_error(
            i64::from(ERR_OAUTH_REQUIRED),
            "OAuth required: No refresh token available for credential 'x-api-user' at 'https://api.x.com'.\n\nFor agents:\n  1. uxc auth oauth start x-api-user --endpoint https://api.x.com --redirect-uri <callback_uri>\n  2. uxc auth oauth complete x-api-user --session-id <session_id> --authorization-response '<callback_url_or_code>'\n\nInteractive fallback:\n  uxc auth oauth login x-api-user --endpoint https://api.x.com",
            None,
            "OAUTH_REQUIRED",
        );
        assert_eq!(err.code, "OAUTH_REQUIRED");
        assert_eq!(
            err.message,
            "OAuth required: No refresh token available for credential 'x-api-user' at 'https://api.x.com'.\n\nFor agents:\n  1. uxc auth oauth start x-api-user --endpoint https://api.x.com --redirect-uri <callback_uri>\n  2. uxc auth oauth complete x-api-user --session-id <session_id> --authorization-response '<callback_url_or_code>'\n\nInteractive fallback:\n  uxc auth oauth login x-api-user --endpoint https://api.x.com"
        );
        assert!(
            !err.message.contains("OAuth required: OAuth required:"),
            "oauth-required message should not duplicate display prefixes"
        );
        assert!(err.message.contains("For agents:"));
        assert!(err.message.contains("uxc auth oauth start x-api-user"));
        assert!(err.message.contains("Interactive fallback:"));
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
        assert!(!temp.path().join("subscriptions.json").exists());
        assert_no_legacy_managed_source_files(temp.path(), &ensured.run_id);

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
    async fn managed_source_stream_runner_restarts_after_remote_disconnect() {
        let temp = tempdir().unwrap();
        let (endpoint, connects, server_task) = start_test_websocket_server(vec![
            TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":"first"}"#)],
                hold_open_after_send: false,
            },
            TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":"second"}"#)],
                hold_open_after_send: true,
            },
        ])
        .await;

        let runtime = test_runtime_with_store(&temp);
        let ensured = runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "test".to_string(),
                source_key: "websocket:restart".to_string(),
                spec: managed_source_spec(&endpoint),
            })
            .await
            .unwrap();

        let mut seen_second = false;
        for _ in 0..50 {
            let page = runtime
                .stream_read(&ManagedStreamReadRequest {
                    stream_id: ensured.stream_id.clone(),
                    after_offset: 0,
                    limit: 10,
                })
                .await
                .unwrap();
            if page
                .events
                .iter()
                .any(|event| event.raw_payload == json!({"value":"second"}))
            {
                seen_second = true;
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }

        assert!(
            seen_second,
            "runner should reconnect and capture later events"
        );
        assert!(
            connects.load(Ordering::SeqCst) >= 2,
            "server should observe a reconnect"
        );

        let status = runtime
            .source_status(&ManagedSourceStatusRequest {
                namespace: "test".to_string(),
                source_key: "websocket:restart".to_string(),
            })
            .await
            .unwrap();
        assert_ne!(status.status, "failed");

        let doctor = runtime
            .source_doctor(&ManagedSourceStatusRequest {
                namespace: "test".to_string(),
                source_key: "websocket:restart".to_string(),
            })
            .await
            .unwrap();
        assert!(
            !doctor
                .issues
                .iter()
                .any(|issue| issue.code == "runner_inactive"),
            "active source must keep a registered runner after reconnect"
        );

        runtime
            .source_stop(&ManagedSourceStatusRequest {
                namespace: "test".to_string(),
                source_key: "websocket:restart".to_string(),
            })
            .await
            .unwrap();
        server_task.abort();
    }

    #[test]
    fn managed_source_store_migrates_v0_schema_to_v3() {
        let temp = tempdir().unwrap();
        let db_path = managed_source_streams_db_path(temp.path());
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            pragma user_version = 0;
            create table managed_sources (
                namespace text not null,
                source_key text not null,
                spec_json text not null,
                spec_key text not null,
                run_id text not null,
                stream_id text not null,
                status text not null,
                created_at_unix integer not null,
                updated_at_unix integer not null,
                started_at_unix integer,
                stopped_at_unix integer,
                last_error text,
                underlying_job_id text,
                mirrored_after_seq integer not null default 0,
                primary key (namespace, source_key)
            );
            create table event_streams (
                stream_id text primary key,
                namespace text not null,
                source_key text not null,
                created_at_unix integer not null,
                retention_max_rows integer not null,
                retention_max_age_secs integer not null
            );
            create table stream_events (
                stream_id text not null,
                offset integer not null,
                ingested_at_unix integer not null,
                raw_payload_json text not null,
                primary key (stream_id, offset)
            );
            "#,
        )
        .unwrap();
        conn.execute(
            r#"
            insert into event_streams(
                stream_id,
                namespace,
                source_key,
                created_at_unix,
                retention_max_rows,
                retention_max_age_secs
            )
            values ('stream_v0', 'test', 'poll-v0', 1, 10000, 604800)
            "#,
            [],
        )
        .unwrap();
        conn.execute(
            r#"
            insert into stream_events(stream_id, offset, ingested_at_unix, raw_payload_json)
            values ('stream_v0', 5, 1, '{"id":5}')
            "#,
            [],
        )
        .unwrap();
        drop(conn);

        let _store = ManagedSourceStore::new(db_path.clone()).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        let version: i64 = conn
            .query_row("pragma user_version", [], |row| row.get(0))
            .unwrap();
        let has_checkpoint_column: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('managed_sources') where name = 'poll_checkpoint_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let has_last_success_column: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('managed_sources') where name = 'last_success_at_unix'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let has_last_event_column: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('managed_sources') where name = 'last_event_at_unix'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let has_reconnect_column: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('managed_sources') where name = 'reconnect_count'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let has_written_events_column: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('managed_sources') where name = 'written_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let has_next_event_offset_column: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('event_streams') where name = 'next_event_offset'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let next_event_offset: u64 = conn
            .query_row(
                "select next_event_offset from event_streams where stream_id = 'stream_v0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 3);
        assert_eq!(has_checkpoint_column, 1);
        assert_eq!(has_last_success_column, 1);
        assert_eq!(has_last_event_column, 1);
        assert_eq!(has_reconnect_column, 1);
        assert_eq!(has_written_events_column, 1);
        assert_eq!(has_next_event_offset_column, 1);
        assert_eq!(next_event_offset, 6);
    }

    #[test]
    fn managed_source_view_redacts_endpoint_from_spec() {
        let mut view = ManagedSourceView {
            namespace: "test".to_string(),
            source_key: "redacted".to_string(),
            spec_key: "spec-redacted".to_string(),
            run_id: "run-redacted".to_string(),
            stream_id: managed_stream_id("test", "redacted"),
            status: "running".to_string(),
            created_at_unix: 1,
            updated_at_unix: 1,
            started_at_unix: Some(1),
            stopped_at_unix: None,
            last_error: None,
            mode: None,
            endpoint: None,
            operation_id: None,
            resource_uri: None,
            poll_interval_secs: None,
            last_success_at_unix: None,
            last_event_at_unix: None,
            reconnect_count: 0,
            written_events: 0,
            checkpoint: None,
            stream: None,
        };
        let mut spec =
            managed_source_spec("https://user:pass@example.com/events?api_key=secret123&token=abc");
        spec.mode = SubscriptionMode::Poll;
        spec.poll_config = Some(json!({
            "interval_secs": 5,
            "extract_items_pointer": "/items",
            "checkpoint_strategy": {
                "type": "cursor_only"
            }
        }));

        enrich_managed_source_view_from_spec(
            &mut view,
            &serde_json::to_value(spec).expect("managed source spec should serialize"),
        );

        assert_eq!(
            view.endpoint.as_deref(),
            Some("https://***:***@example.com/events?api_key=***&token=***")
        );
        assert_eq!(view.mode, Some(SubscriptionMode::Poll));
        assert_eq!(view.poll_interval_secs, Some(5));
    }

    #[tokio::test]
    async fn legacy_managed_source_checkpoint_imports_into_database() {
        let temp = tempdir().unwrap();
        let runtime = test_runtime_with_store(&temp);
        let record = poll_checkpoint_test_record("test", "poll-import", "run-import");
        runtime
            .managed_sources
            .store
            .upsert_source(&record, true)
            .await
            .unwrap();

        let checkpoint = crate::subscription_poll::PollCheckpointState {
            cursor: Some(json!(123)),
            watermark: None,
            tie_breaker: None,
            seen_keys: VecDeque::from([json!("event-123").to_string()]),
            etag: Some("\"etag-v1\"".to_string()),
        };
        let checkpoint_path = runtime.managed_source_checkpoint_path(&record.run_id);
        fs::create_dir_all(checkpoint_path.parent().unwrap()).unwrap();
        fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();

        let loaded = runtime
            .managed_sources
            .load_or_import_legacy_managed_source_checkpoint(&runtime, &record)
            .await
            .unwrap();
        assert_eq!(loaded, Some(checkpoint.clone()));
        assert!(!checkpoint_path.exists());

        let stored = runtime
            .managed_sources
            .store
            .load_poll_checkpoint(&record.namespace, &record.source_key, &record.run_id)
            .await
            .unwrap();
        assert_eq!(stored, Some(checkpoint));
    }

    #[tokio::test]
    async fn poll_checkpoint_only_write_updates_checkpoint_without_stream_events() {
        let temp = tempdir().unwrap();
        let runtime = test_runtime_with_store(&temp);
        let record =
            poll_checkpoint_test_record("test", "poll-checkpoint-only", "run-checkpoint-only");
        runtime
            .managed_sources
            .store
            .upsert_source(&record, true)
            .await
            .unwrap();

        let checkpoint = crate::subscription_poll::PollCheckpointState {
            cursor: Some(json!(789)),
            watermark: Some(json!("2024-01-01T00:00:00Z")),
            tie_breaker: Some(json!("event-789")),
            seen_keys: VecDeque::from([json!("event-789").to_string()]),
            etag: Some("\"etag-v2\"".to_string()),
        };
        let offsets = runtime
            .managed_sources
            .store
            .append_events_and_store_poll_checkpoint(
                &record.namespace,
                &record.source_key,
                &record.run_id,
                &record.stream_id,
                &[],
                &checkpoint,
            )
            .await
            .unwrap();
        assert!(offsets.is_empty());

        let stored = runtime
            .managed_sources
            .store
            .load_poll_checkpoint(&record.namespace, &record.source_key, &record.run_id)
            .await
            .unwrap();
        assert_eq!(stored, Some(checkpoint));

        let page = runtime
            .stream_read(&ManagedStreamReadRequest {
                stream_id: record.stream_id.clone(),
                after_offset: 0,
                limit: 10,
            })
            .await
            .unwrap();
        assert!(page.events.is_empty());
    }

    #[tokio::test]
    async fn poll_event_writes_use_stream_offset_counter() {
        let temp = tempdir().unwrap();
        let runtime = test_runtime_with_store(&temp);
        let record = poll_checkpoint_test_record("test", "poll-offset-counter", "run-offset");
        runtime
            .managed_sources
            .store
            .upsert_source(&record, true)
            .await
            .unwrap();

        let checkpoint = crate::subscription_poll::PollCheckpointState {
            cursor: Some(json!(2)),
            ..Default::default()
        };
        let offsets = runtime
            .managed_sources
            .store
            .append_events_and_store_poll_checkpoint(
                &record.namespace,
                &record.source_key,
                &record.run_id,
                &record.stream_id,
                &[
                    PendingStreamEvent {
                        ingested_at_unix: now_unix_secs(),
                        payload: json!({"id": 1}),
                    },
                    PendingStreamEvent {
                        ingested_at_unix: now_unix_secs(),
                        payload: json!({"id": 2}),
                    },
                ],
                &checkpoint,
            )
            .await
            .unwrap();
        assert_eq!(offsets, vec![1, 2]);

        let db_path = managed_source_streams_db_path(temp.path());
        let next_event_offset: u64 = Connection::open(&db_path)
            .unwrap()
            .query_row(
                "select next_event_offset from event_streams where stream_id = ?1",
                [&record.stream_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(next_event_offset, 3);

        let checkpoint = crate::subscription_poll::PollCheckpointState {
            cursor: Some(json!(3)),
            ..Default::default()
        };
        let offsets = runtime
            .managed_sources
            .store
            .append_events_and_store_poll_checkpoint(
                &record.namespace,
                &record.source_key,
                &record.run_id,
                &record.stream_id,
                &[PendingStreamEvent {
                    ingested_at_unix: now_unix_secs(),
                    payload: json!({"id": 3}),
                }],
                &checkpoint,
            )
            .await
            .unwrap();
        assert_eq!(offsets, vec![3]);
    }

    #[tokio::test]
    async fn stream_read_reports_cursor_out_of_range_after_latest_offset() {
        let temp = tempdir().unwrap();
        let runtime = test_runtime_with_store(&temp);
        let record = poll_checkpoint_test_record("test", "poll-cursor-range", "run-cursor-range");
        runtime
            .managed_sources
            .store
            .upsert_source(&record, true)
            .await
            .unwrap();

        let checkpoint = crate::subscription_poll::PollCheckpointState {
            cursor: Some(json!(1)),
            ..Default::default()
        };
        runtime
            .managed_sources
            .store
            .append_events_and_store_poll_checkpoint(
                &record.namespace,
                &record.source_key,
                &record.run_id,
                &record.stream_id,
                &[PendingStreamEvent {
                    ingested_at_unix: now_unix_secs(),
                    payload: json!({"id": 1}),
                }],
                &checkpoint,
            )
            .await
            .unwrap();

        let zero_limit = runtime
            .managed_sources
            .store
            .read_stream(&record.stream_id, 0, 0)
            .await
            .unwrap();
        assert!(zero_limit.events.is_empty());
        assert_eq!(zero_limit.next_after_offset, 0);
        assert!(!zero_limit.has_more);

        let caught_up = runtime
            .stream_read(&ManagedStreamReadRequest {
                stream_id: record.stream_id.clone(),
                after_offset: 1,
                limit: 10,
            })
            .await
            .unwrap();
        assert!(caught_up.events.is_empty());
        assert_eq!(caught_up.next_after_offset, 1);

        let err = runtime
            .stream_read(&ManagedStreamReadRequest {
                stream_id: record.stream_id.clone(),
                after_offset: 2,
                limit: 10,
            })
            .await
            .unwrap_err();
        let structured =
            structured_error_from_anyhow(&err).expect("cursor error should be structured");
        assert_eq!(structured.code, "cursor_out_of_range");
        let details = structured
            .details
            .expect("cursor error should include details");
        assert_eq!(details["stream_id"], record.stream_id);
        assert_eq!(details["after_offset"], json!(2));
        assert_eq!(details["latest_offset"], json!(1));
    }

    #[tokio::test]
    async fn poll_checkpoint_write_failure_does_not_advance_checkpoint() {
        let temp = tempdir().unwrap();
        let runtime = test_runtime_with_store(&temp);
        let record = poll_checkpoint_test_record("test", "poll-atomic", "run-atomic");
        runtime
            .managed_sources
            .store
            .upsert_source(&record, true)
            .await
            .unwrap();

        let checkpoint = crate::subscription_poll::PollCheckpointState {
            cursor: Some(json!(456)),
            ..Default::default()
        };
        let err = runtime
            .managed_sources
            .store
            .append_events_and_store_poll_checkpoint(
                &record.namespace,
                &record.source_key,
                &record.run_id,
                "stream_wrong",
                &[PendingStreamEvent {
                    ingested_at_unix: now_unix_secs(),
                    payload: json!({"id": 456}),
                }],
                &checkpoint,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stream mismatch"));

        let stored = runtime
            .managed_sources
            .store
            .load_poll_checkpoint(&record.namespace, &record.source_key, &record.run_id)
            .await
            .unwrap();
        assert!(stored.is_none());

        let page = runtime
            .stream_read(&ManagedStreamReadRequest {
                stream_id: record.stream_id.clone(),
                after_offset: 0,
                limit: 10,
            })
            .await
            .unwrap();
        assert!(page.events.is_empty());
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

    #[tokio::test]
    async fn managed_source_list_and_daemon_status_expose_overview_counts() {
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
        runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "team".to_string(),
                source_key: "alpha".to_string(),
                spec: managed_source_spec(&endpoint_one),
            })
            .await
            .unwrap();
        runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "team".to_string(),
                source_key: "beta".to_string(),
                spec: managed_source_spec(&endpoint_two),
            })
            .await
            .unwrap();

        let mut sources = runtime.source_list().await.unwrap();
        sources.sort_by(|left, right| left.source_key.cmp(&right.source_key));
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].namespace, "team");
        assert_eq!(sources[0].source_key, "alpha");
        assert_eq!(sources[1].source_key, "beta");

        let mut status = runtime.status().await;
        for _ in 0..20 {
            if status.managed_sources_running == 2 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(100)).await;
            status = runtime.status().await;
        }
        assert_eq!(status.managed_sources, 2);
        assert_eq!(status.managed_sources_running, 2);
        assert_eq!(status.managed_streams, 2);

        runtime
            .source_stop(&ManagedSourceStatusRequest {
                namespace: "team".to_string(),
                source_key: "beta".to_string(),
            })
            .await
            .unwrap();

        let status_after_stop = runtime.status().await;
        assert_eq!(status_after_stop.managed_sources, 2);
        assert_eq!(status_after_stop.managed_sources_running, 1);
        assert_eq!(status_after_stop.managed_streams, 2);

        server_task_one.abort();
        server_task_two.abort();
    }

    #[tokio::test]
    async fn resume_managed_sources_does_not_create_duplicate_runners_or_legacy_store() {
        let temp = tempdir().unwrap();
        let (endpoint, _connects, server_task) =
            start_test_websocket_server(vec![TestWsConnectionPlan {
                frames: vec![TestWsFrame::Text(r#"{"value":"resume"}"#)],
                hold_open_after_send: true,
            }])
            .await;

        let runtime = test_runtime_with_store(&temp);
        runtime
            .source_ensure(ManagedSourceEnsureRequest {
                namespace: "team".to_string(),
                source_key: "resume-check".to_string(),
                spec: managed_source_spec(&endpoint),
            })
            .await
            .unwrap();

        for _ in 0..20 {
            if runtime.managed_sources.entries.lock().await.len() == 1 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
        assert_eq!(runtime.managed_sources.entries.lock().await.len(), 1);

        runtime.resume_managed_sources().await.unwrap();
        assert_eq!(runtime.managed_sources.entries.lock().await.len(), 1);
        assert!(!temp.path().join("subscriptions.json").exists());

        runtime
            .source_stop(&ManagedSourceStatusRequest {
                namespace: "team".to_string(),
                source_key: "resume-check".to_string(),
            })
            .await
            .unwrap();

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

struct DaemonOwnerLockGuard {
    file: fs::File,
}

impl DaemonOwnerLockGuard {
    fn acquire(path: &Path, metadata: &DaemonOwnerMetadata) -> Result<Self> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("Failed to open daemon owner lock {}", path.display()))?;

        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                return Err(StructuredError::new(
                    "DAEMON_OWNER_HELD",
                    format!(
                        "Another daemon owner already holds {}. Run `uxc daemon doctor` for diagnostics.",
                        path.display()
                    ),
                    Some(json!({
                        "lock_path": path.display().to_string(),
                    })),
                )
                .into());
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("Failed to lock daemon owner lock {}", path.display())
                });
            }
        }

        write_daemon_owner_metadata(&mut file, metadata)?;
        Ok(Self { file })
    }

    fn clear_metadata(&mut self) {
        let _ = clear_daemon_owner_metadata_file(&mut self.file);
    }
}

impl Drop for DaemonOwnerLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
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

fn write_daemon_owner_metadata(file: &mut fs::File, metadata: &DaemonOwnerMetadata) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&serde_json::to_vec(metadata)?)?;
    file.flush()?;
    Ok(())
}

fn clear_daemon_owner_metadata_file(file: &mut fs::File) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.flush()?;
    Ok(())
}

fn clear_daemon_owner_metadata_path(path: &Path) -> Result<()> {
    let mut file = match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    clear_daemon_owner_metadata_file(&mut file)
}

fn read_daemon_owner_metadata(path: &Path) -> Result<Option<DaemonOwnerMetadata>> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if contents.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&contents)?))
}

fn daemon_lock_is_held(path: &Path) -> Result<bool> {
    let file = match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    match file.try_lock_exclusive() {
        Ok(()) => {
            file.unlock()?;
            Ok(false)
        }
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(true),
        Err(err) => Err(err.into()),
    }
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: kill(pid, 0) is a pure liveness probe.
    let rc = unsafe { kill(pid as i32, 0) };
    if rc == 0 {
        return true;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(1) => true,  // EPERM
        Some(3) => false, // ESRCH
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    false
}

fn inspect_daemon_local_diagnostics() -> Result<DaemonLocalDiagnostics> {
    let lock_path = daemon_lock_path();
    let socket = socket_path();
    let metadata = read_daemon_owner_metadata(&lock_path)?;
    let owner_lock_held = daemon_lock_is_held(&lock_path)?;
    let owner_pid = metadata.as_ref().map(|m| m.pid);
    let owner_pid_alive = owner_pid.map(is_process_alive).unwrap_or(false);

    Ok(DaemonLocalDiagnostics {
        socket: socket.display().to_string(),
        socket_exists: socket.exists(),
        owner_lock_held,
        owner_pid,
        owner_pid_alive,
        owner_version: metadata.as_ref().map(|m| m.version.clone()),
        owner_socket: metadata.as_ref().map(|m| m.socket.clone()),
        owner_started_at_unix: metadata.as_ref().map(|m| m.started_at_unix),
    })
}

pub fn daemon_status_from_diagnostics(diagnostics: &DaemonLocalDiagnostics) -> DaemonStatus {
    DaemonStatus {
        running: false,
        pid: None,
        socket: diagnostics.socket.clone(),
        version: None,
        started_at_unix: None,
        request_count: 0,
        mcp_stdio_sessions: 0,
        mcp_http_sessions: 0,
        mcp_reuse_hits: 0,
        managed_sources: 0,
        managed_sources_running: 0,
        managed_streams: 0,
        log_file: None,
        owner_lock_held: Some(diagnostics.owner_lock_held),
        owner_pid: diagnostics.owner_pid,
        owner_pid_alive: Some(diagnostics.owner_pid_alive),
        owner_version: diagnostics.owner_version.clone(),
        owner_socket: diagnostics.owner_socket.clone(),
        owner_started_at_unix: diagnostics.owner_started_at_unix,
        socket_exists: Some(diagnostics.socket_exists),
    }
}

fn doctor_message_for_diagnostics(diagnostics: &DaemonLocalDiagnostics, status: &str) -> String {
    match status {
        "healthy" => "Daemon is healthy.".to_string(),
        "owner_unreachable" => format!(
            "Live daemon owner exists (pid={}) but the daemon socket is unreachable. Refusing repair; run `uxc daemon stop` or inspect the owner process.",
            diagnostics
                .owner_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
        "owner_held" => "Daemon owner lock is still held but owner metadata is stale or incomplete. Refusing repair; wait for the owner to exit or inspect the lock holder.".to_string(),
        "repaired" => "Removed stale daemon artifacts.".to_string(),
        _ => "No live daemon owner found.".to_string(),
    }
}

pub fn daemon_local_diagnostics() -> Result<DaemonLocalDiagnostics> {
    inspect_daemon_local_diagnostics()
}

fn daemon_doctor_local_blocking() -> Result<DaemonDoctorResponse> {
    if daemon_status_client_blocking().is_ok() {
        let diagnostics = inspect_daemon_local_diagnostics()?;
        return Ok(DaemonDoctorResponse {
            status: "healthy".to_string(),
            repaired: false,
            socket_removed: false,
            owner_metadata_cleared: false,
            socket: diagnostics.socket.clone(),
            diagnostics: diagnostics.clone(),
            message: Some(doctor_message_for_diagnostics(&diagnostics, "healthy")),
        });
    }

    let diagnostics = inspect_daemon_local_diagnostics()?;
    if diagnostics.owner_lock_held {
        let status = if diagnostics.owner_pid_alive {
            "owner_unreachable"
        } else {
            "owner_held"
        };
        return Ok(DaemonDoctorResponse {
            status: status.to_string(),
            repaired: false,
            socket_removed: false,
            owner_metadata_cleared: false,
            socket: diagnostics.socket.clone(),
            diagnostics: diagnostics.clone(),
            message: Some(doctor_message_for_diagnostics(&diagnostics, status)),
        });
    }

    let mut repaired = false;
    let mut socket_removed = false;
    let mut owner_metadata_cleared = false;
    let socket = socket_path();
    if socket.exists() {
        fs::remove_file(&socket).with_context(|| {
            format!("Failed to remove stale daemon socket {}", socket.display())
        })?;
        repaired = true;
        socket_removed = true;
    }

    let lock_path = daemon_lock_path();
    if lock_path.exists() {
        fs::remove_file(&lock_path).with_context(|| {
            format!(
                "Failed to remove stale daemon owner metadata {}",
                lock_path.display()
            )
        })?;
        repaired = true;
        owner_metadata_cleared = true;
    }

    let diagnostics = inspect_daemon_local_diagnostics()?;
    let status = if repaired { "repaired" } else { "clean" }.to_string();
    Ok(DaemonDoctorResponse {
        status: status.clone(),
        repaired,
        socket_removed,
        owner_metadata_cleared,
        socket: diagnostics.socket.clone(),
        diagnostics: diagnostics.clone(),
        message: Some(doctor_message_for_diagnostics(&diagnostics, &status)),
    })
}

#[cfg(unix)]
fn daemon_status_client_blocking() -> Result<DaemonStatus> {
    let socket = socket_path();
    let mut stream = StdUnixStream::connect(&socket)
        .with_context(|| format!("Failed to connect daemon socket {}", socket.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(CONNECT_TIMEOUT_SECS)))?;
    stream.set_write_timeout(Some(Duration::from_secs(CONNECT_TIMEOUT_SECS)))?;
    let request = json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": 1,
        "method": "daemon.status",
        "params": Value::Null,
    });
    write_frame_blocking(&mut stream, &request)?;
    let resp_val = read_frame_blocking(&mut stream)?;
    let resp: JsonRpcResponse = serde_json::from_value(resp_val)?;
    if let Some(err) = resp.error {
        bail!("{}", err.message);
    }
    Ok(serde_json::from_value(resp.result.unwrap_or(Value::Null))?)
}

#[cfg(not(unix))]
fn daemon_status_client_blocking() -> Result<DaemonStatus> {
    bail!("uxcd daemon is not supported on this platform; run uxc inside WSL")
}

#[cfg(unix)]
fn write_frame_blocking(stream: &mut StdUnixStream, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stream.write_all(header.as_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;
    Ok(())
}

#[cfg(unix)]
fn read_frame_blocking(stream: &mut StdUnixStream) -> Result<Value> {
    use std::io::Read;

    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let n = stream.read(&mut byte)?;
        if n == 0 {
            bail!("EOF while reading frame header");
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8(header)?;
    let mut content_len = None;
    for line in header_str.split("\r\n") {
        if let Some(rest) = line.trim().strip_prefix("Content-Length:") {
            content_len = Some(rest.trim().parse::<usize>()?);
        }
    }
    let len = content_len.ok_or_else(|| anyhow!("Missing Content-Length header"))?;
    let mut body = vec![0_u8; len];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
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
