//! Daemon logging module for troubleshooting observability.
//!
//! This module provides file-based JSON Lines logging for daemon operations.
//! It is designed for troubleshooting and diagnostics, not compliance audit logging.
//!
//! # Features
//! - JSON Lines format for machine parsing
//! - Automatic secret redaction for sensitive values
//! - Simple log rotation to bound file size
//! - Thread-safe async operations

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_MAX_LOG_BYTES: u64 = 10 * 1024 * 1024; // 10 MiB
const DEFAULT_LOG_BACKUPS: usize = 3;

/// Daemon log event types for troubleshooting
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonEventType {
    // Daemon lifecycle events
    DaemonStart,
    DaemonStop,
    DaemonStatus,
    DaemonAutostart,

    // Runtime invoke events
    RuntimeInvokeStart,
    RuntimeInvokeSuccess,
    RuntimeInvokeFailure,

    // Protocol detection events
    ProtocolDetectionSuccess,
    ProtocolDetectionFailure,

    // Cache events
    CacheHit,
    CacheStale,
    CacheFallback,

    // Session management
    DaemonSessionReused,
}

/// Daemon log entry with redaction support
#[derive(Debug, Clone, Serialize)]
pub struct DaemonLogEntry {
    /// Event type identifier
    #[serde(rename = "type")]
    pub event_type: DaemonEventType,

    /// Unix timestamp in seconds
    pub timestamp: u64,

    /// Optional request ID for correlation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,

    /// Endpoint being operated on (redacted if contains secrets)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Protocol type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,

    /// Operation ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,

    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Error message (redacted)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Additional metadata (redacted)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl DaemonLogEntry {
    /// Create a new log entry with current timestamp
    pub fn new(event_type: DaemonEventType) -> Self {
        Self {
            event_type,
            timestamp: now_unix_secs(),
            request_id: None,
            endpoint: None,
            protocol: None,
            operation_id: None,
            duration_ms: None,
            error: None,
            meta: None,
        }
    }

    /// Add request ID
    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    /// Add endpoint (with automatic redaction)
    pub fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint = Some(redact_endpoint(&endpoint));
        self
    }

    /// Add protocol
    pub fn with_protocol(mut self, protocol: String) -> Self {
        self.protocol = Some(protocol);
        self
    }

    /// Add operation ID
    pub fn with_operation_id(mut self, operation_id: String) -> Self {
        self.operation_id = Some(operation_id);
        self
    }

    /// Add duration in milliseconds
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Add error message (with automatic redaction)
    pub fn with_error(mut self, error: String) -> Self {
        self.error = Some(redact_sensitive(&error));
        self
    }

    /// Add metadata (with automatic redaction)
    #[allow(dead_code)]
    pub fn with_meta(mut self, meta: serde_json::Value) -> Self {
        self.meta = Some(redact_value(meta));
        self
    }
}

/// Daemon file logger with rotation support
#[derive(Clone)]
pub struct DaemonLogger {
    log_file: PathBuf,
    max_bytes: u64,
    backups: usize,
    #[allow(dead_code)]
    inner: Arc<Mutex<LoggerInner>>,
}

struct LoggerInner {
    file: Option<File>,
}

impl DaemonLogger {
    /// Create a new daemon logger
    pub fn new(daemon_dir: &Path) -> Result<Self> {
        let log_file = daemon_dir.join("daemon.log");
        let max_bytes = DEFAULT_MAX_LOG_BYTES;
        let backups = DEFAULT_LOG_BACKUPS;

        // Ensure log directory exists
        if let Some(parent) = log_file.parent() {
            std::fs::create_dir_all(parent).context("Failed to create log directory")?;
        }

        // Perform initial rotation check if log exists
        if should_rotate(&log_file, max_bytes)? {
            rotate_log_if_needed(&log_file, backups)?;
        }

        let file = open_log_file(&log_file)?;

        Ok(Self {
            log_file,
            max_bytes,
            backups,
            inner: Arc::new(Mutex::new(LoggerInner { file: Some(file) })),
        })
    }

