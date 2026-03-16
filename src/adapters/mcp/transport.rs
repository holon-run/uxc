//! MCP stdio transport for communicating with MCP server processes

use super::types::*;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex, Notify};

const DEFAULT_STDIO_REQUEST_TIMEOUT_MS: u64 = 30_000;
const MAX_BUFFERED_NOTIFICATIONS: usize = 64;
// Buffer enough stderr for startup diagnostics without keeping an unbounded history.
const MAX_BUFFERED_STDERR_LINES: usize = 32;
const MAX_BUFFERED_STDERR_LINE_BYTES: usize = 2048;
const STDERR_DRAIN_GRACE_PERIOD_MS: u64 = 50;

/// Trait for executing MCP stdio processes (abstracted for testing)
#[async_trait]
pub trait StdioProcessExecutor: Send + Sync {
    /// Spawn a new process with the given command and arguments
    async fn spawn(
        &self,
        command: &str,
        args: &[String],
        options: &StdioSpawnOptions,
    ) -> Result<SpawnedProcess>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StdioSpawnOptions {
    pub env_overrides: Vec<(String, String)>,
}

/// Result of spawning a process
pub struct SpawnedProcess {
    /// The child process handle
    pub child: tokio::process::Child,
    /// The stdin handle
    pub stdin: tokio::process::ChildStdin,
    /// The stdout handle
    pub stdout: tokio::process::ChildStdout,
    /// The stderr handle
    pub stderr: tokio::process::ChildStderr,
}

/// Default stdio process executor using tokio::process::Command
pub struct DefaultStdioProcessExecutor;

#[async_trait]
impl StdioProcessExecutor for DefaultStdioProcessExecutor {
    async fn spawn(
        &self,
        command: &str,
        args: &[String],
        options: &StdioSpawnOptions,
    ) -> Result<SpawnedProcess> {
        // Parse the command (handle quoted strings, etc.)
        let parts = parse_command(command);
        let (cmd, cmd_args) = parts.split_first().context("Empty command")?;

        // Build the full argument list.
        //
        // Note: stdio endpoints are executed without a shell, so "~" and "$HOME" won't expand.
        // We support "~/" expansion for common path-like flags to make linked endpoints ergonomic.
        let mut full_args: Vec<String> = cmd_args
            .iter()
            .cloned()
            .chain(args.iter().cloned())
            .collect();
        expand_tilde_args(&mut full_args);

        tracing::info!("Spawning MCP server: {} {:?}", cmd, full_args);
        if !options.env_overrides.is_empty() {
            let env_names = options
                .env_overrides
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            tracing::debug!("Injecting child env vars for MCP stdio: {:?}", env_names);
        }

        // Spawn the process
        let mut child = Command::new(cmd)
            .args(&full_args)
            .envs(options.env_overrides.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn MCP server process")?;

        // Get stdin and stdout handles
        let stdin = child.stdin.take().context("Failed to get stdin handle")?;
        let stdout = child.stdout.take().context("Failed to get stdout handle")?;
        let stderr = child.stderr.take().context("Failed to get stderr handle")?;

        Ok(SpawnedProcess {
            child,
            stdin,
            stdout,
            stderr,
        })
    }
}

/// Mock executor for testing (must be public for use in other test modules)
#[cfg(test)]
pub struct MockStdioExecutor {
    /// Simulated responses to send back
    pub responses: Arc<std::sync::Mutex<Vec<String>>>,
    /// Whether to fail spawning
    pub should_fail_spawn: bool,
    /// Whether to fail immediately after spawn
    pub should_fail_after_spawn: bool,
    /// Captured env overrides passed to spawn.
    pub captured_env_overrides: Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

#[cfg(test)]
impl MockStdioExecutor {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail_spawn: false,
            should_fail_after_spawn: false,
            captured_env_overrides: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn with_responses(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(responses)),
            should_fail_spawn: false,
            should_fail_after_spawn: false,
            captured_env_overrides: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn with_spawn_failure() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail_spawn: true,
            should_fail_after_spawn: false,
            captured_env_overrides: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn with_post_spawn_failure() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail_spawn: false,
            should_fail_after_spawn: true,
            captured_env_overrides: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[cfg(test)]
impl Default for MockStdioExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[async_trait]
impl StdioProcessExecutor for MockStdioExecutor {
    async fn spawn(
        &self,
        _command: &str,
        _args: &[String],
        options: &StdioSpawnOptions,
    ) -> Result<SpawnedProcess> {
        if self.should_fail_spawn {
            bail!("Mock executor: failed to spawn process");
        }

        *self.captured_env_overrides.lock().unwrap() = options.env_overrides.clone();

        // Create a mock child process
        let mut child = tokio::process::Command::new("echo")
            .arg("test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn mock process")?;

        let stdin = child.stdin.take().context("Failed to get stdin handle")?;
        let stdout = child.stdout.take().context("Failed to get stdout handle")?;
        let stderr = child.stderr.take().context("Failed to get stderr handle")?;

        Ok(SpawnedProcess {
            child,
            stdin,
            stdout,
            stderr,
        })
    }
}

/// MCP stdio transport client
pub struct McpStdioTransport {
    /// Child process handle
    child: tokio::process::Child,
    /// Request ID counter
    next_id: Arc<Mutex<i64>>,
    /// Request sender
    request_tx: Option<mpsc::UnboundedSender<OutboundMessage>>,
    /// Pending response channels keyed by request id
    response_channels: Arc<
        Mutex<std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
    >,
    /// Buffered server notifications received outside the request/response flow.
    notifications: Arc<Mutex<VecDeque<JsonRpcNotification>>>,
    /// Recent child stderr lines for surfacing startup failures.
    stderr_lines: Arc<Mutex<VecDeque<String>>>,
    /// Signals when the stderr reader has reached EOF.
    stderr_eof: Arc<Notify>,
    /// Tracks whether the stderr reader has fully drained.
    stderr_drained: Arc<AtomicBool>,
    /// Process executor (abstracted for testing)
    _executor: Arc<dyn StdioProcessExecutor>,
}

// Manual Debug implementation since we can't derive it for executor trait object
impl std::fmt::Debug for McpStdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpStdioTransport")
            .field("next_id", &self.next_id)
            .field("request_tx_open", &self.request_tx.is_some())
            .field("response_channels", &self.response_channels)
            .field("notifications", &self.notifications)
            .field("stderr_lines", &self.stderr_lines)
            .field(
                "stderr_drained",
                &self.stderr_drained.load(Ordering::Relaxed),
            )
            .finish()
    }
}

/// Message queued for the writer task
struct OutboundMessage {
    request_id: Option<RequestId>,
    message: String,
}

impl McpStdioTransport {
    /// Spawn a new MCP server process and create a transport
    #[allow(dead_code)]
    pub async fn connect(command: &str, args: &[String]) -> Result<Self> {
        Self::connect_with_options(command, args, StdioSpawnOptions::default()).await
    }

