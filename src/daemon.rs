use crate::adapters::{
    self, Adapter, AdapterEnum, DetectionOptions, Operation, ProtocolDetector, ProtocolType,
};
use crate::arg_coercion::prepare_execute_args;
use crate::auth::injected_env::{fingerprint_injected_env, render_injected_env, InjectEnvSpec};
use crate::auth::{self, Profile};
use crate::cache::{self, Cache, CacheConfig};
use crate::daemon_log::{redact_endpoint, redact_sensitive};
use crate::daemon_log::{DaemonEventType, DaemonLogEntry, DaemonLogger};
use crate::error::UxcError;
use crate::subscription_graphql::{
    derive_graphql_websocket_endpoint, graphql_transport_init_message, GraphQLSubscriptionConfig,
    GraphQLSubscriptionHandler,
};
use crate::subscription_jsonrpc::{
    derive_jsonrpc_unsubscribe_operation, JsonRpcSubscriptionConfig, JsonRpcSubscriptionHandler,
};
use crate::subscription_poll::{PollRuntimeContext, PollRuntimeObserver, PollSubscriptionConfig};
use crate::subscription_websocket::{
    self, RawFrameHandler, WebSocketRuntimeConfig, WebSocketRuntimeObserver,
};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
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
const MCP_STDIO_EXIT_TIMEOUT_SECS: u64 = 5;
const CONNECT_TIMEOUT_SECS: u64 = 2;
const FRAME_IO_TIMEOUT_SECS: u64 = 120;
const MAX_FRAME_BODY_BYTES: usize = 8 * 1024 * 1024;
const SUBSCRIPTION_HTTP_TIMEOUT_SECS: u64 = 300;
const SUBSCRIPTION_STOP_TIMEOUT_SECS: u64 = 5;
const SUBSCRIPTION_INITIAL_RECONNECT_DELAY_SECS: u64 = 1;
const SUBSCRIPTION_MAX_RECONNECT_DELAY_SECS: u64 = 30;
const SUBSCRIPTION_MAX_BUFFER_BYTES: usize = 1024 * 1024;
const ERR_PROTOCOL_DETECTION: i32 = -32010;
const ERR_OPERATION_NOT_FOUND: i32 = -32011;
const ERR_OAUTH_REQUIRED: i32 = -32012;
const ERR_OAUTH_REFRESH_FAILED: i32 = -32013;
const ERR_OAUTH_SCOPE_INSUFFICIENT: i32 = -32014;
const ERR_RUNTIME_GENERIC: i32 = -32030;