    /// Write a log entry
    pub async fn log(&self, entry: &DaemonLogEntry) -> Result<()> {
        let line = serde_json::to_string(entry).context("Failed to serialize log entry")?;

        let mut inner = self.inner.lock().await;
        if inner.file.is_none() {
            inner.file = Some(open_log_file(&self.log_file)?);
        }

        if let Some(file) = &mut inner.file {
            writeln!(file, "{}", line).context("Failed to write log entry")?;
            file.flush().context("Failed to flush log entry")?;
        }

        // Keep write + rotate in one critical section to avoid races and stale fds.
        // Close the active fd before rotate so Windows rename can succeed.
        if should_rotate(&self.log_file, self.max_bytes)? {
            inner.file.take();
            rotate_log_if_needed(&self.log_file, self.backups)?;
            inner.file = Some(open_log_file(&self.log_file)?);
        }

        Ok(())
    }

    /// Get the log file path
    pub fn log_file_path(&self) -> &Path {
        &self.log_file
    }

    /// Check if logging is enabled
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        true // Logging is always enabled when logger is created
    }
}

/// Rotate log file
fn rotate_log_if_needed(log_file: &Path, backups: usize) -> Result<()> {
    // Rotate existing backups
    for i in (1..backups).rev() {
        let old_backup = log_file.with_extension(format!("log.{}", i));
        let new_backup = log_file.with_extension(format!("log.{}", i + 1));
        if old_backup.exists() {
            rename_replace(&old_backup, &new_backup).context("Failed to rotate backup log file")?;
        }
    }

    // Move current log to .1
    if log_file.exists() {
        let backup1 = log_file.with_extension("log.1");
        rename_replace(log_file, &backup1).context("Failed to rotate current log file")?;
    }

    Ok(())
}

fn open_log_file(log_file: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .context("Failed to open log file")
}

fn should_rotate(log_file: &Path, max_bytes: u64) -> Result<bool> {
    if !log_file.exists() {
        return Ok(false);
    }
    let meta = std::fs::metadata(log_file).context("Failed to read log file metadata")?;
    Ok(meta.len() > max_bytes)
}

fn rename_replace(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        std::fs::remove_file(dst).context("Failed to remove existing rotation destination")?;
    }
    std::fs::rename(src, dst).context("Failed to rename rotation file")?;
    Ok(())
}

/// Redact sensitive information from endpoint URLs
pub(crate) fn redact_endpoint(endpoint: &str) -> String {
    // Use regex for proper pattern matching with capture groups
    let api_key_re = regex::Regex::new(r"([?&]api_key=)[^&]*").unwrap();
    let token_re = regex::Regex::new(r"([?&](?:access_)?token=)[^&]*").unwrap();
    let bearer_re = regex::Regex::new(r"(/bearer/)[^/?&]*").unwrap();
    let basic_auth_re = regex::Regex::new(r"(://)[^:@]*:([^@]*)@").unwrap();

    let redacted = endpoint.to_string();
    let redacted = api_key_re.replace_all(&redacted, "${1}***").to_string();
    let redacted = token_re.replace_all(&redacted, "${1}***").to_string();
    let redacted = bearer_re.replace_all(&redacted, "${1}***").to_string();
    let redacted = basic_auth_re
        .replace_all(&redacted, "${1}***:***@")
        .to_string();

    redacted
}

