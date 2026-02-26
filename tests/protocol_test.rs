//! Protocol routing and detection tests
//!
//! This test module comprehensively covers:
//! - Protocol detection for each supported protocol (MCP, GraphQL, OpenAPI, JSON-RPC, gRPC)
//! - Adapter retrieval with and without options
//! - Detection order verification
//! - Error paths and edge cases
//!
//! Note: Due to mockito 1.2 compatibility issues with tokio::test,
//! HTTP-based tests use a manual runtime pattern.

use uxc::adapters::graphql::GraphQLAdapter;
use uxc::adapters::jsonrpc::JsonRpcAdapter;
use uxc::adapters::mcp::McpAdapter;
use uxc::adapters::openapi::OpenAPIAdapter;
use uxc::adapters::{Adapter, AdapterEnum, DetectionOptions, ProtocolDetector, ProtocolType};
use uxc::protocol::ProtocolRouter;

/// Helper to run async code with a mock server
fn run_async_with_server<F, R>(f: F) -> R
where
    F: FnOnce(String) -> R,
    R: Send + 'static,
{
    let mut server = mockito::Server::new();
    let url = server.url();
    f(url)
}

#[test]
fn test_protocol_router_detect_graphql() {
    run_async_with_server(|url| {
        let mut server = mockito::Server::new();

        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "hello",
                                "description": "Say hello",
                                "args": [],
                                "type": {
                                    "kind": "SCALAR",
                                    "name": "String",
                                    "ofType": null
                                }
                            }
                        ]
                    },
                    "mutationType": null,
                    "subscriptionType": null
                }
            }
        });

        let _mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&introspection_response.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let router = ProtocolRouter::new();
            router.detect_protocol(&server.url()).await
        });

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ProtocolType::GraphQL);
    })
}

#[test]
fn test_protocol_router_detect_openapi() {
    let mut server = mockito::Server::new();

    let openapi_doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {
            "/users": {
                "get": {
                    "operationId": "getUsers",
                    "responses": {
                        "200": {
                            "description": "Success"
                        }
                    }
                }
            }
        }
    });

    let _mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_doc.to_string())
        .create();

    let url = format!("{}/openapi.json", server.url());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol(&url).await
    });

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ProtocolType::OpenAPI);
}

#[test]
fn test_protocol_router_detect_jsonrpc() {
    let mut server = mockito::Server::new();

    let openrpc_doc = serde_json::json!({
        "openrpc": "1.2.6",
        "info": {
            "title": "Test JSON-RPC API",
            "version": "1.0.0"
        },
        "methods": [
            {
                "name": "test_method",
                "description": "A test method",
                "params": [],
                "result": {
                    "name": "result",
                    "schema": {
                        "type": "string"
                    }
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

    let url = format!("{}/openrpc.json", server.url());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol(&url).await
    });

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ProtocolType::JsonRpc);
}

#[test]
fn test_protocol_router_detect_mcp_stdio() {
    // MCP stdio commands are detected based on format, not network
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Test npx command
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router
            .detect_protocol("npx @modelcontextprotocol/server-everything")
            .await
    });
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ProtocolType::Mcp);

    // Test command with path
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol("./my-mcp-server").await
    });
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ProtocolType::Mcp);

    // Test absolute path
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol("/usr/local/bin/my-mcp-server").await
    });
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ProtocolType::Mcp);
}

#[test]
fn test_protocol_router_detection_order_graphql_wins() {
    let mut server = mockito::Server::new();

    let graphql_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": []
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let openapi_response = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Ambiguous API",
            "version": "1.0.0"
        },
        "paths": {}
    });

    let jsonrpc_response = serde_json::json!({
        "openrpc": "1.2.6",
        "info": {
            "title": "Ambiguous API",
            "version": "1.0.0"
        },
        "methods": []
    });

    let _graphql_mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&graphql_response.to_string())
        .create();

    let _openapi_mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_response.to_string())
        .create();

    let _jsonrpc_mock = server
        .mock("GET", "/openrpc.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&jsonrpc_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol(&url).await
    });

    assert!(result.is_ok());
    // GraphQL should be detected first (before OpenAPI and JSON-RPC)
    assert_eq!(result.unwrap(), ProtocolType::GraphQL);
}

#[test]
fn test_protocol_router_get_adapter_graphql() {
    let mut server = mockito::Server::new();

    let introspection_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": [
                        {
                            "name": "hello",
                            "description": "Say hello",
                            "args": [],
                            "type": {
                                "kind": "SCALAR",
                                "name": "String",
                                "ofType": null
                            }
                        }
                    ]
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let _mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&introspection_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.get_adapter_for_url(&url).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::GraphQL);
}

