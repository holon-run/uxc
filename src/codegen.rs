use crate::adapters::{Operation, OperationDetail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenHostSchemaV1 {
    pub version: String,
    pub generated_at_unix: u64,
    pub host: CodegenHost,
    pub runtime: CodegenRuntimeContract,
    pub operations: Vec<CodegenOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenHost {
    pub id: String,
    pub endpoint: String,
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenRuntimeContract {
    pub invoke_options_schema: Value,
    pub result_meta_schema: Value,
    pub artifact_meta_schema: Value,
    pub lifecycle_contract: Value,
    pub artifact_contract: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenOperation {
    pub id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: String,
    pub input_schema: Option<Value>,
    pub output_schema: Option<Value>,
    pub result_kind: String,
    pub execute: bool,
    pub help_only: bool,
    pub subscribable: bool,
}

pub fn build_codegen_host_schema(
    endpoint: &str,
    protocol: &str,
    link_name: Option<&str>,
    operations: &[Operation],
    operation_details: &HashMap<String, OperationDetail>,
    generated_at_unix: u64,
) -> CodegenHostSchemaV1 {
    let link_name = link_name
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    let host_id = link_name.clone().unwrap_or_else(|| endpoint.to_string());

    let mut generated_operations = operations
        .iter()
        .map(|op| {
            let detail = operation_details.get(&op.operation_id);
            let input_schema = detail.and_then(|d| d.input_schema.clone());
            let description = detail
                .and_then(|d| d.description.clone())
                .or_else(|| op.description.clone());
            let subscribable = operation_subscribable(protocol, &op.operation_id);
            let kind = operation_kind(protocol, &op.operation_id);
            CodegenOperation {
                id: op.operation_id.clone(),
                display_name: op.display_name.clone(),
                description,
                kind,
                input_schema,
                output_schema: None,
                result_kind: if subscribable {
                    "subscription_event".to_string()
                } else {
                    "call_result".to_string()
                },
                execute: true,
                help_only: false,
                subscribable,
            }
        })
        .collect::<Vec<_>>();
    generated_operations.sort_by(|a, b| a.id.cmp(&b.id));

    CodegenHostSchemaV1 {
        version: "v1".to_string(),
        generated_at_unix,
        host: CodegenHost {
            id: host_id,
            endpoint: endpoint.to_string(),
            protocol: protocol.to_string(),
            link_name,
        },
        runtime: CodegenRuntimeContract {
            invoke_options_schema: invoke_options_schema(),
            result_meta_schema: result_meta_schema(),
            artifact_meta_schema: artifact_meta_schema(),
            lifecycle_contract: lifecycle_contract(),
            artifact_contract: artifact_contract(),
        },
        operations: generated_operations,
    }
}

fn operation_kind(protocol: &str, operation_id: &str) -> String {
    match protocol {
        "openapi" => "execute".to_string(),
        "mcp" => "execute".to_string(),
        "grpc" => "execute".to_string(),
        "graphql" => {
            if operation_id.starts_with("subscription/") {
                "subscription".to_string()
            } else if operation_id.starts_with("query/") {
                "query".to_string()
            } else if operation_id.starts_with("mutation/") {
                "mutation".to_string()
            } else {
                "execute".to_string()
            }
        }
        "jsonrpc" => {
            if operation_subscribable(protocol, operation_id) {
                "subscription".to_string()
            } else {
                "execute".to_string()
            }
        }
        _ => "execute".to_string(),
    }
}

fn operation_subscribable(protocol: &str, operation_id: &str) -> bool {
    match protocol {
        "graphql" => operation_id.starts_with("subscription/"),
        "jsonrpc" => {
            let normalized = operation_id.to_ascii_lowercase();
            normalized.contains("subscribe") && !normalized.contains("unsubscribe")
        }
        _ => false,
    }
}

fn invoke_options_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "auth": { "type": ["string", "null"] },
            "inject_env": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "template": { "type": "string" }
                    },
                    "required": ["name", "template"]
                },
                "default": []
            },
            "no_cache": { "type": "boolean", "default": false },
            "cache_ttl": { "type": ["integer", "null"], "minimum": 0 },
            "timeout_ms": { "type": ["integer", "null"], "minimum": 0 },
            "refresh_schema": { "type": "boolean", "default": false },
            "schema_url": { "type": ["string", "null"] },
            "link_name": { "type": ["string", "null"] },
            "schema_mapping_file": { "type": ["string", "null"] },
            "daemon_exclusive": {
                "type": "array",
                "items": { "type": "string" },
                "default": []
            },
            "daemon_idle_ttl": { "type": ["integer", "null"], "minimum": 0 }
        },
        "session_identity_fields": [
            "endpoint",
            "auth_fingerprint",
            "inject_env_fingerprint",
            "runtime_family"
        ],
        "ownership_policy_fields": [
            "daemon_exclusive",
            "in_flight_requests",
            "subscription_held"
        ],
        "presentation_cleanup_fields": [
            "link_name",
            "daemon_idle_ttl"
        ]
    })
}

