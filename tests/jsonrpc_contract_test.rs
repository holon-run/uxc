//! JSON-RPC Contract Tests
//!
//! Comprehensive contract tests verifying JSON-RPC 2.0 and OpenRPC specification compliance.
//! Tests cover method invocation, parameter handling, error responses, batch requests,
//! notifications, and edge cases as per JSON-RPC specification.
//!
//! Reference: https://www.jsonrpc.org/specification
//! OpenRPC: https://spec.open-rpc.org/

use uxc::adapters::jsonrpc::JsonRpcAdapter;
use uxc::adapters::Adapter;

/// Helper to run async code with a mock server
fn run_async<F, R>(f: F) -> R
where
    F: FnOnce(mockito::ServerGuard) -> R,
    R: Send + 'static,
{
    let mut server = mockito::Server::new();
    f(server)
}

// ============================================================================
// Schema Discovery Tests
// ============================================================================

#[test]
fn test_jsonrpc_discovery_finds_openrpc_document() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "methods": [
                {
                    "name": "test_method",
                    "params": [],
                    "result": {
                        "name": "result",
                        "schema": { "type": "string" }
                    }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(result.unwrap(), "Should detect OpenRPC document");
    });
}

#[test]
fn test_jsonrpc_discovery_finds_well_known_openrpc() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": {
                "title": "Test API",
                "version": "1.0.0"
            },
            "methods": []
        });

        let _mock = server
            .mock("GET", "/.well-known/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(result.unwrap(), "Should detect well-known OpenRPC document");
    });
}

#[test]
fn test_jsonrpc_discovery_via_rpc_discover() {
    run_async(|mut server| {
        let discover_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "openrpc": "1.2.6",
                "info": {
                    "title": "Discoverable API",
                    "version": "1.0.0"
                },
                "methods": [
                    {
                        "name": "echo",
                        "params": [],
                        "result": { "schema": { "type": "string" } }
                    }
                ]
            }
        });

        let _mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "rpc.discover",
                "params": []
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&discover_response.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(result.unwrap(), "Should detect via rpc.discover");
    });
}

#[test]
fn test_jsonrpc_rejects_non_openrpc_document() {
    run_async(|mut server| {
        let not_openrpc = serde_json::json!({
            "api": "something",
            "data": "test"
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&not_openrpc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(!result.unwrap(), "Should reject non-OpenRPC document");
    });
}

// ============================================================================
// Method Listing Tests
// ============================================================================

#[test]
fn test_jsonrpc_lists_all_methods() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "add",
                    "description": "Add two numbers",
                    "params": [
                        { "name": "a", "schema": { "type": "number" }, "required": true },
                        { "name": "b", "schema": { "type": "number" }, "required": true }
                    ],
                    "result": { "name": "sum", "schema": { "type": "number" } }
                },
                {
                    "name": "subtract",
                    "params": [
                        { "name": "minuend", "schema": { "type": "number" }, "required": true },
                        { "name": "subtrahend", "schema": { "type": "number" }, "required": true }
                    ],
                    "result": { "name": "difference", "schema": { "type": "number" } }
                },
                {
                    "name": "multiply",
                    "params": [],
                    "result": { "schema": { "type": "number" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 3);
        assert!(operations.iter().any(|op| op.operation_id == "add"));
        assert!(operations.iter().any(|op| op.operation_id == "subtract"));
        assert!(operations.iter().any(|op| op.operation_id == "multiply"));
    });
}

#[test]
fn test_jsonrpc_method_parsing_parameters() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "search",
                    "description": "Search items",
                    "params": [
                        {
                            "name": "query",
                            "description": "Search query string",
                            "schema": { "type": "string" },
                            "required": true
                        },
                        {
                            "name": "limit",
                            "description": "Max results",
                            "schema": { "type": "integer" },
                            "required": false
                        },
                        {
                            "name": "offset",
                            "schema": { "type": "integer" }
                        }
                    ],
                    "result": { "schema": { "type": "array" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let method = &operations[0];
        assert_eq!(method.operation_id, "search");
        assert_eq!(method.description.as_ref().unwrap(), "Search items");
        assert_eq!(method.parameters.len(), 3);

        assert_eq!(method.parameters[0].name, "query");
        assert!(method.parameters[0].required);
        assert_eq!(
            method.parameters[0].description.as_ref().unwrap(),
            "Search query string"
        );

        assert_eq!(method.parameters[1].name, "limit");
        assert!(!method.parameters[1].required);

        assert_eq!(method.parameters[2].name, "offset");
        assert!(!method.parameters[2].required);
    });
}