#[test]
fn test_protocol_router_get_adapter_openapi() {
    let mut server = mockito::Server::new();

    let openapi_doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {
            "/users": {
                "get": {
                    "operationId": "getUsers",
                    "responses": {
                        "200": {
                            "description": "Success"
                        }
                    }
                }
            }
        }
    });

    let _mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_doc.to_string())
        .create();

    let url = format!("{}/openapi.json", server.url());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.get_adapter_for_url(&url).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::OpenAPI);
}

#[test]
fn test_protocol_router_get_adapter_with_schema_url_override() {
    let mut server = mockito::Server::new();

    let openapi_doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {
            "/users": {
                "get": {
                    "operationId": "getUsers",
                    "responses": {
                        "200": {
                            "description": "Success"
                        }
                    }
                }
            }
        }
    });

    let _mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_doc.to_string())
        .create();

    let base_url = server.url();
    let schema_url = format!("{}/openapi.json", base_url);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        let options = DetectionOptions {
            schema_url: Some(schema_url),
        };
        router
            .get_adapter_for_url_with_options(&base_url, &options)
            .await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::OpenAPI);
}

#[test]
fn test_protocol_router_unsupported_url() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol("unsupported://example.com").await
    });

    assert!(result.is_err(), "Should fail for unsupported protocol");
}

#[test]
fn test_protocol_router_unreachable_server() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol("http://localhost:59999/api").await
    });

    // Should fail gracefully
    assert!(result.is_err(), "Should fail for unreachable server");
}

#[test]
fn test_protocol_detector_mcp_stdio_command() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Test various MCP stdio command patterns
    let test_cases = vec![
        "npx @modelcontextprotocol/server-everything",
        "node my-mcp-server.js",
        "python3 /path/to/mcp-server.py",
        "./my-mcp-server",
        "/usr/local/bin/mcp-server",
        "mcp://custom-server",
    ];

    for command in test_cases {
        let result = rt.block_on(async {
            let detector = ProtocolDetector::new();
            detector.detect_adapter(command).await
        });

        assert!(result.is_ok(), "Should detect MCP stdio for: {}", command);
        let adapter = result.unwrap();
        assert_eq!(
            adapter.protocol_type(),
            ProtocolType::Mcp,
            "MCP stdio should be detected for: {}",
            command
        );
    }
}

#[test]
fn test_protocol_detector_http_vs_stdio() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // HTTP URLs should not be treated as stdio commands
    let http_url = "http://example.com/api";
    let https_url = "https://example.com/api";

    let detector = ProtocolDetector::new();

    // These should not be detected as MCP stdio
    let http_result = rt.block_on(async { detector.detect_adapter(http_url).await });
    let https_result = rt.block_on(async { detector.detect_adapter(https_url).await });

    // Both should fail (no actual server), but not because of stdio detection
    assert!(http_result.is_err() || https_result.is_err());
}

#[test]
fn test_protocol_detector_order_mcp_first() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // An MCP command with "node"
    let mcp_command = "node my-mcp-server";

    let result = rt.block_on(async {
        let detector = ProtocolDetector::new();
        detector.detect_adapter(mcp_command).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::Mcp);
}

#[test]
fn test_protocol_detector_order_graphql_before_openapi() {
    let mut server = mockito::Server::new();

    let graphql_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": []
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let openapi_response = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Ambiguous API",
            "version": "1.0.0"
        },
        "paths": {}
    });

    let jsonrpc_response = serde_json::json!({
        "openrpc": "1.2.6",
        "info": {
            "title": "Ambiguous API",
            "version": "1.0.0"
        },
        "methods": []
    });

    let _graphql_mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&graphql_response.to_string())
        .create();

    let _openapi_mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_response.to_string())
        .create();

    let _jsonrpc_mock = server
        .mock("GET", "/openrpc.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&jsonrpc_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let detector = ProtocolDetector::new();
        detector.detect_adapter(&url).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    // GraphQL should win (it's checked before OpenAPI)
    assert_eq!(adapter.protocol_type(), ProtocolType::GraphQL);
}