    pub async fn connect_with_options(
        command: &str,
        args: &[String],
        options: StdioSpawnOptions,
    ) -> Result<Self> {
        Self::connect_with_executor(
            command,
            args,
            options,
            Arc::new(DefaultStdioProcessExecutor),
        )
        .await
    }

    /// Create a new transport with a custom executor (for testing)
    pub async fn connect_with_executor(
        command: &str,
        args: &[String],
        options: StdioSpawnOptions,
        executor: Arc<dyn StdioProcessExecutor>,
    ) -> Result<Self> {
        let SpawnedProcess {
            child,
            stdin,
            stdout,
            stderr,
        } = executor.spawn(command, args, &options).await?;

        // Create channels for sending requests
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<OutboundMessage>();

        let next_id = Arc::new(Mutex::new(1i64));
        let response_channels = Arc::new(Mutex::new(std::collections::HashMap::<
            RequestId,
            tokio::sync::oneshot::Sender<JsonRpcResponse>,
        >::new()));
        let notifications = Arc::new(Mutex::new(VecDeque::<JsonRpcNotification>::new()));
        let stderr_lines = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let stderr_eof = Arc::new(Notify::new());
        let stderr_drained = Arc::new(AtomicBool::new(false));

        // Spawn a task to handle writing to stdin
        let mut stdin_writer = stdin;
        let response_channels_for_writer = response_channels.clone();
        tokio::spawn(async move {
            while let Some(req) = request_rx.recv().await {
                if let Err(e) = stdin_writer.write_all(req.message.as_bytes()).await {
                    tracing::error!("Failed to write to stdin: {}", e);
                    if let Some(request_id) = req.request_id {
                        let mut channels = response_channels_for_writer.lock().await;
                        if let Some(tx) = channels.remove(&request_id) {
                            let _ = tx.send(JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: request_id,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: -32603,
                                    message: format!("Write error: {}", e),
                                    data: None,
                                }),
                            });
                        }
                    }
                    break;
                }
                if let Err(e) = stdin_writer.write_all(b"\n").await {
                    tracing::error!("Failed to write newline to stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin_writer.flush().await {
                    tracing::error!("Failed to flush stdin: {}", e);
                    break;
                }
            }
        });

        // Spawn a task to read responses from stdout
        let response_channels_clone = response_channels.clone();
        let notifications_clone = notifications.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut buffer = String::new();