pub fn daemon_supported() -> bool {
    cfg!(unix)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAction {
    HostHelp,
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
    pub refresh_schema: bool,
    pub schema_url: Option<String>,
    pub link_name: Option<String>,
    pub schema_mapping_file: Option<String>,
    #[serde(default)]
    pub daemon_exclusive: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_hint: Option<SubscriptionTransportHint>,
    pub mode: SubscriptionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_config: Option<Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionTransportHint {
    Websocket,
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
    pub created_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub stopped_at_unix: Option<u64>,
    pub last_event_at_unix: Option<u64>,
    pub last_error: Option<String>,
    pub reconnect_count: u64,
    pub written_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubscriptionEventEnvelope {
    version: String,
    job_id: String,
    seq: u64,
    timestamp_unix: u64,
    protocol: String,
    source_kind: String,
    event_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
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
    stdio_init_locks: Arc<Mutex<HashMap<String, InitLockEntry>>>,
    stdio_exclusive_locks: Arc<Mutex<HashMap<String, InitLockEntry>>>,
    stdio_exclusive_owners: Arc<Mutex<HashMap<String, String>>>, // exclusive_key -> session_key
    stdio_session_exclusives: Arc<Mutex<HashMap<String, Vec<String>>>>, // session_key -> [exclusive_key]
    http: Arc<Mutex<HashMap<String, Arc<McpHttpSession>>>>,
    reuse_hits: Arc<Mutex<u64>>,
}

struct InitLockEntry {
    lock: Arc<Mutex<()>>,
    touched_at: Instant,
}

struct McpStdioSession {
    client: adapters::mcp::McpStdioClient,
    tools: Option<Vec<adapters::mcp::types::Tool>>,
    tools_dirty: bool,
    last_used: Instant,
}

struct McpHttpSession {
    transport: adapters::mcp::McpRemoteTransport,
    last_used: Arc<Mutex<Instant>>,
}

#[derive(Clone, Default)]
struct SubscriptionManager {
    jobs: Arc<Mutex<HashMap<String, Arc<SubscriptionJobEntry>>>>,
    next_id: Arc<Mutex<u64>>,
}

struct SubscriptionJobEntry {
    view: Arc<Mutex<SubscriptionJobView>>,
    stop_tx: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl McpStdioSession {
    async fn refresh_tools_if_needed(
        &mut self,
        _endpoint: &str,
        _cache: &Arc<dyn Cache>,
    ) -> Result<Vec<adapters::mcp::types::Tool>> {
        if self.tools.is_none() || self.tools_dirty {
            let tools = self.client.list_tools().await?;
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
        if self.client.take_tool_list_changed().await {
            self.tools_dirty = true;
            let _ = cache.invalidate(endpoint);
            return true;
        }
        false
    }
}

impl McpSessionManager {
    fn new() -> Self {
        Self {
            stdio: Arc::new(Mutex::new(HashMap::new())),
            stdio_init_locks: Arc::new(Mutex::new(HashMap::new())),
            stdio_exclusive_locks: Arc::new(Mutex::new(HashMap::new())),
            stdio_exclusive_owners: Arc::new(Mutex::new(HashMap::new())),
            stdio_session_exclusives: Arc::new(Mutex::new(HashMap::new())),
            http: Arc::new(Mutex::new(HashMap::new())),
            reuse_hits: Arc::new(Mutex::new(0)),
        }
    }

    async fn cleanup_idle(&self) {
        let cutoff = Instant::now() - Duration::from_secs(MCP_IDLE_TTL_SECS);

        let stdio_entries: Vec<(String, Arc<Mutex<McpStdioSession>>)> = {
            let map = self.stdio.lock().await;
            map.iter().map(|(k, s)| (k.clone(), s.clone())).collect()
        };
        let mut stdio_remove = Vec::new();
        for (key, session) in &stdio_entries {
            // Use try_lock to avoid blocking on sessions that may be held across .await in invoke_mcp.
            // If a session is busy, we'll check it again in the next cleanup cycle.
            if let Ok(mut guard) = session.try_lock() {
                if guard.last_used < cutoff {
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
            let last = *session.last_used.lock().await;
            if last < cutoff {
                http_remove.push(key.clone());
            }
        }
        if !http_remove.is_empty() {
            let mut map = self.http.lock().await;
            for key in http_remove {
                map.remove(&key);
            }
        }
    }

    async fn get_or_create_stdio(
        &self,
        session_key: &str,
        command: &str,
        args: &[String],
        spawn_options: &adapters::mcp::StdioSpawnOptions,
        exclusive_keys: &[String],
    ) -> Result<(Arc<Mutex<McpStdioSession>>, bool)> {
        let exclusive_keys = normalize_exclusive_keys(exclusive_keys);

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

        {
            let map = self.stdio.lock().await;
            if let Some(s) = map.get(session_key) {
                *self.reuse_hits.lock().await += 1;
                if !exclusive_keys.is_empty() {
                    self.register_stdio_exclusive_keys(session_key, &exclusive_keys)
                        .await;
                }
                return Ok((s.clone(), true));
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

        {
            let map = self.stdio.lock().await;
            if let Some(s) = map.get(session_key) {
                *self.reuse_hits.lock().await += 1;
                if !exclusive_keys.is_empty() {
                    self.register_stdio_exclusive_keys(session_key, &exclusive_keys)
                        .await;
                }
                return Ok((s.clone(), true));
            }
        }

        let client = adapters::mcp::McpStdioClient::connect_with_options(
            command,
            args,
            spawn_options.clone(),
        )
        .await?;
        let session = Arc::new(Mutex::new(McpStdioSession {
            client,
            tools: None,
            tools_dirty: false,
            last_used: Instant::now(),
        }));

        let mut map = self.stdio.lock().await;
        map.insert(session_key.to_string(), session.clone());
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

    async fn get_or_create_http(
        &self,
        key: &str,
        resolved: &adapters::mcp::ResolvedMcpHttpTransport,
        auth_profile: Option<Profile>,
    ) -> Result<(Arc<McpHttpSession>, bool)> {
        {
            let map = self.http.lock().await;
            if let Some(s) = map.get(key) {
                *self.reuse_hits.lock().await += 1;
                *s.last_used.lock().await = Instant::now();
                return Ok((s.clone(), true));
            }
        }

        let transport =
            adapters::mcp::McpRemoteTransport::with_auth(resolved.clone(), auth_profile)?;
        transport.initialize().await?;
        let session = Arc::new(McpHttpSession {
            transport,
            last_used: Arc::new(Mutex::new(Instant::now())),
        });

        let mut map = self.http.lock().await;
        map.insert(key.to_string(), session.clone());
        Ok((session, false))
    }

    async fn status_counts(&self) -> (usize, usize, u64) {
        let stdio_count = self.stdio.lock().await.len();
        let http_count = self.http.lock().await.len();
        let reuse_hits = *self.reuse_hits.lock().await;
        (stdio_count, http_count, reuse_hits)
    }
}

fn resolve_stream_subscription_protocol(request: &SubscribeStartRequest) -> Result<String> {
    if matches!(
        request.transport_hint,
        Some(SubscriptionTransportHint::Websocket)
    ) {
        let lower = request.endpoint.to_ascii_lowercase();
        if !lower.starts_with("ws://") && !lower.starts_with("wss://") {
            bail!("websocket subscription transport requires a ws:// or wss:// endpoint");
        }
        return Ok("websocket".to_string());
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
            derive_jsonrpc_unsubscribe_operation(operation_id)?;
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
    fn new() -> Self {
        Self::default()
    }

    async fn start(
        &self,
        runtime: &DaemonRuntime,
        request: &SubscribeStartRequest,
    ) -> Result<SubscribeStartResponse> {
        let sink_path = parse_file_sink(&request.sink)?;
        let sink_spec = format!("file:{}", sink_path.display());
        if request.resource_uri.is_some() && request.operation_id.is_some() {
            bail!("subscribe start cannot combine --resource-uri with an operation_id");
        }
        if request.args.is_some() && request.operation_id.is_none() {
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

        let job_id = {
            let mut next = self.next_id.lock().await;
            *next += 1;
            format!("sub_{}", *next)
        };
        let now = now_unix_secs();
        let view = Arc::new(Mutex::new(SubscriptionJobView {
            job_id: job_id.clone(),
            mode: request.mode,
            endpoint: request.endpoint.clone(),
            protocol: protocol.clone(),
            sink: sink_spec.clone(),
            resource_uri: request.resource_uri.clone(),
            status: "running".to_string(),
            created_at_unix: now,
            started_at_unix: Some(now),
            stopped_at_unix: None,
            last_event_at_unix: None,
            last_error: None,
            reconnect_count: 0,
            written_events: 0,
        }));
        let (stop_tx, stop_rx) = watch::channel(false);
        let request_clone = request.clone();
        let job_id_clone = job_id.clone();
        let view_clone = view.clone();
        let runtime_clone = runtime.clone();
        let task = tokio::spawn(async move {
            let result = match request_clone.mode {
                SubscriptionMode::Stream => {
                    run_stream_subscription_job(
                        &job_id_clone,
                        &request_clone,
                        sink_path,
                        view_clone.clone(),
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
                        view_clone.clone(),
                        stop_rx,
                    )
                    .await
                }
            };

            let mut guard = view_clone.lock().await;
            if guard.status != "stopped" {
                match result {
                    Ok(()) => {
                        guard.status = "stopped".to_string();
                    }
                    Err(err) => {
                        guard.status = "failed".to_string();
                        guard.last_error = Some(err.to_string());
                    }
                }
            }
            guard.stopped_at_unix = Some(now_unix_secs());
        });
        let entry = Arc::new(SubscriptionJobEntry {
            view: view.clone(),
            stop_tx,
            task: Mutex::new(Some(task)),
        });
        self.jobs.lock().await.insert(job_id.clone(), entry);

        let guard = view.lock().await;
        Ok(SubscribeStartResponse {
            job_id,
            mode: request.mode,
            protocol,
            endpoint: guard.endpoint.clone(),
            sink: sink_spec,
            resource_uri: guard.resource_uri.clone(),
            status: guard.status.clone(),
        })
    }

    async fn list(&self) -> Vec<SubscriptionJobView> {
        let entries = {
            let jobs = self.jobs.lock().await;
            jobs.values().cloned().collect::<Vec<_>>()
        };
        let mut views = Vec::with_capacity(entries.len());
        for entry in entries {
            views.push(entry.view.lock().await.clone());
        }
        views.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        views
    }

    async fn status(&self, job_id: &str) -> Result<SubscriptionJobView> {
        let entry = {
            let jobs = self.jobs.lock().await;
            jobs.get(job_id).cloned()
        }
        .ok_or_else(|| {
            UxcError::OperationNotFound(format!("subscription job not found: {}", job_id))
        })?;
        let view = entry.view.lock().await.clone();
        Ok(view)
    }

    async fn stop(&self, job_id: &str) -> Result<SubscribeStopResponse> {
        let entry = {
            let jobs = self.jobs.lock().await;
            jobs.get(job_id).cloned()
        }
        .ok_or_else(|| {
            UxcError::OperationNotFound(format!("subscription job not found: {}", job_id))
        })?;

        let _ = entry.stop_tx.send(true);
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
        {
            let mut guard = entry.view.lock().await;
            guard.status = "stopped".to_string();
            guard.stopped_at_unix = Some(now_unix_secs());
        }
        Ok(SubscribeStopResponse {
            job_id: job_id.to_string(),
            stopped: true,
        })
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
    should_stop: Arc<RwLock<bool>>,
    schema_mapping_lock: Arc<Mutex<()>>,
    logger: Option<DaemonLogger>,
}

impl DaemonRuntime {
    pub fn new() -> Self {
        let logger = Self::initialize_logger();
        Self {
            state: Arc::new(Mutex::new(ServerState {
                started_at_unix: now_unix_secs(),
                request_count: 0,
            })),
            mcp: McpSessionManager::new(),
            subscriptions: SubscriptionManager::new(),
            should_stop: Arc::new(RwLock::new(false)),
            schema_mapping_lock: Arc::new(Mutex::new(())),
            logger,
        }
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
        };

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

        let mut result: Result<(String, Option<String>, Value)> = if protocol == "mcp"
            && matches!(request.action, RuntimeAction::Execute)
        {
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

            Ok((kind, operation, data))
        } else if protocol == "mcp" {
            if let Some(live_result) = invoke_live_stdio_mcp_help(
                self,
                &request,
                execution_auth_profile.as_ref(),
                cache_for_mcp.clone(),
            )
            .await?
            {
                Ok(live_result)
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
                            Ok((kind, operation, data))
                        } else {
                            invoke_with_adapter(&adapter, &request).await
                        };
                    }
                }
            }
        }

        match result {
            Ok((kind, operation, data)) => {
                let duration_ms = start.elapsed().as_millis() as u64;
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
        if matches!(
            request.transport_hint,
            Some(SubscriptionTransportHint::Websocket)
        ) {
            bail!("poll subscriptions do not support websocket transport hints");
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
        let arguments = if args.is_empty() {
            None
        } else {
            Some(Value::Object(args.into_iter().collect()))
        };

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
                    &request.options.daemon_exclusive,
                )
                .await?;
            let mut guard = session.lock().await;
            guard.last_used = Instant::now();
            let result = guard.client.call_tool(op, arguments).await?;
            let _ = guard
                .mark_tools_dirty_from_notifications(endpoint, &cache)
                .await;
            Ok((
                "call_result".to_string(),
                Some(op.clone()),
                adapters::mcp::convert_tool_result_to_value(&result),
                reused,
            ))
        } else {
            let resolved_transport =
                resolve_mcp_http_endpoint(endpoint, auth_profile.clone()).await?;
            let key = format!(
                "http:{:?}:{}:{}",
                resolved_transport.mode,
                resolved_transport.connect_url,
                auth_fingerprint(auth_profile.as_ref())
            );
            let (session, reused) = self
                .mcp
                .get_or_create_http(&key, &resolved_transport, auth_profile)
                .await?;
            *session.last_used.lock().await = Instant::now();
            let result = session.transport.call_tool(op, arguments).await?;
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
        bail!("subscribe sink must use file:<path>");
    };
    if path.trim().is_empty() {
        bail!("subscribe sink path cannot be empty");
    }
    let path = PathBuf::from(path);
    validate_subscription_sink_path(&path)?;
    Ok(path)
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
    if path.is_absolute() {
        let allowed_roots = [
            std::env::var_os("HOME").map(PathBuf::from),
            Some(std::env::temp_dir()),
        ];
        let allowed = allowed_roots
            .into_iter()
            .flatten()
            .any(|root| path.starts_with(&root));
        if !allowed {
            bail!("absolute subscribe sink path must be under HOME or temp directory");
        }
    }
    Ok(())
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

async fn run_stream_subscription_job(
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
        return run_mcp_subscription_job(job_id, request, sink_path, view, stop_rx).await;
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
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
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

fn resolve_jsonrpc_subscription_config(
    request: &SubscribeStartRequest,
) -> Result<JsonRpcSubscriptionConfig> {
    let operation_id = request
        .operation_id
        .as_ref()
        .ok_or_else(|| anyhow!("operation_id is required for JSON-RPC subscriptions"))?;
    let unsubscribe_operation_id = derive_jsonrpc_unsubscribe_operation(operation_id)?;
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
    let mut handler = GraphQLSubscriptionHandler::new(GraphQLSubscriptionConfig {
        operation_id: operation_id.clone(),
        query: prepared.query,
        variables: prepared.variables,
    });
    let mut observer = DaemonWebSocketObserver {
        sink: &mut sink,
        view: &view,
        seq: &mut seq,
        source_kind: "graphql",
    };

    let result = subscription_websocket::run_websocket_subscription_runtime(
        WebSocketRuntimeConfig {
            endpoint,
            auth_profile,
            subprotocols: vec!["graphql-transport-ws".to_string()],
            initial_text_frames: vec![graphql_transport_init_message()],
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
    ) -> Result<crate::subscription_poll::PollFetchResult> {
        let response = self
            .runtime
            .invoke(RuntimeInvokeRequest {
                request_id: format!("{}-poll-{}", self.request.request_id, now_unix_secs()),
                endpoint: self.request.endpoint.clone(),
                action: RuntimeAction::Execute,
                operation_id: self.request.operation_id.clone(),
                args: Some(args),
                options: self.request.options.clone(),
            })
            .await?;
        Ok(crate::subscription_poll::PollFetchResult {
            data: response.data,
            duration_ms: response.duration_ms,
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
        return run_mcp_http_subscription_job(request, sink_path, view, resource_uri, stop_rx)
            .await;
    }
    if !adapters::mcp::McpAdapter::is_stdio_command(&request.endpoint) {
        bail!("MCP subscriptions require a stdio command or http(s) MCP endpoint");
    }
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let spawn_options =
        build_stdio_spawn_options(&request.endpoint, &request.options, auth_profile.as_ref())?
            .unwrap_or_default();
    let (cmd, cmd_args) = adapters::mcp::McpAdapter::parse_stdio_command(&request.endpoint)?;
    let mut client =
        adapters::mcp::McpStdioClient::connect_with_options(&cmd, &cmd_args, spawn_options).await?;
    if !client.supports_resource_subscribe() {
        bail!("MCP server does not support resources.subscribe");
    }
    client.subscribe_resource(resource_uri).await?;

    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
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

    loop {
        tokio::select! {
            stop_requested = subscription_stop_requested(&mut stop_rx) => {
                if stop_requested {
                    if let Err(err) = client.unsubscribe_resource(resource_uri).await {
                        let msg = format!("failed to unsubscribe resource before shutdown: {}", err);
                        append_subscription_event(
                            &mut sink,
                            &view,
                            &mut seq,
                            "mcp_resource",
                            "error",
                            None,
                            Some(json!({ "message": msg })),
                        ).await?;
                        update_subscription_view(&view, None, Some(msg), false).await;
                    }
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "mcp_resource").await?;
                    return Ok(());
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let notifications = client.drain_notifications().await;
                for notification in notifications {
                    append_subscription_event(
                        &mut sink,
                        &view,
                        &mut seq,
                        "mcp_resource",
                        "data",
                        notification.params.clone(),
                        Some(json!({"method": notification.method})),
                    ).await?;
                }
            }
        }
    }
}

async fn run_mcp_http_subscription_job(
    request: &SubscribeStartRequest,
    sink_path: PathBuf,
    view: Arc<Mutex<SubscriptionJobView>>,
    resource_uri: &str,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let auth_profile =
        auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;
    let resolved_transport =
        resolve_mcp_http_endpoint(&request.endpoint, auth_profile.clone()).await?;
    let transport =
        adapters::mcp::McpRemoteTransport::with_auth(resolved_transport.clone(), auth_profile)?;
    let init = transport.initialize().await?;
    let supports_resource_subscribe = init
        .capabilities
        .resources
        .as_ref()
        .and_then(|resources| resources.subscribe)
        .unwrap_or(false);
    if !supports_resource_subscribe {
        bail!("MCP server does not support resources.subscribe");
    }

    transport.subscribe_resource(resource_uri).await?;

    let mut sink = open_subscription_sink(&sink_path).await?;
    let mut seq = 0u64;
    append_subscription_event(
        &mut sink,
        &view,
        &mut seq,
        "mcp_resource",
        "open",
        None,
        Some(json!({
            "resource_uri": resource_uri,
            "transport_mode": format!("{:?}", resolved_transport.mode),
            "connect_url": redact_endpoint(&resolved_transport.connect_url),
        })),
    )
    .await?;

    loop {
        tokio::select! {
            stop_requested = subscription_stop_requested(&mut stop_rx) => {
                if stop_requested {
                    if let Err(err) = transport.unsubscribe_resource(resource_uri).await {
                        let msg = format!("failed to unsubscribe resource before shutdown: {}", err);
                        append_subscription_event(
                            &mut sink,
                            &view,
                            &mut seq,
                            "mcp_resource",
                            "error",
                            None,
                            Some(json!({ "message": msg })),
                        ).await?;
                        update_subscription_view(&view, None, Some(msg), false).await;
                    }
                    transport.shutdown_notification_stream().await;
                    close_subscription_as_stopped(&mut sink, &view, &mut seq, "mcp_resource").await?;
                    return Ok(());
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if let Some(err) = transport.take_stream_error().await {
                    transport.shutdown_notification_stream().await;
                    return Err(anyhow!("MCP HTTP subscription stream failed: {}", err));
                }
                for notification in transport.drain_notifications().await {
                    append_subscription_event(
                        &mut sink,
                        &view,
                        &mut seq,
                        "mcp_resource",
                        "data",
                        notification.params.clone(),
                        Some(json!({"method": notification.method})),
                    ).await?;
                }
            }
        }
    }
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
async fn start_daemon_process() -> Result<()> {
    let dir = daemon_dir();
    ensure_private_dir(&dir)?;
    let lock_path = dir.join("start.lock");
    let start_lock = try_acquire_start_lock(&lock_path)?;
    let got_lock = start_lock.is_some();

    if got_lock {
        let current_exe = std::env::current_exe().context("Cannot resolve current executable")?;
        let _child = std::process::Command::new(current_exe)
            .arg("daemon")
            .arg("_serve")
            // Avoid corrupting coverage artifacts when parent test runners
            // terminate long-lived daemon processes in CI.
            .env_remove("LLVM_PROFILE_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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

    let runtime = Arc::new(DaemonRuntime::new());

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
                    error: Some(JsonRpcError {
                        code: map_runtime_error_code(&err),
                        message: err.to_string(),
                    }),
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
                    error: Some(JsonRpcError {
                        code: map_runtime_error_code(&err),
                        message: err.to_string(),
                    }),
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
                    error: Some(JsonRpcError {
                        code: map_runtime_error_code(&err),
                        message: err.to_string(),
                    }),
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
                    error: Some(JsonRpcError {
                        code: map_runtime_error_code(&err),
                        message: err.to_string(),
                    }),
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
            }),
        },
    };

    write_frame(&mut stream, &serde_json::to_value(response)?).await?;
    Ok(())
}

pub async fn daemon_status_local() -> Result<DaemonStatus> {
    daemon_status_client().await
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
                .with_schema_url_override(options.schema_url.clone()),
        ),
        ProtocolType::GRpc => AdapterEnum::GRpc(adapters::grpc::GrpcAdapter::new()),
        ProtocolType::JsonRpc => AdapterEnum::JsonRpc(adapters::jsonrpc::JsonRpcAdapter::new()),
        ProtocolType::Mcp => {
            let mut adapter = adapters::mcp::McpAdapter::new();
            if let Some(spawn_options) = options.stdio_spawn_options.clone() {
                adapter = adapter.with_stdio_spawn_options(spawn_options);
            }
            AdapterEnum::Mcp(adapter)
        }
        ProtocolType::GraphQL => AdapterEnum::GraphQL(adapters::graphql::GraphQLAdapter::new()),
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
) -> Result<(String, Option<String>, Value)> {
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
            if let Some(service) = service {
                payload["service"] = serde_json::to_value(service)?;
            }
            Ok(("host_help".to_string(), None, payload))
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
            ))
        }
        RuntimeAction::Execute => {
            let op = request
                .operation_id
                .as_ref()
                .ok_or_else(|| anyhow!("operation_id is required"))?;
            let args = prepare_runtime_execute_args(adapter, request).await?;
            let result = adapter.execute(&request.endpoint, op, args).await?;
            Ok(("call_result".to_string(), Some(op.clone()), result.data))
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
        RuntimeAction::HostHelp | RuntimeAction::OperationHelp
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

    let mut guard = session.lock().await;
    guard.last_used = Instant::now();
    let _ = guard
        .mark_tools_dirty_from_notifications(&request.endpoint, &cache)
        .await;
    let tools = guard
        .refresh_tools_if_needed(&request.endpoint, &cache)
        .await?;

    match request.action {
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
            let service = live_stdio_service_summary(&guard.client);
            if let Some(service) = service {
                payload["service"] = serde_json::to_value(service)?;
            }
            Ok(Some(("host_help".to_string(), None, payload)))
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
    }
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
                refresh_schema: false,
                schema_url: Some("https://example.com/schema.json".to_string()),
                link_name: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
            },
        };

        assert_eq!(
            openapi_runtime_endpoint(&request).as_deref(),
            Some("https://testnet.binance.vision/api/v3/account")
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
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                refresh_schema: false,
                schema_url: Some("https://example.com/schema.json".to_string()),
                link_name: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
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
    fn parse_file_sink_rejects_absolute_path_outside_allowed_roots() {
        let err = parse_file_sink("file:/etc/passwd").unwrap_err();
        assert!(err.to_string().contains("under HOME or temp directory"));
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
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
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
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
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
            transport_hint: Some(SubscriptionTransportHint::Websocket),
            mode: SubscriptionMode::Stream,
            poll_config: None,
            options: RuntimeInvokeOptions {
                auth: None,
                inject_env: Vec::new(),
                no_cache: false,
                cache_ttl: None,
                refresh_schema: false,
                schema_url: None,
                link_name: None,
                schema_mapping_file: None,
                daemon_exclusive: Vec::new(),
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

        let runtime = DaemonRuntime::new();
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

        let runtime = DaemonRuntime::new();
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
        error: Some(JsonRpcError { code, message }),
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
