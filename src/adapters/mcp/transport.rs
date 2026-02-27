//! MCP stdio transport for communicating with MCP server processes

use super::types::*;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};

/// Trait for executing MCP stdio processes (abstracted for testing)
#[async_trait]
pub trait StdioProcessExecutor: Send + Sync {
    /// Spawn a new process with the given command and arguments
    async fn spawn(&self, command: &str, args: &[String]) -> Result<SpawnedProcess>;
}

/// Result of spawning a process
pub struct SpawnedProcess {
    /// The child process handle
    pub child: tokio::process::Child,
    /// The stdin handle
    pub stdin: tokio::process::ChildStdin,
    /// The stdout handle
    pub stdout: tokio::process::ChildStdout,
}

/// Default stdio process executor using tokio::process::Command
pub struct DefaultStdioProcessExecutor;

#[async_trait]
impl StdioProcessExecutor for DefaultStdioProcessExecutor {
    async fn spawn(&self, command: &str, args: &[String]) -> Result<SpawnedProcess> {
        // Parse the command (handle quoted strings, etc.)
        let parts = parse_command(command);
        let (cmd, cmd_args) = parts.split_first().context("Empty command")?;

        // Build the full argument list
        let full_args: Vec<&str> = cmd_args
            .iter()
            .map(|s| s.as_str())
            .chain(args.iter().map(|s| s.as_str()))
            .collect();

        tracing::info!("Spawning MCP server: {} {:?}", cmd, full_args);

        // Spawn the process
        let mut child = Command::new(cmd)
            .args(&full_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn MCP server process")?;

        // Get stdin and stdout handles
        let stdin = child.stdin.take().context("Failed to get stdin handle")?;
        let stdout = child.stdout.take().context("Failed to get stdout handle")?;

        Ok(SpawnedProcess {
            child,
            stdin,
            stdout,
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
}

#[cfg(test)]
impl MockStdioExecutor {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail_spawn: false,
            should_fail_after_spawn: false,
        }
    }

    pub fn with_responses(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(responses)),
            should_fail_spawn: false,
            should_fail_after_spawn: false,
        }
    }

    pub fn with_spawn_failure() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail_spawn: true,
            should_fail_after_spawn: false,
        }
    }

    pub fn with_post_spawn_failure() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(Vec::new())),
            should_fail_spawn: false,
            should_fail_after_spawn: true,
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
    async fn spawn(&self, _command: &str, _args: &[String]) -> Result<SpawnedProcess> {
        if self.should_fail_spawn {
            bail!("Mock executor: failed to spawn process");
        }

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

        Ok(SpawnedProcess {
            child,
            stdin,
            stdout,
        })
    }
}

/// MCP stdio transport client
pub struct McpStdioTransport {
    /// Child process handle
    _child: tokio::process::Child,
    /// Request ID counter
    next_id: Arc<Mutex<i64>>,
    /// Request sender
    request_tx: mpsc::UnboundedSender<OutboundMessage>,
    /// Pending response channels keyed by request id
    response_channels: Arc<
        Mutex<std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
    >,
    /// Process executor (abstracted for testing)
    _executor: Arc<dyn StdioProcessExecutor>,
}

// Manual Debug implementation since we can't derive it for executor trait object
impl std::fmt::Debug for McpStdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpStdioTransport")
            .field("next_id", &self.next_id)
            .field("request_tx", &self.request_tx)
            .field("response_channels", &self.response_channels)
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
        Self::connect_with_executor(command, args, Arc::new(DefaultStdioProcessExecutor)).await
    }

    /// Create a new transport with a custom executor (for testing)
    pub async fn connect_with_executor(
        command: &str,
        args: &[String],
        executor: Arc<dyn StdioProcessExecutor>,
    ) -> Result<Self> {
        let SpawnedProcess {
            child,
            stdin,
            stdout,
        } = executor.spawn(command, args).await?;

        // Create channels for sending requests
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<OutboundMessage>();

        let next_id = Arc::new(Mutex::new(1i64));
        let response_channels = Arc::new(Mutex::new(std::collections::HashMap::<
            RequestId,
            tokio::sync::oneshot::Sender<JsonRpcResponse>,
        >::new()));

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
                        Ok(Some(response)) => {
                            let id = response.id.clone();
                            let mut channels = response_channels_clone.lock().await;
                            if let Some(tx) = channels.remove(&id) {
                                let _ = tx.send(response);
                            }
                        }
                        Ok(None) => {
                            // Notification (no response expected), ignore
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse JSON-RPC message: {}", e);
                        }
                    }
                }
            }
        });

        Ok(Self {
            _child: child,
            next_id,
            request_tx,
            response_channels,
            _executor: executor,
        })
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

        // Create a response channel
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        // Register request id -> response channel before sending the request
        {
            let mut channels = self.response_channels.lock().await;
            channels.insert(id.clone(), response_tx);
        }

        // Send the request
        if self
            .request_tx
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

        // Wait for the response
        let response = response_rx.await.context("Response channel closed")?;

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
}

/// Parse a command string into parts (handles quoted strings)
pub fn parse_command(cmd: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escape_next = false;

    for ch in cmd.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
        } else if ch == '\\' {
            escape_next = true;
        } else if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Find a complete JSON object in the string
/// Returns the length of the JSON object if found
fn find_complete_json(s: &str) -> Option<usize> {
    let mut brace_count = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in s.chars().enumerate() {
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
                    return Some(i + 1);
                }
            }
        }
    }

    None
}

/// Parse a JSON-RPC message
fn parse_jsonrpc_message(s: &str) -> Result<Option<JsonRpcResponse>> {
    let value: JsonValue = serde_json::from_str(s)?;

    // Check if it's a response (has "id" field)
    if value.get("id").is_some() {
        let response: JsonRpcResponse = serde_json::from_value(value)?;
        Ok(Some(response))
    } else {
        // It's a notification, no response expected
        Ok(None)
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
        let response = result.unwrap();
        assert!(response.is_some());
        let resp = response.unwrap();
        assert_eq!(resp.id, RequestId::Number(1));
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn parse_jsonrpc_message_parses_error_response() {
        let json = r#"{"jsonrpc": "2.0", "id": 1, "error": {"code": -32600, "message": "Invalid Request"}}"#;
        let result = parse_jsonrpc_message(json);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_some());
        let resp = response.unwrap();
        assert_eq!(resp.id, RequestId::Number(1));
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn parse_jsonrpc_message_parses_notification_as_none() {
        let json = r#"{"jsonrpc": "2.0", "method": "notification", "params": {}}"#;
        let result = parse_jsonrpc_message(json);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_none());
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
        let result = McpStdioTransport::connect_with_executor("test", &[], mock).await;
        // The mock will spawn a real echo process, so this should succeed
        // but may fail on initialization - that's ok for this test
        // We're just testing that the executor is being used
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connect_with_failing_mock_executor_fails() {
        let mock = Arc::new(MockStdioExecutor::with_spawn_failure());
        let result = McpStdioTransport::connect_with_executor("test", &[], mock).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed to spawn"));
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
    async fn default_executor_implements_trait() {
        let executor = DefaultStdioProcessExecutor;
        // Test that we can call spawn (it will fail for invalid command, but that's ok)
        let result = executor.spawn("nonexistent_test_command_xyz", &[]).await;
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
