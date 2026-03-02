use crate::adapters::{
    self, Adapter, AdapterEnum, DetectionOptions, Operation, OperationDetail, ProtocolDetector,
    ProtocolType,
};
use crate::auth::{self, Profile};
use crate::cache::{self, Cache, CacheConfig};
use crate::daemon_log::{DaemonEventType, DaemonLogEntry, DaemonLogger};
use crate::error::UxcError;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, RwLock};

const JSONRPC_VERSION: &str = "2.0";
const START_POLL_TRIES: usize = 30;
const START_POLL_INTERVAL_MS: u64 = 100;
const STOP_POLL_TRIES: usize = 50;
const STOP_POLL_INTERVAL_MS: u64 = 100;
const START_LOCK_STALE_SECS: u64 = 30;
const STDIO_INIT_LOCK_STALE_SECS: u64 = 30;
const MCP_IDLE_TTL_SECS: u64 = 600;
const CONNECT_TIMEOUT_SECS: u64 = 2;
const FRAME_IO_TIMEOUT_SECS: u64 = 120;
const MAX_FRAME_BODY_BYTES: usize = 8 * 1024 * 1024;
const ERR_PROTOCOL_DETECTION: i32 = -32010;
const ERR_OPERATION_NOT_FOUND: i32 = -32011;
const ERR_OAUTH_REQUIRED: i32 = -32012;
const ERR_OAUTH_REFRESH_FAILED: i32 = -32013;
const ERR_OAUTH_SCOPE_INSUFFICIENT: i32 = -32014;
const ERR_RUNTIME_GENERIC: i32 = -32030;

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
    pub no_cache: bool,
    pub cache_ttl: Option<u64>,
    pub refresh_schema: bool,
    pub schema_url: Option<String>,
    pub link_name: Option<String>,
    pub schema_mapping_file: Option<String>,
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
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub socket: String,
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
    http: Arc<Mutex<HashMap<String, Arc<McpHttpSession>>>>,
    reuse_hits: Arc<Mutex<u64>>,
}

struct InitLockEntry {
    lock: Arc<Mutex<()>>,
    touched_at: Instant,
}

struct McpStdioSession {
    client: adapters::mcp::McpStdioClient,
    last_used: Instant,
}

struct McpHttpSession {
    transport: adapters::mcp::McpHttpTransport,
    last_used: Arc<Mutex<Instant>>,
    init_result: adapters::mcp::types::InitializeResult,
}

