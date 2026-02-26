//! UXC error types

use thiserror::Error;

pub type Result<T> = std::result::Result<T, UxcError>;

#[derive(Error, Debug)]
pub enum UxcError {
    #[error("Protocol detection failed: {0}")]
    ProtocolDetectionFailed(String),

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

    #[error("OAuth refresh failed: {0}")]
    OAuthRefreshFailed(String),

    #[error("OAuth scope insufficient: {0}")]
    OAuthScopeInsufficient(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Generic error: {0}")]
    GenericError(#[from] anyhow::Error),
}
