//! UXC error types

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, UxcError>;

#[derive(Error, Debug)]
pub enum UxcError {
    #[error("Protocol detection failed: {0}")]
    ProtocolDetectionFailed(String),

    #[allow(dead_code)]
    #[error("Unsupported protocol: {0}")]
    UnsupportedProtocol(String),

    #[error("Schema retrieval failed: {0}")]
    SchemaRetrievalFailed(String),

    #[error("Operation not found: {0}")]
    OperationNotFound(String),

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("HTTP error {status_code}: {message}")]
    HttpError { status_code: u16, message: String },

    #[error("OAuth required: {0}")]
    OAuthRequired(String),

    #[error("OAuth discovery failed: {0}")]
    OAuthDiscoveryFailed(String),

    #[error("OAuth token exchange failed: {0}")]
    OAuthTokenExchangeFailed(String),

    #[error("OAuth session not found: {0}")]
    OAuthSessionNotFound(String),

    #[error("OAuth session expired: {0}")]
    OAuthSessionExpired(String),

    #[error("OAuth refresh failed: {0}")]
    OAuthRefreshFailed(String),

    #[error("OAuth scope insufficient: {0}")]
    OAuthScopeInsufficient(String),

    #[error("Daemon version mismatch: {0}")]
    DaemonVersionMismatch(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Generic error: {0}")]
    GenericError(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct StructuredError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl StructuredError {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Option<Value>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details,
        }
    }

    pub fn payload(&self) -> StructuredErrorPayload {
        StructuredErrorPayload {
            code: self.code.clone(),
            message: self.message.clone(),
            details: self.details.clone(),
        }
    }
}

pub fn structured_error_from_anyhow(err: &anyhow::Error) -> Option<StructuredErrorPayload> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<StructuredError>())
        .map(StructuredError::payload)
}

pub fn structured_error_from_jsonrpc_error(
    code: i64,
    message: &str,
    data: Option<&Value>,
    fallback_code: &str,
) -> StructuredError {
    match data.cloned() {
        Some(Value::Object(mut obj)) => {
            let structured_code = obj
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or(fallback_code)
                .to_string();
            let structured_message = obj
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(message)
                .to_string();
            obj.entry("jsonrpc_code".to_string()).or_insert(json!(code));
            obj.entry("jsonrpc_message".to_string())
                .or_insert(json!(message));
            StructuredError::new(
                structured_code,
                structured_message,
                Some(Value::Object(obj)),
            )
        }
        Some(other) => StructuredError::new(
            fallback_code,
            message,
            Some(json!({
                "jsonrpc_code": code,
                "jsonrpc_message": message,
                "data": other,
            })),
        ),
        None => StructuredError::new(
            fallback_code,
            message,
            Some(json!({
                "jsonrpc_code": code,
                "jsonrpc_message": message,
            })),
        ),
    }
}