#[test]
fn test_protocol_detector_order_openapi_before_jsonrpc() {
    let mut server = mockito::Server::new();

    // Create a server that responds to OpenAPI but not GraphQL
    let openapi_response = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {}
    });

    let _openapi_mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_response.to_string())
        .create();

    // GraphQL should fail
    let _graphql_mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(404)
        .create();

    // JSON-RPC also available
    let jsonrpc_response = serde_json::json!({
        "openrpc": "1.2.6",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "methods": []
    });

    let _jsonrpc_mock = server
        .mock("GET", "/openrpc.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&jsonrpc_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let detector = ProtocolDetector::new();
        detector.detect_adapter(&url).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    // OpenAPI should be detected (GraphQL fails, OpenAPI comes before JSON-RPC)
    assert_eq!(adapter.protocol_type(), ProtocolType::OpenAPI);
}

#[test]
fn test_adapter_enum_protocol_type() {
    let mut server = mockito::Server::new();

    let introspection_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": [
                        {
                            "name": "hello",
                            "description": "Say hello",
                            "args": [],
                            "type": {
                                "kind": "SCALAR",
                                "name": "String",
                                "ofType": null
                            }
                        }
                    ]
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let _mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&introspection_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.get_adapter_for_url(&url).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();

    // Test that protocol_type() works correctly
    match adapter {
        AdapterEnum::GraphQL(_) => {
            // Correct adapter type
            assert_eq!(adapter.protocol_type(), ProtocolType::GraphQL);
        }
        _ => panic!("Expected GraphQL adapter"),
    }
}

#[test]
fn test_detection_options_schema_url_none() {
    let mut server = mockito::Server::new();

    let openapi_doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {
            "/users": {
                "get": {
                    "operationId": "getUsers",
                    "responses": {
                        "200": {
                            "description": "Success"
                        }
                    }
                }
            }
        }
    });

    let _mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_doc.to_string())
        .create();

    let url = format!("{}/openapi.json", server.url());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let detector = ProtocolDetector::new();
        let options = DetectionOptions { schema_url: None };
        detector.detect_adapter_with_options(&url, &options).await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::OpenAPI);
}

#[test]
fn test_detection_options_schema_url_some() {
    let mut server = mockito::Server::new();

    let openapi_doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {
            "/users": {
                "get": {
                    "operationId": "getUsers",
                    "responses": {
                        "200": {
                            "description": "Success"
                        }
                    }
                }
            }
        }
    });

    let _mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_doc.to_string())
        .create();

    let base_url = server.url();
    let schema_url = format!("{}/openapi.json", base_url);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let detector = ProtocolDetector::new();
        let options = DetectionOptions {
            schema_url: Some(schema_url),
        };
        detector
            .detect_adapter_with_options(&base_url, &options)
            .await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::OpenAPI);
}

#[test]
fn test_protocol_type_as_str() {
    assert_eq!(ProtocolType::OpenAPI.as_str(), "openapi");
    assert_eq!(ProtocolType::GRpc.as_str(), "grpc");
    assert_eq!(ProtocolType::JsonRpc.as_str(), "jsonrpc");
    assert_eq!(ProtocolType::Mcp.as_str(), "mcp");
    assert_eq!(ProtocolType::GraphQL.as_str(), "graphql");
}

#[test]
fn test_protocol_router_default() {
    let mut server = mockito::Server::new();

    let introspection_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": [
                        {
                            "name": "hello",
                            "description": "Say hello",
                            "args": [],
                            "type": {
                                "kind": "SCALAR",
                                "name": "String",
                                "ofType": null
                            }
                        }
                    ]
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let _mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&introspection_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::default();
        router.detect_protocol(&url).await
    });

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ProtocolType::GraphQL);
}

#[test]
fn test_protocol_detector_default() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let detector = ProtocolDetector::default();
        detector.detect_adapter("npx test-server").await
    });

    assert!(result.is_ok());
    let adapter = result.unwrap();
    assert_eq!(adapter.protocol_type(), ProtocolType::Mcp);
}

#[test]
fn test_multiple_detection_attempts() {
    let mut server = mockito::Server::new();

    let introspection_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": [
                        {
                            "name": "hello",
                            "description": "Say hello",
                            "args": [],
                            "type": {
                                "kind": "SCALAR",
                                "name": "String",
                                "ofType": null
                            }
                        }
                    ]
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let _mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&introspection_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();

        // First detection
        let protocol1 = router.detect_protocol(&url).await;

        // Second detection (should return same result)
        let protocol2 = router.detect_protocol(&url).await;

        (protocol1, protocol2)
    });

    assert!(result.0.is_ok());
    assert!(result.1.is_ok());
    assert_eq!(result.0.unwrap(), result.1.unwrap());
}

