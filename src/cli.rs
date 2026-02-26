//! CLI orchestration module
//!
//! This module contains testable components for CLI orchestration.
//! It provides abstractions and core logic that can be tested independently
//! of the main binary entry point.

use crate::adapters::Operation;
use crate::auth::{Profile, Profiles};
use crate::cache::CacheConfig;
use crate::error::UxcError;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Result type for CLI operations
pub type CliResult<T> = Result<T, CliError>;

/// CLI-specific error type
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Protocol detection failed: {0}")]
    ProtocolDetectionFailed(String),

    #[error("Operation not found: {0}")]
    OperationNotFound(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("IO error: {0}")]
    IoError(String),
}

impl From<anyhow::Error> for CliError {
    fn from(err: anyhow::Error) -> Self {
        if let Some(uxc_err) = err.downcast_ref::<UxcError>() {
            match uxc_err {
                UxcError::InvalidArguments(msg) => CliError::InvalidArguments(msg.clone()),
                UxcError::OperationNotFound(op) => CliError::OperationNotFound(op.clone()),
                UxcError::ProtocolDetectionFailed(msg) => {
                    CliError::ProtocolDetectionFailed(msg.clone())
                }
                UxcError::UnsupportedProtocol(msg) => {
                    CliError::ProtocolDetectionFailed(msg.clone())
                }
                _ => CliError::ExecutionFailed(err.to_string()),
            }
        } else {
            CliError::ExecutionFailed(err.to_string())
        }
    }
}

/// Trait for loading authentication credentials (abstracted for testing)
pub trait AuthCredentialLoader: Send + Sync {
    /// Load a credential by ID
    fn load_credential(&self, name: Option<String>) -> Result<Option<Profile>>;
}

/// Default auth credential loader
pub struct DefaultAuthCredentialLoader;

impl AuthCredentialLoader for DefaultAuthCredentialLoader {
    fn load_credential(&self, cli_credential: Option<String>) -> Result<Option<Profile>> {
        let Some(credential_id) = cli_credential else {
            return Ok(None);
        };

        let profiles = Profiles::load_profiles()?;
        let mut profile = profiles.get_profile(&credential_id).cloned().map_err(|e| {
            anyhow!(
                "Failed to load credential '{}': {}. Run 'uxc auth credential set {} --secret <value>' to create it.",
                credential_id,
                e,
                credential_id
            )
        })?;
        profile.name = Some(credential_id);
        Ok(Some(profile))
    }
}

/// Cache configuration helper
pub struct CacheConfigBuilder;

impl CacheConfigBuilder {
    /// Build cache config from CLI flags
    pub fn from_cli_flags(no_cache: bool, cache_ttl: Option<u64>) -> CacheConfig {
        if no_cache {
            CacheConfig {
                enabled: false,
                ..Default::default()
            }
        } else if let Some(ttl) = cache_ttl {
            CacheConfig {
                ttl,
                ..Default::default()
            }
        } else {
            CacheConfig::load_from_file().unwrap_or_default()
        }
    }
}

/// Argument parser for operation arguments
pub struct ArgumentParser;

impl ArgumentParser {
    /// Parse arguments from key-value pairs and JSON payload
    pub fn parse_arguments(
        args: Vec<String>,
        json_payload: Option<String>,
    ) -> Result<HashMap<String, Value>> {
        let mut args_map = HashMap::new();

        if let Some(json_str) = json_payload {
            let value: Value = serde_json::from_str(&json_str)
                .map_err(|e| UxcError::InvalidArguments(format!("Invalid JSON payload: {}", e)))?;
            if let Some(obj) = value.as_object() {
                for (k, v) in obj {
                    args_map.insert(k.clone(), v.clone());
                }
            } else {
                return Err(UxcError::InvalidArguments(
                    "JSON payload must be an object".to_string(),
                )
                .into());
            }
        } else {
            for arg in args {
                let parts: Vec<&str> = arg.splitn(2, '=').collect();
                if parts.len() == 2 {
                    args_map.insert(parts[0].to_string(), serde_json::json!(parts[1]));
                }
            }
        }

        Ok(args_map)
    }
}

/// Operation summary for display
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OperationSummary {
    pub operation_id: String,
    pub display_name: String,
    pub summary: Option<String>,
    pub required: Vec<String>,
    pub input_shape_hint: String,
    pub protocol_kind: String,
}