/// Redact sensitive information from general strings
pub(crate) fn redact_sensitive(text: &str) -> String {
    // Check if text contains patterns that look like secrets
    let mut redacted = text.to_string();

    // Common secret field patterns in error messages (case-insensitive)
    // Use capture groups to preserve the field name
    // Order matters - more specific patterns first
    let patterns = [
        (r"(?i)(bearer\s+)[a-zA-Z0-9\-._~+/=]*", "${1}***"),
        (r"(?i)(api[_-]?key\s*[=:]\s*)[^\s,']+", "${1}***"),
        (r"(?i)(token\s*[=:]\s*)[^\s,']+", "${1}***"),
        (r"(?i)(secret\s*[=:]\s*)[^\s,']+", "${1}***"),
        (r"(?i)(password\s*[=:]\s*)[^\s,']+", "${1}***"),
        (r"(?i)(authorization\s*[=:]\s*)[^\s,']+", "${1}***"),
    ];

    for (pattern, replacement) in patterns {
        let re = regex::Regex::new(pattern).unwrap();
        redacted = re.replace_all(&redacted, replacement).to_string();
    }

    // Redact header-like key/value pairs using semantic key checks to avoid
    // accidental matches like "monkey: banana".
    let header_re = regex::Regex::new(r"(?i)\b([a-z0-9_-]+\s*:\s*)([^\s,']+)").unwrap();
    redacted = header_re
        .replace_all(&redacted, |caps: &regex::Captures| {
            let key_with_colon = caps.get(1).map_or("", |m| m.as_str());
            let key = key_with_colon
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if is_sensitive_header_name(&key) {
                format!("{}***", key_with_colon)
            } else {
                caps.get(0).map_or("", |m| m.as_str()).to_string()
            }
        })
        .to_string();

    // Also redact things that look like JWTs (long base64-like strings with dots)
    let jwt_re = regex::Regex::new(r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap();
    if redacted.len() > 50 && jwt_re.is_match(&redacted) {
        redacted = jwt_re.replace_all(&redacted, "***").to_string();
    }

    redacted
}

fn is_sensitive_header_name(name: &str) -> bool {
    let keywords = ["auth", "token", "secret", "key", "passphrase"];
    name.split(['-', '_'])
        .any(|segment| keywords.iter().any(|kw| segment.eq_ignore_ascii_case(kw)))
}

/// Recursively redact sensitive values in JSON
#[allow(dead_code)]
fn redact_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            let sensitive_keys = [
                "api_key",
                "apikey",
                "api-key",
                "token",
                "access_token",
                "accesstoken",
                "access-token",
                "secret",
                "secret_key",
                "secretkey",
                "password",
                "passwd",
                "authorization",
                "auth",
                "bearer",
                "credential",
                "credentials",
                "private_key",
                "privatekey",
            ];

            for (key, val) in map.iter_mut() {
                let key_lower = key.to_lowercase();
                let is_sensitive = sensitive_keys.iter().any(|sk| key_lower.contains(sk));

                if is_sensitive {
                    *val = json!("***");
                } else {
                    *val = redact_value(val.clone());
                }
            }

            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(redact_value).collect())
        }
        serde_json::Value::String(s) => serde_json::Value::String(redact_sensitive(&s)),
        _ => value,
    }
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_endpoint() {
        assert_eq!(
            redact_endpoint("https://api.example.com?api_key=secret123"),
            "https://api.example.com?api_key=***"
        );

        assert_eq!(
            redact_endpoint("https://user:pass@example.com/path"),
            "https://***:***@example.com/path"
        );

        assert_eq!(
            redact_endpoint("https://api.example.com?token=abc&other=def"),
            "https://api.example.com?token=***&other=def"
        );
    }

    #[test]
    fn test_redact_sensitive() {
        assert_eq!(
            redact_sensitive("Failed with api_key=secret123"),
            "Failed with api_key=***"
        );

        assert_eq!(
            redact_sensitive("monkey: banana"),
            "monkey: banana",
            "non-sensitive header-like labels should not be redacted"
        );
        assert_eq!(
            redact_sensitive("x-api-key: secret123"),
            "x-api-key: ***",
            "sensitive header names should be redacted"
        );

        // Bearer tokens get redacted - both "Bearer" keyword and JWT pattern match
        assert!(
            redact_sensitive("Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9")
                .contains("***")
        );
        assert!(
            !redact_sensitive("Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9")
                .contains("eyJ")
        );
    }

    #[test]
    fn test_redact_value() {
        let value = json!({
            "endpoint": "https://api.example.com",
            "api_key": "secret123",
            "nested": {
                "token": "abc123"
            }
        });

        let redacted = redact_value(value);
        assert_eq!(redacted["endpoint"], "https://api.example.com");
        assert_eq!(redacted["api_key"], "***");
        assert_eq!(redacted["nested"]["token"], "***");
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = DaemonLogEntry::new(DaemonEventType::RuntimeInvokeStart)
            .with_request_id("req-123".to_string())
            .with_endpoint("https://api.example.com?api_key=secret".to_string())
            .with_protocol("openapi".to_string());

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "runtime_invoke_start");
        assert_eq!(parsed["request_id"], "req-123");
        assert!(parsed["endpoint"].as_str().unwrap().contains("***"));
    }
}