impl McpSessionManager {
    fn new() -> Self {
        Self {
            stdio: Arc::new(Mutex::new(HashMap::new())),
            stdio_init_locks: Arc::new(Mutex::new(HashMap::new())),
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
            if let Ok(guard) = session.try_lock() {
                if guard.last_used < cutoff {
                    stdio_remove.push(key.clone());
                }
            }
        }
        if !stdio_remove.is_empty() {
            let mut map = self.stdio.lock().await;
            for key in stdio_remove {
                map.remove(&key);
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
        key: &str,
        command: &str,
        args: &[String],
    ) -> Result<(Arc<Mutex<McpStdioSession>>, bool)> {
        {
            let map = self.stdio.lock().await;
            if let Some(s) = map.get(key) {
                *self.reuse_hits.lock().await += 1;
                return Ok((s.clone(), true));
            }
        }

        // Singleflight for stdio process initialization by endpoint key.
        // This avoids duplicate process spawns under concurrent cold requests.
        let key_lock = {
            let mut lock_map = self.stdio_init_locks.lock().await;
            let entry = lock_map
                .entry(key.to_string())
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
            if let Some(s) = map.get(key) {
                *self.reuse_hits.lock().await += 1;
                return Ok((s.clone(), true));
            }
        }

        let client = adapters::mcp::McpStdioClient::connect(command, args).await?;
        let session = Arc::new(Mutex::new(McpStdioSession {
            client,
            last_used: Instant::now(),
        }));

        let mut map = self.stdio.lock().await;
        map.insert(key.to_string(), session.clone());
        Ok((session, false))
    }

    async fn get_or_create_http(
        &self,
        key: &str,
        endpoint: &str,
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
            adapters::mcp::McpHttpTransport::with_auth(endpoint.to_string(), auth_profile)?;
        let init_result = transport.initialize().await?;
        let session = Arc::new(McpHttpSession {
            transport,
            last_used: Arc::new(Mutex::new(Instant::now())),
            init_result,
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
        let auth_profile =
            auth::resolve_auth_for_endpoint(&request.endpoint, request.options.auth.clone())?;

        let detection_options = DetectionOptions {
            schema_url: request.options.schema_url.clone(),
            auth_profile: auth_profile.clone(),
        };

        let resolved = resolve_adapter_with_schema_cache(
            &request.endpoint,
            &detection_options,
            cache,
            auth_profile.clone(),
            request.options.no_cache,
            request.options.refresh_schema,
        )
        .await;

        let resolved = match resolved {
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

        let protocol = resolved.adapter.protocol_type().as_str().to_string();
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

        let result: Result<(String, Option<String>, Value)> = if protocol == "mcp" {
            let (kind, operation, data, reused) = self.invoke_mcp(&request, auth_profile).await?;
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
        } else {
            invoke_with_adapter(&resolved.adapter, &request).await
        };

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
            started_at_unix: Some(state.started_at_unix),
            request_count: state.request_count,
            mcp_stdio_sessions: stdio_sessions,
            mcp_http_sessions: http_sessions,
            mcp_reuse_hits: reuse_hits,
            log_file,
        }
    }

    pub async fn request_stop(&self) {
        let mut stop = self.should_stop.write().await;
        *stop = true;
        let _ = UnixStream::connect(socket_path()).await;
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

    async fn invoke_mcp(
        &self,
        request: &RuntimeInvokeRequest,
        auth_profile: Option<Profile>,
    ) -> Result<(String, Option<String>, Value, bool)> {
        let endpoint = &request.endpoint;
        if adapters::mcp::McpAdapter::is_stdio_command(endpoint) {
            let (cmd, cmd_args) = adapters::mcp::McpAdapter::parse_stdio_command(endpoint)?;
            let key = format!(
                "stdio:{}:{}",
                endpoint,
                auth_fingerprint(auth_profile.as_ref())
            );
            let (session, reused) = self.mcp.get_or_create_stdio(&key, &cmd, &cmd_args).await?;
            let mut guard = session.lock().await;
            guard.last_used = Instant::now();
            match request.action {
                RuntimeAction::HostHelp => {
                    let operations = guard
                        .client
                        .list_tools()
                        .await?
                        .into_iter()
                        .map(tool_to_operation)
                        .collect::<Vec<_>>();
                    let protocol = "mcp";
                    let summaries = operations
                        .iter()
                        .map(|op| to_operation_summary(protocol, op))
                        .collect::<Vec<_>>();
                    let service = Some(ServiceSummary {
                        name: guard.client.server_info().map(|i| i.name.clone()),
                        description: guard.client.instructions().map(ToString::to_string),
                    });
                    Ok((
                        "host_help".to_string(),
                        None,
                        serde_json::to_value(json!({
                            "operations": summaries,
                            "count": summaries.len(),
                            "examples": host_help_examples(request.options.link_name.as_deref()),
                            "service": service
                        }))?,
                        reused,
                    ))
                }
                RuntimeAction::OperationHelp => {
                    let op = request
                        .operation_id
                        .as_ref()
                        .ok_or_else(|| anyhow!("operation_id is required"))?;
                    let tools = guard.client.list_tools().await?;
                    let tool = tools
                        .into_iter()
                        .find(|t| t.name == *op)
                        .ok_or_else(|| UxcError::OperationNotFound(op.clone()))?;
                    let detail = tool_to_operation_detail(tool);
                    Ok((
                        "operation_detail".to_string(),
                        Some(op.clone()),
                        serde_json::to_value(detail)?,
                        reused,
                    ))
                }
                RuntimeAction::Execute => {
                    let op = request
                        .operation_id
                        .as_ref()
                        .ok_or_else(|| anyhow!("operation_id is required"))?;
                    let args = request.args.clone().unwrap_or_default();
                    let tools = guard.client.list_tools().await?;
                    validate_mcp_tool_args(op, &tools, &args)?;
                    let arguments = if args.is_empty() {
                        None
                    } else {
                        Some(Value::Object(args.into_iter().collect()))
                    };
                    let result = guard.client.call_tool(op, arguments).await?;
                    Ok((
                        "call_result".to_string(),
                        Some(op.clone()),
                        convert_tool_content_to_value(&result.content),
                        reused,
                    ))
                }
            }
        } else {
            let resolved_endpoint =
                resolve_mcp_http_endpoint(endpoint, auth_profile.clone()).await?;
            let key = format!(
                "http:{}:{}",
                resolved_endpoint,
                auth_fingerprint(auth_profile.as_ref())
            );
            let (session, reused) = self
                .mcp
                .get_or_create_http(&key, &resolved_endpoint, auth_profile)
                .await?;
            *session.last_used.lock().await = Instant::now();

            match request.action {
                RuntimeAction::HostHelp => {
                    let operations = session
                        .transport
                        .list_tools()
                        .await?
                        .into_iter()
                        .map(tool_to_operation)
                        .collect::<Vec<_>>();
                    let protocol = "mcp";
                    let summaries = operations
                        .iter()
                        .map(|op| to_operation_summary(protocol, op))
                        .collect::<Vec<_>>();
                    let service = Some(ServiceSummary {
                        name: session
                            .init_result
                            .serverInfo
                            .as_ref()
                            .map(|i| i.name.clone()),
                        description: session.init_result.instructions.clone(),
                    });
                    Ok((
                        "host_help".to_string(),
                        None,
                        serde_json::to_value(json!({
                            "operations": summaries,
                            "count": summaries.len(),
                            "examples": host_help_examples(request.options.link_name.as_deref()),
                            "service": service
                        }))?,
                        reused,
                    ))
                }
                RuntimeAction::OperationHelp => {
                    let op = request
                        .operation_id
                        .as_ref()
                        .ok_or_else(|| anyhow!("operation_id is required"))?;
                    let tools = session.transport.list_tools().await?;
                    let tool = tools
                        .into_iter()
                        .find(|t| t.name == *op)
                        .ok_or_else(|| UxcError::OperationNotFound(op.clone()))?;
                    let detail = tool_to_operation_detail(tool);
                    Ok((
                        "operation_detail".to_string(),
                        Some(op.clone()),
                        serde_json::to_value(detail)?,
                        reused,
                    ))
                }
                RuntimeAction::Execute => {
                    let op = request
                        .operation_id
                        .as_ref()
                        .ok_or_else(|| anyhow!("operation_id is required"))?;
                    let args = request.args.clone().unwrap_or_default();
                    let tools = session.transport.list_tools().await?;
                    validate_mcp_tool_args(op, &tools, &args)?;
                    let arguments = if args.is_empty() {
                        None
                    } else {
                        Some(Value::Object(args.into_iter().collect()))
                    };
                    let result = session.transport.call_tool(op, arguments).await?;
                    Ok((
                        "call_result".to_string(),
                        Some(op.clone()),
                        convert_tool_content_to_value(&result.content),
                        reused,
                    ))
                }
            }
        }
    }
}

pub async fn daemon_status_client() -> Result<DaemonStatus> {
    let value = client_call("daemon.status", None).await?;
    Ok(serde_json::from_value(value)?)
}

pub async fn daemon_stop_client() -> Result<()> {
    let _ = client_call("daemon.shutdown", None).await?;
    Ok(())
}

pub async fn runtime_invoke_client(
    request: &RuntimeInvokeRequest,
) -> Result<RuntimeInvokeResponse> {
    let params = serde_json::to_value(request)?;
    let value = client_call("runtime.invoke", Some(params)).await?;
    Ok(serde_json::from_value(value)?)
}

pub async fn ensure_daemon_running() -> Result<bool> {
    if daemon_status_client().await.is_ok() {
        return Ok(false);
    }

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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn daemon process")?;
    }

    for _ in 0..START_POLL_TRIES {
        tokio::time::sleep(Duration::from_millis(START_POLL_INTERVAL_MS)).await;
        if daemon_status_client().await.is_ok() {
            return Ok(true);
        }
    }

    drop(start_lock);
    bail!("Daemon failed to start. Run `uxc daemon status` for diagnostics.")
}

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

pub async fn daemon_start_local() -> Result<bool> {
    ensure_daemon_running().await
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
    }
    format!("{:x}", hasher.finalize())
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
        ProtocolType::Mcp => AdapterEnum::Mcp(adapters::mcp::McpAdapter::new()),
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
            Ok((
                "host_help".to_string(),
                None,
                json!({
                    "operations": summaries,
                    "count": summaries.len(),
                    "examples": host_help_examples(request.options.link_name.as_deref()),
                }),
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
            ))
        }
        RuntimeAction::Execute => {
            let op = request
                .operation_id
                .as_ref()
                .ok_or_else(|| anyhow!("operation_id is required"))?;
            let args = request.args.clone().unwrap_or_default();
            let result = adapter.execute(&request.endpoint, op, args).await?;
            Ok(("call_result".to_string(), Some(op.clone()), result.data))
        }
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

fn tool_to_operation(tool: adapters::mcp::types::Tool) -> Operation {
    let parameters = if let Some(schema) = tool.inputSchema {
        parse_schema_to_parameters(&schema)
    } else {
        Vec::new()
    };

    Operation {
        operation_id: tool.name.clone(),
        display_name: tool.name.clone(),
        description: Some(tool.description),
        parameters,
        return_type: Some("ToolContent".to_string()),
    }
}

fn tool_to_operation_detail(tool: adapters::mcp::types::Tool) -> OperationDetail {
    OperationDetail {
        operation_id: tool.name.clone(),
        display_name: tool.name,
        description: Some(tool.description),
        parameters: tool
            .inputSchema
            .as_ref()
            .map(parse_schema_to_parameters)
            .unwrap_or_default(),
        return_type: Some("ToolContent".to_string()),
        input_schema: tool.inputSchema,
    }
}

fn parse_schema_to_parameters(schema: &Value) -> Vec<adapters::Parameter> {
    let mut parameters = Vec::new();

    if let Some(obj) = schema.as_object() {
        if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
            let required = obj
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<std::collections::HashSet<_>>()
                })
                .unwrap_or_default();

            for (name, prop_schema) in props {
                let param_type = prop_schema
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let description = prop_schema
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                parameters.push(adapters::Parameter {
                    name: name.clone(),
                    param_type,
                    required: required.contains(name.as_str()),
                    description,
                });
            }
        }
    }