fn result_meta_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_involved": { "type": ["boolean", "null"] },
            "cache_source": { "type": ["string", "null"] },
            "cache_age_ms": { "type": ["integer", "null"], "minimum": 0 },
            "cache_stale": { "type": ["boolean", "null"] },
            "cache_fallback": { "type": ["boolean", "null"] },
            "daemon_session_reused": { "type": ["boolean", "null"] },
            "artifact_truncated": { "type": ["boolean", "null"] },
            "artifact_kind": { "type": ["string", "null"] },
            "artifact_bytes": { "type": ["integer", "null"], "minimum": 0 },
            "artifact_path": { "type": ["string", "null"] },
            "artifact_ref": { "type": ["string", "null"] },
            "artifact_sha256": { "type": ["string", "null"] }
        }
    })
}

fn artifact_meta_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string" },
            "name": { "type": ["string", "null"] },
            "path": { "type": ["string", "null"] },
            "ref": { "type": ["string", "null"] },
            "mimeType": { "type": ["string", "null"] },
            "bytes": { "type": ["integer", "null"], "minimum": 0 },
            "sha256": { "type": ["string", "null"] },
            "source": { "type": ["string", "null"] },
            "description": { "type": ["string", "null"] }
        },
        "required": ["kind"]
    })
}

fn lifecycle_contract() -> Value {
    json!({
        "version": "v1",
        "session_identity": [
            "endpoint",
            "auth_fingerprint",
            "inject_env_fingerprint",
            "runtime_family"
        ],
        "ownership_policy": {
            "daemon_exclusive_is_identity": false,
            "busy_sessions_evictable": false,
            "idle_sessions_evictable": true
        },
        "idle_policy": {
            "latest_request_wins_ttl": true,
            "ttl_zero_disables_reap": true,
            "daemon_busy_blocks_idle": true,
            "subscription_held_blocks_idle": true
        },
        "idle_reap_requirements": [
            "not_subscription_held",
            "not_busy",
            "idle_ttl_non_zero",
            "idle_exceeded",
            "can_reap_or_best_effort_cleanup"
        ],
        "observable_session_fields": [
            "endpoint",
            "protocol",
            "link_name",
            "idle_ttl_secs",
            "expires_in_secs",
            "daemon_exclusive",
            "in_flight_requests",
            "reuse_eligible",
            "can_reap_contract"
        ]
    })
}

fn artifact_contract() -> Value {
    json!({
        "version": "v1",
        "input_model": {
            "file_input_requires_schema_marking": true,
            "local_file_input_repr": "local_path_string",
            "supports_mixed_argument_models": [
                "key=value",
                "nested_path_assignment",
                "json_assignment",
                "positional_json_object"
            ]
        },
        "output_model": {
            "artifact_reference_may_appear_in_data": true,
            "local_path_supported": true,
            "daemon_ref_supported": false
        },
        "compaction_model": {
            "automatic_above_bytes": 65536,
            "preview_inline": true,
            "full_payload_externalized": true,
            "applies_to_result_kinds": [
                "host_help",
                "operation_detail",
                "call_result"
            ],
            "excluded_result_kinds": [
                "codegen_host_schema"
            ],
            "meta_fields": [
                "artifact_truncated",
                "artifact_kind",
                "artifact_bytes",
                "artifact_path",
                "artifact_ref",
                "artifact_sha256"
            ]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_subscribable_detection_matches_subscribe_family() {
        assert!(operation_subscribable("jsonrpc", "eth_subscribe"));
        assert!(operation_subscribable("jsonrpc", "wallet_subscribeEvents"));
        assert!(!operation_subscribable("jsonrpc", "eth_unsubscribe"));
        assert!(!operation_subscribable("jsonrpc", "web3_clientVersion"));
    }
}