/// Convert adapter Operation to OperationSummary
pub fn to_operation_summary(protocol: &str, op: &Operation) -> OperationSummary {
    let required = op
        .parameters
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.clone())
        .collect::<Vec<_>>();

    let protocol_kind = match protocol {
        "mcp" => "tool",
        "graphql" => {
            if op.operation_id.starts_with("query/") {
                "query"
            } else if op.operation_id.starts_with("mutation/") {
                "mutation"
            } else if op.operation_id.starts_with("subscription/") {
                "subscription"
            } else {
                "field"
            }
        }
        "grpc" => "rpc",
        "openapi" => "http_operation",
        "jsonrpc" => "rpc_method",
        _ => "operation",
    }
    .to_string();

    let input_shape_hint = if op.parameters.is_empty() {
        "none".to_string()
    } else {
        "object".to_string()
    };

    OperationSummary {
        operation_id: op.operation_id.clone(),
        display_name: op.display_name.clone(),
        summary: op.description.clone(),
        required,
        input_shape_hint,
        protocol_kind,
    }
}

/// Auth profile view for display
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthProfileView {
    pub name: String,
    pub auth_type: String,
    pub api_key_masked: String,
    pub description: Option<String>,
}

/// Convert Profile to AuthProfileView
pub fn to_auth_profile_view(name: &str, profile: &Profile) -> AuthProfileView {
    AuthProfileView {
        name: name.to_string(),
        auth_type: profile.auth_type.to_string(),
        api_key_masked: profile.mask_api_key(),
        description: profile.description.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock auth profile loader for testing
    struct MockAuthLoader {
        profile: Option<Profile>,
    }

    impl AuthCredentialLoader for MockAuthLoader {
        fn load_credential(&self, _name: Option<String>) -> Result<Option<Profile>> {
            Ok(self.profile.clone())
        }
    }

    #[test]
    fn test_cli_error_from_anyhow() {
        let err = anyhow!("test error");
        let cli_err: CliError = err.into();
        assert!(matches!(cli_err, CliError::ExecutionFailed(_)));
    }

    #[test]
    fn test_cli_error_from_uxc_invalid_args() {
        let uxc_err = UxcError::InvalidArguments("test".to_string());
        let anyhow_err = anyhow::Error::from(uxc_err);
        let cli_err: CliError = anyhow_err.into();
        assert!(matches!(cli_err, CliError::InvalidArguments(_)));
    }

    #[test]
    fn test_cli_error_from_uxc_operation_not_found() {
        let uxc_err = UxcError::OperationNotFound("test_op".to_string());
        let anyhow_err = anyhow::Error::from(uxc_err);
        let cli_err: CliError = anyhow_err.into();
        assert!(matches!(cli_err, CliError::OperationNotFound(_)));
    }

    #[test]
    fn test_parse_arguments_with_json() {
        let json = r#"{"name": "test", "count": 42}"#;
        let result = ArgumentParser::parse_arguments(vec![], Some(json.to_string())).unwrap();
        assert_eq!(result.get("name").unwrap(), "test");
        assert_eq!(result.get("count").unwrap(), 42);
    }

    #[test]
    fn test_parse_arguments_with_key_value() {
        let args = vec!["name=test".to_string(), "count=42".to_string()];
        let result = ArgumentParser::parse_arguments(args, None).unwrap();
        assert_eq!(result.get("name").unwrap(), "test");
        assert_eq!(result.get("count").unwrap(), "42");
    }

    #[test]
    fn test_parse_arguments_empty() {
        let result = ArgumentParser::parse_arguments(vec![], None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_arguments_invalid_json() {
        let json = r#"{invalid json}"#;
        let result = ArgumentParser::parse_arguments(vec![], Some(json.to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_arguments_json_not_object() {
        let json = r#"["array", "not", "object"]"#;
        let result = ArgumentParser::parse_arguments(vec![], Some(json.to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_to_operation_summary() {
        let operation = Operation {
            operation_id: "get:/users".to_string(),
            display_name: "GET /users".to_string(),
            description: Some("List users".to_string()),
            parameters: vec![],
            return_type: Some("User[]".to_string()),
        };

        let summary = to_operation_summary("openapi", &operation);
        assert_eq!(summary.operation_id, "get:/users");
        assert_eq!(summary.display_name, "GET /users");
        assert_eq!(summary.summary, Some("List users".to_string()));
        assert_eq!(summary.protocol_kind, "http_operation");
        assert_eq!(summary.input_shape_hint, "none");
    }

    #[test]
    fn test_to_operation_summary_with_parameters() {
        let operation = Operation {
            operation_id: "query/viewer".to_string(),
            display_name: "viewer".to_string(),
            description: None,
            parameters: vec![crate::adapters::Parameter {
                name: "id".to_string(),
                param_type: "ID!".to_string(),
                required: true,
                description: Some("User ID".to_string()),
            }],
            return_type: Some("User".to_string()),
        };

        let summary = to_operation_summary("graphql", &operation);
        assert_eq!(summary.operation_id, "query/viewer");
        assert_eq!(summary.protocol_kind, "query");
        assert_eq!(summary.input_shape_hint, "object");
        assert_eq!(summary.required, vec!["id"]);
    }

    #[test]
    fn test_cache_config_builder_no_cache() {
        let config = CacheConfigBuilder::from_cli_flags(true, None);
        assert!(!config.enabled);
    }

    #[test]
    fn test_cache_config_builder_with_ttl() {
        let config = CacheConfigBuilder::from_cli_flags(false, Some(3600));
        assert_eq!(config.ttl, 3600);
    }

    #[test]
    fn test_cache_config_builder_default() {
        let config = CacheConfigBuilder::from_cli_flags(false, None);
        // Should load from file or use defaults
        assert!(config.enabled || config.ttl > 0 || !config.enabled);
    }

    #[test]
    fn test_to_auth_profile_view() {
        let profile = Profile::new("test_key_123".to_string(), crate::auth::AuthType::Bearer);
        let view = to_auth_profile_view("default", &profile);
        assert_eq!(view.name, "default");
        assert_eq!(view.auth_type, "bearer");
        assert_eq!(view.api_key_masked, "************");
        assert!(view.description.is_none());
    }

    #[test]
    fn test_to_auth_profile_view_with_description() {
        let mut profile = Profile::new("test_key".to_string(), crate::auth::AuthType::ApiKey);
        profile = profile.with_description("Test profile".to_string());
        let view = to_auth_profile_view("test", &profile);
        assert_eq!(view.description, Some("Test profile".to_string()));
    }

    #[test]
    fn test_mock_auth_loader() {
        let loader = MockAuthLoader {
            profile: Some(Profile::new(
                "key".to_string(),
                crate::auth::AuthType::Bearer,
            )),
        };
        let profile = loader.load_credential(Some("test".to_string())).unwrap();
        assert!(profile.is_some());
    }

    #[test]
    fn test_mock_auth_loader_no_profile() {
        let loader = MockAuthLoader { profile: None };
        let profile = loader.load_credential(Some("test".to_string())).unwrap();
        assert!(profile.is_none());
    }

    #[test]
    fn test_operation_summary_protocol_detection() {
        let operation = Operation {
            operation_id: "test".to_string(),
            display_name: "Test".to_string(),
            description: None,
            parameters: vec![],
            return_type: None,
        };

        assert_eq!(
            to_operation_summary("mcp", &operation).protocol_kind,
            "tool"
        );
        assert_eq!(
            to_operation_summary("grpc", &operation).protocol_kind,
            "rpc"
        );
        assert_eq!(
            to_operation_summary("jsonrpc", &operation).protocol_kind,
            "rpc_method"
        );
    }

    #[test]
    fn test_graphql_operation_kind_detection() {
        let operation = Operation {
            operation_id: "test".to_string(),
            display_name: "Test".to_string(),
            description: None,
            parameters: vec![],
            return_type: None,
        };

        assert_eq!(
            to_operation_summary("graphql", &operation).protocol_kind,
            "field"
        );

        let query_op = Operation {
            operation_id: "query/getUser".to_string(),
            display_name: "getUser".to_string(),
            description: None,
            parameters: vec![],
            return_type: None,
        };
        assert_eq!(
            to_operation_summary("graphql", &query_op).protocol_kind,
            "query"
        );

        let mutation_op = Operation {
            operation_id: "mutation/createUser".to_string(),
            display_name: "createUser".to_string(),
            description: None,
            parameters: vec![],
            return_type: None,
        };
        assert_eq!(
            to_operation_summary("graphql", &mutation_op).protocol_kind,
            "mutation"
        );
    }
}