            while let Ok(Some(line)) = lines.next_line().await {
                buffer.push_str(&line);
                buffer.push('\n');

                // Try to parse a JSON-RPC message from the buffer
                while let Some(pos) = find_complete_json(&buffer) {
                    let json_str = buffer[..pos].to_string();
                    buffer = buffer[pos..].to_string();

                    // Parse the JSON-RPC message
                    match parse_jsonrpc_message(&json_str) {
                        Ok(ParsedJsonRpcMessage::Response(response)) => {
                            let id = response.id.clone();
                            let mut channels = response_channels_clone.lock().await;
                            if let Some(tx) = channels.remove(&id) {
                                let _ = tx.send(response);
                            }
                        }
                        Ok(ParsedJsonRpcMessage::Notification(notification)) => {
                            let mut queue = notifications_clone.lock().await;
                            if queue.len() >= MAX_BUFFERED_NOTIFICATIONS {
                                let dropped = queue.pop_front();
                                if let Some(dropped) = dropped {
                                    tracing::warn!(
                                        method = %dropped.method,
                                        max_buffered = MAX_BUFFERED_NOTIFICATIONS,
                                        "Dropping oldest buffered MCP stdio notification"
                                    );
                                }
                            }
                            queue.push_back(notification);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse JSON-RPC message: {}", e);
                        }
                    }
                }
            }
        });

        let stderr_lines_clone = stderr_lines.clone();
        let stderr_eof_clone = stderr_eof.clone();
        let stderr_drained_clone = stderr_drained.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("MCP stdio child stderr: {}", line);
                let mut buffer = stderr_lines_clone.lock().await;
                if buffer.len() >= MAX_BUFFERED_STDERR_LINES {
                    buffer.pop_front();
                }
                buffer.push_back(truncate_for_stderr_buffer(line));
            }

            stderr_drained_clone.store(true, Ordering::Release);
            stderr_eof_clone.notify_waiters();
        });

        Ok(Self {
            child,
            next_id,
            request_tx: Some(request_tx),
            response_channels,
            notifications,
            stderr_lines,
            stderr_eof,
            stderr_drained,
            _executor: executor,
        })
    }

    pub fn child_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn child_has_exited(&mut self) -> Result<bool> {
        Ok(self
            .child
            .try_wait()
            .context("Failed to check MCP stdio child status")?
            .is_some())
    }

    pub async fn recent_stderr_lines(&self, limit: usize) -> Vec<String> {
        if limit == 0 {
            return Vec::new();
        }
        let lines = self.stderr_lines.lock().await;
        lines
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn start_kill(&mut self) {
        // Best-effort: ensure cached/evicted MCP processes are terminated promptly.
        // Many MCP servers (including Node-based) will exit when their stdio is closed, but
        // explicitly killing here avoids profile dir locks and long-lived orphans.
        let _ = self.child.start_kill();
    }

    pub async fn kill_and_wait(&mut self, timeout: Duration) -> Result<()> {
        self.request_tx.take();

        match tokio::time::timeout(timeout, self.child.wait()).await {
            Ok(status) => {
                status.context("Failed waiting for MCP stdio child to exit after stdin close")?;
                Ok(())
            }
            Err(_) => {
                self.start_kill();
                tokio::time::timeout(timeout, self.child.wait())
                    .await
                    .context("Timed out waiting for MCP stdio child to exit after kill")?
                    .context("Failed waiting for MCP stdio child to exit after kill")?;
                Ok(())
            }
        }
    }

    /// Send a request and wait for the response
    pub async fn send_request(
        &mut self,
        method: &str,
        params: Option<JsonValue>,
    ) -> Result<JsonValue> {
        // Get the next ID
        let id = {
            let mut id_guard = self.next_id.lock().await;
            let id = *id_guard;
            *id_guard += 1;
            RequestId::Number(id)
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: id.clone(),
            method: method.to_string(),
            params,
        };

        let request_json = serde_json::to_string(&request)?;
        tracing::debug!("Sending request: {}", request_json);

        let request_tx = self
            .request_tx
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("Request channel closed"))?;

        // Create a response channel
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        // Register request id -> response channel before sending the request
        {
            let mut channels = self.response_channels.lock().await;
            channels.insert(id.clone(), response_tx);
        }

        // Send the request
        if request_tx
            .send(OutboundMessage {
                request_id: Some(id.clone()),
                message: request_json,
            })
            .is_err()
        {
            let mut channels = self.response_channels.lock().await;
            channels.remove(&id);
            return Err(anyhow!("Request channel closed"));
        }

        // Wait for the response with timeout so a stuck MCP server/tool call
        // does not block the caller indefinitely.
        let timeout = stdio_request_timeout();
        let mut response_rx = response_rx;
        let response = match tokio::select! {
            biased;
            response = &mut response_rx => StdioRequestOutcome::Response(response),
            exit_status = self.child.wait() => StdioRequestOutcome::ChildExited(exit_status),
            _ = tokio::time::sleep(timeout) => StdioRequestOutcome::TimedOut,
        } {
            StdioRequestOutcome::Response(Ok(response)) => response,
            StdioRequestOutcome::Response(Err(_)) => {
                return Err(anyhow!("Response channel closed"))
            }
            StdioRequestOutcome::ChildExited(Ok(status)) => {
                match self
                    .drain_response_after_child_exit(method, &mut response_rx)
                    .await?
                {
                    Some(response) => response,
                    None => {
                        let mut channels = self.response_channels.lock().await;
                        channels.remove(&id);
                        return Err(anyhow!(
                            "{}",
                            self.child_exit_error(method, status.code()).await
                        ));
                    }
                }
            }
            StdioRequestOutcome::ChildExited(Err(err)) => {
                let mut channels = self.response_channels.lock().await;
                channels.remove(&id);
                return Err(anyhow!(
                    "Failed waiting for MCP stdio child process while handling {}: {}",
                    method,
                    err
                ));
            }
            StdioRequestOutcome::TimedOut => {
                let mut channels = self.response_channels.lock().await;
                channels.remove(&id);
                return Err(anyhow!(
                    "MCP stdio request timed out after {}ms: {}",
                    timeout.as_millis(),
                    method
                ));
            }
        };

        if let Some(error) = response.error {
            bail!("JSON-RPC error: {} - {}", error.code, error.message);
        }

        response.result.context("No result in response")
    }

    /// Send a notification (no response expected)
    pub async fn send_notification(
        &mut self,
        method: &str,
        params: Option<JsonValue>,
    ) -> Result<()> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };

        let notification_json = serde_json::to_string(&notification)?;
        tracing::debug!("Sending notification: {}", notification_json);

        self.request_tx
            .as_ref()
            .ok_or_else(|| anyhow!("Request channel closed"))?
            .send(OutboundMessage {
                request_id: None,
                message: notification_json,
            })
            .map_err(|_| anyhow!("Request channel closed"))?;

        Ok(())
    }

    /// Initialize the MCP session
    pub async fn initialize(&mut self, client_info: ClientInfo) -> Result<InitializeResult> {
        let params = InitializeParams {
            protocolVersion: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities::default(),
            clientInfo: client_info,
        };

        let result = self
            .send_request("initialize", Some(serde_json::to_value(params)?))
            .await?;

        let init_result: InitializeResult = serde_json::from_value(result)?;
        Ok(init_result)
    }

    /// Send initialized notification
    pub async fn initialized(&mut self) -> Result<()> {
        self.send_notification("notifications/initialized", None)
            .await
    }

    pub async fn drain_notifications(&self) -> Vec<JsonRpcNotification> {
        let mut queue = self.notifications.lock().await;
        queue.drain(..).collect()
    }

    async fn drain_response_after_child_exit(
        &self,
        method: &str,
        response_rx: &mut tokio::sync::oneshot::Receiver<JsonRpcResponse>,
    ) -> Result<Option<JsonRpcResponse>> {
        tokio::task::yield_now().await;

        if let Ok(Ok(response)) = tokio::time::timeout(Duration::from_millis(1), response_rx).await
        {
            tracing::debug!(
                "Received MCP stdio response for {} just after child exit; treating request as successful",
                method
            );
            return Ok(Some(response));
        }

        self.wait_for_stderr_drain().await;
        Ok(None)
    }

    async fn child_exit_error(&self, method: &str, exit_code: Option<i32>) -> String {
        let stderr = {
            let lines = self.stderr_lines.lock().await;
            lines
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        };

        let exit_detail = match exit_code {
            Some(code) => format!("exit code {}", code),
            None => "signal".to_string(),
        };

        if stderr.is_empty() {
            format!(
                "MCP stdio child exited before response to {} ({})",
                method, exit_detail
            )
        } else {
            format!(
                "MCP stdio child exited before response to {} ({}). stderr: {}",
                method, exit_detail, stderr
            )
        }
    }

    async fn wait_for_stderr_drain(&self) {
        if self.stderr_drained.load(Ordering::Acquire) {
            return;
        }

        let _ = tokio::time::timeout(
            Duration::from_millis(STDERR_DRAIN_GRACE_PERIOD_MS),
            self.stderr_eof.notified(),
        )
        .await;
    }
}