#[test]
fn test_empty_url() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        router.detect_protocol("").await
    });

    assert!(result.is_err(), "Empty URL should fail detection");
}

#[test]
fn test_invalid_url_format() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let invalid_urls = vec!["not-a-url", "://invalid", "http://", "://example.com"];

    for url in invalid_urls {
        let result = rt.block_on(async {
            let router = ProtocolRouter::new();
            router.detect_protocol(url).await
        });

        // Should fail for invalid URLs
        assert!(
            result.is_err(),
            "Invalid URL '{}' should fail detection",
            url
        );
    }
}

#[test]
fn test_protocol_timeout_handling() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let router = ProtocolRouter::new();
        // Use a URL that will likely timeout (non-routable IP)
        router.detect_protocol("http://192.0.2.1:9999").await
    });

    // Should fail with an error (could be timeout or connection refused)
    assert!(result.is_err(), "Unreachable URL should fail");
}

#[test]
fn test_graphql_adapter_can_handle() {
    let mut server = mockito::Server::new();

    let introspection_response = serde_json::json!({
        "data": {
            "__schema": {
                "queryType": {
                    "name": "Query",
                    "fields": [
                        {
                            "name": "hello",
                            "description": "Say hello",
                            "args": [],
                            "type": {
                                "kind": "SCALAR",
                                "name": "String",
                                "ofType": null
                            }
                        }
                    ]
                },
                "mutationType": null,
                "subscriptionType": null
            }
        }
    });

    let _mock = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&introspection_response.to_string())
        .create();

    let url = server.url();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let adapter = GraphQLAdapter::new();
        adapter.can_handle(&url).await
    });

    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "GraphQL adapter should handle GraphQL endpoint"
    );
}

#[test]
fn test_openapi_adapter_can_handle() {
    let mut server = mockito::Server::new();

    let openapi_doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": "Test API",
            "version": "1.0.0"
        },
        "paths": {
            "/users": {
                "get": {
                    "operationId": "getUsers",
                    "responses": {
                        "200": {
                            "description": "Success"
                        }
                    }
                }
            }
        }
    });

    let _mock = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&openapi_doc.to_string())
        .create();

    let url = format!("{}/openapi.json", server.url());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let adapter = OpenAPIAdapter::new();
        adapter.can_handle(&url).await
    });

    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "OpenAPI adapter should handle OpenAPI endpoint"
    );
}

#[test]
fn test_jsonrpc_adapter_can_handle() {
    let mut server = mockito::Server::new();

    let openrpc_doc = serde_json::json!({
        "openrpc": "1.2.6",
        "info": {
            "title": "Test JSON-RPC API",
            "version": "1.0.0"
        },
        "methods": [
            {
                "name": "test_method",
                "description": "A test method",
                "params": [],
                "result": {
                    "name": "result",
                    "schema": {
                        "type": "string"
                    }
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

    let url = format!("{}/openrpc.json", server.url());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        let adapter = JsonRpcAdapter::new();
        adapter.can_handle(&url).await
    });

    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "JSON-RPC adapter should handle JSON-RPC endpoint"
    );
}

#[test]
fn test_mcp_adapter_is_stdio_command() {
    // Test various stdio command patterns
    assert!(McpAdapter::is_stdio_command(
        "npx @modelcontextprotocol/server"
    ));
    assert!(McpAdapter::is_stdio_command("node server.js"));
    assert!(McpAdapter::is_stdio_command("python3 server.py"));
    assert!(McpAdapter::is_stdio_command("./my-server"));
    assert!(McpAdapter::is_stdio_command("/usr/local/bin/server"));
    assert!(McpAdapter::is_stdio_command("mcp://custom-server"));

    // HTTP URLs should NOT be stdio commands
    assert!(!McpAdapter::is_stdio_command("http://example.com"));
    assert!(!McpAdapter::is_stdio_command("https://example.com"));
}

#[test]
fn test_mcp_adapter_parse_stdio_command() {
    // Test parsing simple command
    let (cmd, args) = McpAdapter::parse_stdio_command("npx @modelcontextprotocol/server")
        .expect("Should parse command");
    assert_eq!(cmd, "npx");
    assert_eq!(args, vec!["@modelcontextprotocol/server"]);

    // Test parsing command with multiple args
    let (cmd, args) = McpAdapter::parse_stdio_command("node server.js --arg1 --arg2")
        .expect("Should parse command with args");
    assert_eq!(cmd, "node");
    assert_eq!(args, vec!["server.js", "--arg1", "--arg2"]);
}