    parameters
}

fn convert_tool_content_to_value(content: &[adapters::mcp::types::ToolContent]) -> Value {
    let mut results = Vec::new();

    for item in content {
        let value = match item {
            adapters::mcp::types::ToolContent::Text { text } => serde_json::json!({
                "type": "text",
                "text": text
            }),
            adapters::mcp::types::ToolContent::Image { data, mimeType } => serde_json::json!({
                "type": "image",
                "data": data,
                "mimeType": mimeType
            }),
            adapters::mcp::types::ToolContent::Resource {
                uri,
                mimeType,
                text,
                blob,
            } => {
                let mut obj = serde_json::json!({
                    "type": "resource",
                    "uri": uri
                });
                if let Some(mt) = mimeType {
                    obj["mimeType"] = serde_json::json!(mt);
                }
                if let Some(t) = text {
                    obj["text"] = serde_json::json!(t);
                }
                if let Some(b) = blob {
                    obj["blob"] = serde_json::json!(b);
                }
                obj
            }
        };
        results.push(value);
    }

    serde_json::json!({ "content": results })
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

async fn resolve_mcp_http_endpoint(url: &str, auth_profile: Option<Profile>) -> Result<String> {
    for candidate in http_endpoint_candidates(url) {
        match adapters::mcp::http_transport::McpHttpTransport::probe_initialize_with_reason(
            &candidate,
            auth_profile.clone(),
        )
        .await
        {
            Ok(adapters::mcp::http_transport::ProbeInitializeOutcome::Success) => {
                return Ok(candidate);
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

fn validate_mcp_tool_args(
    operation: &str,
    tools: &[adapters::mcp::types::Tool],
    args: &HashMap<String, Value>,
) -> Result<()> {
    let tool = tools
        .iter()
        .find(|tool| tool.name == operation)
        .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;

    let required = tool
        .inputSchema
        .as_ref()
        .and_then(|schema| schema.get("required"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let missing = required
        .into_iter()
        .filter(|key| !args.contains_key(key))
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return Ok(());
    }

    Err(UxcError::InvalidArguments(format!(
        "Missing required arguments for MCP tool '{}': {}",
        operation,
        missing.join(", ")
    ))
    .into())
}

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