impl Drop for McpStdioTransport {
    fn drop(&mut self) {
        self.start_kill();
    }
}

fn stdio_request_timeout() -> Duration {
    let ms = std::env::var("UXC_MCP_STDIO_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_STDIO_REQUEST_TIMEOUT_MS);
    Duration::from_millis(ms)
}

enum StdioRequestOutcome {
    Response(std::result::Result<JsonRpcResponse, tokio::sync::oneshot::error::RecvError>),
    ChildExited(std::io::Result<std::process::ExitStatus>),
    TimedOut,
}

fn expand_tilde_args(args: &mut [String]) {
    let Some(home) = resolve_home_dir() else {
        return;
    };
    let home_str = home.to_string_lossy().to_string();

    for arg in args {
        if let Some(expanded) = expand_tilde_token(arg, &home_str) {
            *arg = expanded;
        }
    }
}

fn resolve_home_dir() -> Option<PathBuf> {
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

fn expand_tilde_token(token: &str, home: &str) -> Option<String> {
    if token == "~" {
        return Some(home.to_string());
    }
    if token.starts_with("~/") {
        return Some(format!("{}/{}", home, token.trim_start_matches("~/")));
    }

    // Support common "--flag=~/path" shape.
    if let Some((k, v)) = token.split_once('=') {
        if v == "~" {
            return Some(format!("{}={}", k, home));
        }
        if v.starts_with("~/") {
            return Some(format!("{}={}/{}", k, home, v.trim_start_matches("~/")));
        }
    }

    // Windows-ish "~\\path"
    if token.starts_with("~\\") {
        let mut pb = PathBuf::from_str(home).ok()?;
        pb.push(token.trim_start_matches("~\\"));
        return Some(pb.to_string_lossy().to_string());
    }

    None
}

fn truncate_for_stderr_buffer(mut line: String) -> String {
    if line.len() <= MAX_BUFFERED_STDERR_LINE_BYTES {
        return line;
    }

    while line.len() > MAX_BUFFERED_STDERR_LINE_BYTES && !line.is_empty() {
        line.pop();
    }
    line.push_str("...");
    line
}

/// Parse a command string into parts (handles quoted strings)
///
/// Supports both single quotes (') and double quotes (") as delimiters.
/// Backslash escaping works within double quotes, but single quotes
/// are treated literally (shell-like behavior).
///
/// Note: This is a simple parser, not a full shell parser. It does not
/// support all shell features like variable expansion, command substitution,
/// or complex quoting rules. For simple stdio MCP commands, this is sufficient.
pub fn parse_command(cmd: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum QuoteState {
        None,
        Single,
        Double,
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote_state = QuoteState::None;
    let mut escape_next = false;

    for ch in cmd.chars() {
        if escape_next {
            // Only process escapes in double quotes or outside quotes
            if quote_state != QuoteState::Single {
                current.push(ch);
            } else {
                // Inside single quotes, backslash is literal
                current.push('\\');
                current.push(ch);
            }
            escape_next = false;
        } else if ch == '\\' && quote_state != QuoteState::Single {
            // Backslash only triggers escape mode outside single quotes
            escape_next = true;
        } else if ch == '"' && quote_state != QuoteState::Single {
            // Toggle double quote state (ignored inside single quotes)
            // When closing a quote, always push the token even if empty
            if quote_state == QuoteState::Double {
                // Closing double quote - end the token
                parts.push(current.clone());
                current.clear();
                quote_state = QuoteState::None;
            } else {
                // Opening double quote
                quote_state = QuoteState::Double;
            }
        } else if ch == '\'' && quote_state != QuoteState::Double {
            // Toggle single quote state (ignored inside double quotes)
            // When closing a quote, always push the token even if empty
            if quote_state == QuoteState::Single {
                // Closing single quote - end the token
                parts.push(current.clone());
                current.clear();
                quote_state = QuoteState::None;
            } else {
                // Opening single quote
                quote_state = QuoteState::Single;
            }
        } else if ch.is_whitespace() && quote_state == QuoteState::None {
            // When not in quotes, whitespace ends the current token
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }

    // Handle any remaining content
    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Find a complete JSON object in the string.
/// Returns the byte length of the JSON object if found.
fn find_complete_json(s: &str) -> Option<usize> {
    let mut brace_count = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in s.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }

        if ch == '"' {
            in_string = !in_string;
            continue;
        }

        if !in_string {
            if ch == '{' {
                brace_count += 1;
            } else if ch == '}' {
                brace_count -= 1;
                if brace_count == 0 {
                    return Some(i + ch.len_utf8());
                }
            }
        }
    }

    None
}

/// Parse a JSON-RPC message
#[derive(Debug)]
enum ParsedJsonRpcMessage {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

fn parse_jsonrpc_message(s: &str) -> Result<ParsedJsonRpcMessage> {
    let value: JsonValue = serde_json::from_str(s)?;

    // Check if it's a response (has "id" field)
    if value.get("id").is_some() {
        let response: JsonRpcResponse = serde_json::from_value(value)?;
        Ok(ParsedJsonRpcMessage::Response(response))
    } else {
        let notification: JsonRpcNotification = serde_json::from_value(value)?;
        Ok(ParsedJsonRpcMessage::Notification(notification))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn parse_command_handles_simple_command() {
        let parts = parse_command("node server.js");
        assert_eq!(parts, vec!["node", "server.js"]);
    }

    #[tokio::test]
    async fn parse_command_handles_command_with_args() {
        let parts = parse_command("npx @modelcontextprotocol/server-everything");
        assert_eq!(
            parts,
            vec!["npx", "@modelcontextprotocol/server-everything"]
        );
    }

    #[tokio::test]
    async fn parse_command_handles_quoted_strings() {
        let parts = parse_command("node \"my server.js\"");
        assert_eq!(parts, vec!["node", "my server.js"]);
    }

    #[tokio::test]
    async fn parse_command_handles_escaped_quotes() {
        let parts = parse_command("node \"my \\\"server\\\".js\"");
        assert_eq!(parts, vec!["node", "my \"server\".js"]);
    }

    #[tokio::test]
    async fn parse_command_handles_single_quotes() {
        let parts = parse_command("node 'my server.js'");
        assert_eq!(parts, vec!["node", "my server.js"]);
    }

    #[tokio::test]
    async fn parse_command_preserves_single_quotes_literal_content() {
        let parts = parse_command("sh -lc 'echo \"hello\"'");
        assert_eq!(parts, vec!["sh", "-lc", "echo \"hello\""]);
    }

    #[tokio::test]
    async fn parse_command_handles_backslash_in_single_quotes() {
        let parts = parse_command("sh -lc 'echo \\\"hello\\\"'");
        assert_eq!(parts, vec!["sh", "-lc", "echo \\\"hello\\\""]);
    }

    #[tokio::test]
    async fn parse_command_handles_mixed_quotes() {
        let parts = parse_command("echo \"hello\" 'world'");
        assert_eq!(parts, vec!["echo", "hello", "world"]);
    }

    #[tokio::test]
    async fn parse_command_handles_zsh_command_form() {
        let parts = parse_command("/bin/zsh -lc 'npx -y mcp-remote https://mcp.deepwiki.com/mcp'");
        assert_eq!(
            parts,
            vec![
                "/bin/zsh",
                "-lc",
                "npx -y mcp-remote https://mcp.deepwiki.com/mcp"
            ]
        );
    }

    #[tokio::test]
    async fn parse_command_handles_embedded_single_quotes_in_double_quotes() {
        let parts = parse_command("echo \"it's a test\"");
        assert_eq!(parts, vec!["echo", "it's a test"]);
    }

    #[tokio::test]
    async fn parse_command_handles_embedded_double_quotes_in_single_quotes() {
        let parts = parse_command("echo 'say \"hello\"'");
        assert_eq!(parts, vec!["echo", "say \"hello\""]);
    }

    #[tokio::test]
    async fn parse_command_handles_empty_single_quoted_string() {
        let parts = parse_command("echo ''");
        // Empty single quotes create an empty token
        assert_eq!(parts, vec!["echo", ""]);
    }

    #[tokio::test]
    async fn parse_command_handles_empty_double_quoted_string() {
        let parts = parse_command("echo \"\"");
        // Empty double quotes create an empty token
        assert_eq!(parts, vec!["echo", ""]);
    }

    #[tokio::test]
    async fn parse_command_handles_unclosed_single_quote() {
        // Unclosed single quote - the content is still added as a token
        let parts = parse_command("echo 'hello");
        assert_eq!(parts, vec!["echo", "hello"]);
    }

    #[tokio::test]
    async fn parse_command_handles_unclosed_double_quote() {
        // Unclosed double quote - the content is still added as a token
        let parts = parse_command("echo \"hello");
        assert_eq!(parts, vec!["echo", "hello"]);
    }

    #[tokio::test]
    async fn parse_command_handles_multiple_spaces() {
        let parts = parse_command("node   server.js");
        assert_eq!(parts, vec!["node", "server.js"]);
    }

    #[tokio::test]
    async fn parse_command_handles_empty_command() {
        let parts = parse_command("");
        assert_eq!(parts, Vec::<String>::new());
    }

    #[tokio::test]
    async fn find_complete_json_finds_simple_object() {
        let json = r#"{"key": "value"}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }

    #[tokio::test]
    async fn find_complete_json_finds_nested_object() {
        let json = r#"{"key": {"nested": "value"}}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }

    #[tokio::test]
    async fn find_complete_json_finds_object_with_array() {
        let json = r#"{"key": [1, 2, 3]}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }

    #[tokio::test]
    async fn find_complete_json_handles_strings_with_braces() {
        let json = r#"{"key": "{value}"}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }

    #[tokio::test]
    async fn find_complete_json_handles_escaped_quotes_in_strings() {
        let json = r#"{"key": "value\"with\"quotes"}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }

    #[tokio::test]
    async fn find_complete_json_returns_none_for_incomplete_json() {
        let json = r#"{"key": "value""#;
        assert_eq!(find_complete_json(json), None);
    }

    #[tokio::test]
    async fn find_complete_json_returns_none_for_empty_string() {
        assert_eq!(find_complete_json(""), None);
    }

    #[tokio::test]
    async fn parse_jsonrpc_message_parses_valid_response() {
        let json = r#"{"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}"#;
        let result = parse_jsonrpc_message(json);
        assert!(result.is_ok());
        match result.unwrap() {
            ParsedJsonRpcMessage::Response(resp) => {
                assert_eq!(resp.id, RequestId::Number(1));
                assert!(resp.result.is_some());
            }
            ParsedJsonRpcMessage::Notification(_) => panic!("expected response"),
        }
    }

    #[tokio::test]
    async fn parse_jsonrpc_message_parses_error_response() {
        let json = r#"{"jsonrpc": "2.0", "id": 1, "error": {"code": -32600, "message": "Invalid Request"}}"#;
        let result = parse_jsonrpc_message(json);
        assert!(result.is_ok());
        match result.unwrap() {
            ParsedJsonRpcMessage::Response(resp) => {
                assert_eq!(resp.id, RequestId::Number(1));
                assert!(resp.error.is_some());
            }
            ParsedJsonRpcMessage::Notification(_) => panic!("expected response"),
        }
    }

    #[tokio::test]
    async fn parse_jsonrpc_message_parses_notification() {
        let json = r#"{"jsonrpc": "2.0", "method": "notification", "params": {}}"#;
        let result = parse_jsonrpc_message(json);
        assert!(result.is_ok());
        match result.unwrap() {
            ParsedJsonRpcMessage::Notification(notification) => {
                assert_eq!(notification.method, "notification");
                assert_eq!(notification.params.unwrap(), serde_json::json!({}));
            }
            ParsedJsonRpcMessage::Response(_) => panic!("expected notification"),
        }
    }

    #[tokio::test]
    async fn parse_jsonrpc_message_returns_error_for_invalid_json() {
        let json = r#"{"jsonrpc": "2.0", "id": 1"#;
        let result = parse_jsonrpc_message(json);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_request_routes_response_by_id() {
        let script =
            "read line; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}'; sleep 1";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let response = transport.send_request("ping", None).await.unwrap();
        assert_eq!(response["ok"], true);
    }

    #[tokio::test]
    async fn send_request_handles_large_utf8_json_payload() {
        let text = "百度一下，你就知道 历史记录";
        let large_text = text.repeat(2048);
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": large_text
                    }
                ]
            }
        });
        let script = format!("read line; cat <<'EOF'\n{}\nEOF", response);
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let result = transport.send_request("tools/call", None).await.unwrap();
        let got = result["content"][0]["text"]
            .as_str()
            .expect("text content should be string");
        assert!(got.starts_with("百度一下"));
        assert!(got.contains("历史记录"));
        assert!(got.len() > 10_000);
    }

    #[tokio::test]
    async fn send_notification_does_not_wait_for_response() {
        let script =
            "while read line; do echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'; done";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let result = transport.send_notification("test/notification", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_request_with_params_includes_params_in_request() {
        let script = "read line; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"received\":true}}'; sleep 1";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let params = serde_json::json!({"key": "value"});
        let response = transport.send_request("test", Some(params)).await.unwrap();
        assert_eq!(response["received"], true);
    }

    #[tokio::test]
    async fn send_request_returns_error_for_jsonrpc_error() {
        let script = "read line; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}'; sleep 1";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let result = transport.send_request("unknown_method", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Method not found"));
    }

    #[tokio::test]
    async fn initialize_sends_correct_parameters() {
        let script = "read line; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"serverInfo\":{\"name\":\"test\",\"version\":\"1.0\"}}}'";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let client_info = ClientInfo {
            name: "uxc".to_string(),
            version: "1.0.0".to_string(),
        };

        let result = transport.initialize(client_info).await.unwrap();
        assert_eq!(result.protocolVersion, "2024-11-05");
        assert_eq!(result.serverInfo.unwrap().name, "test");
    }

    #[tokio::test]
    async fn initialized_sends_notification() {
        let script =
            "while read line; do echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'; done";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let result = transport.initialized().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connect_with_invalid_command_fails() {
        let result = McpStdioTransport::connect("nonexistent_command_xyz", &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to spawn"));
    }

    #[tokio::test]
    async fn connect_with_mock_executor_succeeds() {
        let mock = Arc::new(MockStdioExecutor::new());
        let result = McpStdioTransport::connect_with_executor(
            "test",
            &[],
            StdioSpawnOptions::default(),
            mock,
        )
        .await;
        // The mock will spawn a real echo process, so this should succeed
        // but may fail on initialization - that's ok for this test
        // We're just testing that the executor is being used
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connect_with_failing_mock_executor_fails() {
        let mock = Arc::new(MockStdioExecutor::with_spawn_failure());
        let result = McpStdioTransport::connect_with_executor(
            "test",
            &[],
            StdioSpawnOptions::default(),
            mock,
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed to spawn"));
    }

    #[tokio::test]
    async fn connect_with_mock_executor_passes_env_overrides() {
        let mock = Arc::new(MockStdioExecutor::new());
        let options = StdioSpawnOptions {
            env_overrides: vec![("TOKEN".to_string(), "secret".to_string())],
        };
        let _ = McpStdioTransport::connect_with_executor("test", &[], options, mock.clone()).await;
        let captured = mock.captured_env_overrides.lock().unwrap().clone();
        assert_eq!(captured, vec![("TOKEN".to_string(), "secret".to_string())]);
    }

    #[tokio::test]
    async fn request_id_increments_with_each_request() {
        let script =
            "while read line; do echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'; done";
        let transport = McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
            .await
            .unwrap();

        // Check that ID counter starts at 1 and increments
        let id1 = {
            let id_guard = transport.next_id.lock().await;
            *id_guard
        };
        assert_eq!(id1, 1);

        {
            let mut id_guard = transport.next_id.lock().await;
            *id_guard += 1;
        }

        let id2 = {
            let id_guard = transport.next_id.lock().await;
            *id_guard
        };
        assert_eq!(id2, 2);
    }

    #[tokio::test]
    async fn send_request_timeout_returns_error() {
        // This test uses a script that never responds
        let script = "read line; sleep 10";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        // Send a request - the response channel will close without a response
        // This should return an error
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            transport.send_request("timeout_test", None),
        )
        .await;

        // Either timeout or error is acceptable
        assert!(result.is_err() || result.unwrap().is_err());
    }

    #[tokio::test]
    async fn send_request_does_not_leave_pending_channel_when_request_tx_is_closed() {
        let script = "sleep 10";
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();
        transport.request_tx.take();

        let err = transport.send_request("echo", None).await.unwrap_err();
        assert!(err.to_string().contains("Request channel closed"));

        let channels = transport.response_channels.lock().await;
        assert!(channels.is_empty(), "response_channels should be empty");
    }

    #[tokio::test]
    async fn initialize_fails_fast_when_child_exits_before_responding() {
        let script = r#"
            read line
            echo "TARGET_UNREACHABLE: failed to connect" >&2
            exit 7
        "#;
        let timeout_ms = 10_000u64;
        std::env::set_var("UXC_MCP_STDIO_TIMEOUT_MS", timeout_ms.to_string());
        let mut transport =
            McpStdioTransport::connect("sh", &["-c".to_string(), script.to_string()])
                .await
                .unwrap();

        let client_info = ClientInfo {
            name: "uxc".to_string(),
            version: "1.0.0".to_string(),
        };

        let start = tokio::time::Instant::now();
        let err = transport
            .initialize(client_info)
            .await
            .unwrap_err()
            .to_string();
        std::env::remove_var("UXC_MCP_STDIO_TIMEOUT_MS");

        assert!(
            start.elapsed() < Duration::from_millis(timeout_ms / 4),
            "initialize should fail well before timeout {}, got {:?}",
            timeout_ms,
            start.elapsed()
        );
        assert!(err.contains("child exited before response to initialize"));
        assert!(err.contains("exit code 7"));
        assert!(err.contains("TARGET_UNREACHABLE"));
        assert!(
            !err.contains("timed out"),
            "unexpected timeout error: {}",
            err
        );
    }

    #[tokio::test]
    async fn parse_command_handles_windows_paths() {
        // Note: our parser treats backslash as escape, so we need to escape them
        let parts = parse_command(r#"C:\\Users\\test\\server.exe"#);
        assert_eq!(parts, vec![r"C:\Users\test\server.exe"]);
    }

    #[tokio::test]
    async fn parse_command_handles_mixed_paths() {
        let parts = parse_command("./server --arg1 \"value with spaces\" --arg2");
        assert_eq!(
            parts,
            vec!["./server", "--arg1", "value with spaces", "--arg2"]
        );
    }

    #[tokio::test]
    async fn find_complete_json_handles_multiple_json_objects() {
        let json = r#"{"first": 1}{"second": 2}"#;
        assert_eq!(find_complete_json(json), Some(12)); // Length of first JSON: {"first": 1}
    }

    #[tokio::test]
    async fn find_complete_json_handles_json_with_newlines() {
        let json = r#"{"key":
"value"}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }

    #[tokio::test]
    async fn find_complete_json_returns_byte_boundary_for_utf8_content() {
        let json =
            r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"百度 历史"}]}}"#;
        let pos = find_complete_json(json).expect("should find complete json");
        assert_eq!(pos, json.len());
        assert!(serde_json::from_str::<JsonValue>(&json[..pos]).is_ok());
    }

    #[tokio::test]
    async fn default_executor_implements_trait() {
        let executor = DefaultStdioProcessExecutor;
        // Test that we can call spawn (it will fail for invalid command, but that's ok)
        let result = executor
            .spawn(
                "nonexistent_test_command_xyz",
                &[],
                &StdioSpawnOptions::default(),
            )
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_command_sync() {
        let parts = parse_command("echo test");
        assert_eq!(parts, vec!["echo", "test"]);
    }

    #[test]
    fn test_find_complete_json_sync() {
        let json = r#"{"test": true}"#;
        assert_eq!(find_complete_json(json), Some(json.len()));
    }
}