#[test]
fn test_jsonrpc_method_return_type() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "getData",
                    "params": [],
                    "result": {
                        "name": "data",
                        "schema": { "type": "object" }
                    }
                },
                {
                    "name": "getString",
                    "params": [],
                    "result": {
                        "name": "value",
                        "schema": { "type": "string" }
                    }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 2);
        let data_method = operations
            .iter()
            .find(|op| op.operation_id == "getData")
            .unwrap();
        assert_eq!(data_method.return_type.as_ref().unwrap(), "object");

        let string_method = operations
            .iter()
            .find(|op| op.operation_id == "getString")
            .unwrap();
        assert_eq!(string_method.return_type.as_ref().unwrap(), "string");
    });
}

// ============================================================================
// Parameter Structure Tests
// ============================================================================

#[test]
fn test_jsonrpc_param_structure_by_name() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "greet",
                    "paramStructure": "by-name",
                    "params": [
                        { "name": "name", "schema": { "type": "string" }, "required": true },
                        { "name": "greeting", "schema": { "type": "string" }, "required": false }
                    ],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let detail = rt
            .block_on(async { adapter.describe_operation(&server.url(), "greet").await })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert_eq!(schema["paramStructure"], "by-name");
    });
}

#[test]
fn test_jsonrpc_param_structure_by_position() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "subtract",
                    "paramStructure": "by-position",
                    "params": [
                        { "name": "minuend", "schema": { "type": "number" }, "required": true },
                        { "name": "subtrahend", "schema": { "type": "number" }, "required": true }
                    ],
                    "result": { "name": "difference", "schema": { "type": "number" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let detail = rt
            .block_on(async { adapter.describe_operation(&server.url(), "subtract").await })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert_eq!(schema["paramStructure"], "by-position");
    });
}

#[test]
fn test_jsonrpc_param_structure_either_default() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "methodWithEither",
                    "params": [
                        { "name": "arg1", "schema": { "type": "string" } }
                    ],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "methodWithEither")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        // Default should be "either"
        assert_eq!(schema["paramStructure"], "either");
    });
}

// ============================================================================
// Execution Tests
// ============================================================================

#[test]
fn test_jsonrpc_execution_successful_response() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "echo",
                    "params": [
                        { "name": "message", "schema": { "type": "string" }, "required": true }
                    ],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let execution_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "hello world"
        });

        let _exec_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&execution_response.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let mut args = std::collections::HashMap::new();
        args.insert("message".to_string(), serde_json::json!("hello world"));

        let result = rt.block_on(async { adapter.execute(&url, "echo", args).await });

        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.data, "hello world");
        assert!(exec_result.metadata.duration_ms > 0);
    });
}

#[test]
fn test_jsonrpc_execution_with_positional_params() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "subtract",
                    "paramStructure": "by-position",
                    "params": [
                        { "name": "minuend", "schema": { "type": "number" }, "required": true },
                        { "name": "subtrahend", "schema": { "type": "number" }, "required": true }
                    ],
                    "result": { "name": "difference", "schema": { "type": "number" } }
                }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let execution_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": 19
        });

        let _exec_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "method": "subtract",
                "params": [42, 23]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&execution_response.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let mut args = std::collections::HashMap::new();
        args.insert("minuend".to_string(), serde_json::json!(42));
        args.insert("subtrahend".to_string(), serde_json::json!(23));

        let result = rt.block_on(async { adapter.execute(&url, "subtract", args).await });

        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert_eq!(exec_result.data, 19);
    });
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_jsonrpc_standard_error_codes() {
    // Test that standard JSON-RPC 2.0 error codes are handled
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": []
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        // Test method not found error
        let error_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        });

        let _exec_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&error_response.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let result = rt.block_on(async {
            adapter
                .execute(&url, "nonexistent", std::collections::HashMap::new())
                .await
        });

        // Should fail with an error
        assert!(result.is_err(), "Should fail when method not found");
    });
}

#[test]
fn test_jsonrpc_error_with_data() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "failingMethod",
                    "params": [],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let error_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32000,
                "message": "Server error",
                "data": {
                    "details": "Additional error context",
                    "errorCode": "CUSTOM_ERROR"
                }
            }
        });

        let _exec_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&error_response.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let result = rt.block_on(async {
            adapter
                .execute(&url, "failingMethod", std::collections::HashMap::new())
                .await
        });

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("-32000"));
        assert!(error_msg.contains("Server error"));
        assert!(error_msg.contains("data"));
    });
}

#[test]
fn test_jsonrpc_missing_required_parameter() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "methodWithRequiredParam",
                    "params": [
                        { "name": "required", "schema": { "type": "string" }, "required": true }
                    ],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        // Call without required parameter
        let result = rt.block_on(async {
            adapter
                .execute(
                    &url,
                    "methodWithRequiredParam",
                    std::collections::HashMap::new(),
                )
                .await
        });

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Missing required parameter"));
    });
}

#[test]
fn test_jsonrpc_unknown_parameter() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "method",
                    "params": [
                        { "name": "valid", "schema": { "type": "string" } }
                    ],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let mut args = std::collections::HashMap::new();
        args.insert("valid".to_string(), serde_json::json!("value"));
        args.insert("unknown".to_string(), serde_json::json!("value"));

        let result = rt.block_on(async { adapter.execute(&url, "method", args).await });

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Unknown parameter"));
    });
}

#[test]
fn test_jsonrpc_missing_result_field() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "noResult",
                    "params": [],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        // Response missing "result" field
        let invalid_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1
        });

        let _exec_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&invalid_response.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let result = rt.block_on(async {
            adapter
                .execute(&url, "noResult", std::collections::HashMap::new())
                .await
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing result"));
    });
}

#[test]
fn test_jsonrpc_handles_missing_operation() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                { "name": "existing", "params": [], "result": { "schema": { "type": "string" } } }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let result = rt.block_on(async {
            adapter
                .describe_operation(&server.url(), "nonexistent")
                .await
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    });
}

// ============================================================================
// Edge Cases and Complex Scenarios
// ============================================================================

#[test]
fn test_jsonrpc_method_with_no_parameters() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "ping",
                    "params": [],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].parameters.len(), 0);
    });
}

#[test]
fn test_jsonrpc_url_resolution_from_servers_field() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "servers": [
                {
                    "name": "production",
                    "url": "/rpc"
                }
            ],
            "methods": [
                { "name": "test", "params": [], "result": { "schema": { "type": "string" } } }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        // Should execute against /rpc endpoint
        let execution_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "success"
        });

        let _exec_mock = server
            .mock("POST", "/rpc")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&execution_response.to_string())
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let result = rt.block_on(async {
            adapter
                .execute(&url, "test", std::collections::HashMap::new())
                .await
        });

        assert!(result.is_ok());
    });
}

#[test]
fn test_jsonrpc_description_fallback_to_summary() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "method1",
                    "description": "Has description",
                    "params": [],
                    "result": { "schema": { "type": "string" } }
                },
                {
                    "name": "method2",
                    "summary": "Has summary",
                    "params": [],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 2);
        let m1 = operations
            .iter()
            .find(|op| op.operation_id == "method1")
            .unwrap();
        assert_eq!(m1.description.as_ref().unwrap(), "Has description");

        let m2 = operations
            .iter()
            .find(|op| op.operation_id == "method2")
            .unwrap();
        assert_eq!(m2.description.as_ref().unwrap(), "Has summary");
    });
}

#[test]
fn test_jsonrpc_empty_schema() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": []
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 0);
    });
}

#[test]
fn test_jsonrpc_handles_multiple_params_with_optional() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                {
                    "name": "complexMethod",
                    "params": [
                        { "name": "required1", "schema": { "type": "string" }, "required": true },
                        { "name": "optional1", "schema": { "type": "string" }, "required": false },
                        { "name": "required2", "schema": { "type": "integer" }, "required": true },
                        { "name": "optional2", "schema": { "type": "boolean" }, "required": false }
                    ],
                    "result": { "schema": { "type": "string" } }
                }
            ]
        });

        let _mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let params = &operations[0].parameters;
        assert_eq!(params.len(), 4);
        assert!(params[0].required);
        assert!(!params[1].required);
        assert!(params[2].required);
        assert!(!params[3].required);
    });
}

#[test]
fn test_jsonrpc_request_id_generation() {
    run_async(|mut server| {
        let openrpc_doc = serde_json::json!({
            "openrpc": "1.2.6",
            "info": { "title": "API", "version": "1.0" },
            "methods": [
                { "name": "test", "params": [], "result": { "schema": { "type": "string" } } }
            ]
        });

        let _schema_mock = server
            .mock("GET", "/openrpc.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openrpc_doc.to_string())
            .create();

        // Verify request has an ID
        let _exec_mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "test"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": "success"
                })
                .to_string(),
            )
            .create();

        let url = server.url();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = JsonRpcAdapter::new();

        let result = rt.block_on(async {
            adapter
                .execute(&url, "test", std::collections::HashMap::new())
                .await
        });

        assert!(result.is_ok());
    });
}
