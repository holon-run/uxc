//! OpenAPI Contract Tests
//!
//! Comprehensive contract tests verifying OpenAPI 3.0/3.1 specification compliance.
//! Tests cover schema parsing, parameter validation, response handling, authentication,
//! and edge cases as per OpenAPI specification.
//!
//! Reference: https://spec.openapis.org/oas/v3.0.0

use uxc::adapters::openapi::OpenAPIAdapter;
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
// Schema Parsing and Discovery Tests
// ============================================================================

#[test]
fn test_openapi_discovery_finds_openapi_v3() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": "Sample API",
                "version": "1.0.0"
            },
            "paths": {
                "/users": {
                    "get": {
                        "operationId": "getUsers",
                        "responses": {
                            "200": { "description": "Success" }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(result.unwrap(), "Should detect OpenAPI v3 document");
    });
}

#[test]
fn test_openapi_discovery_finds_swagger_v2() {
    run_async(|mut server| {
        let swagger_doc = serde_json::json!({
            "swagger": "2.0",
            "info": {
                "title": "Sample API",
                "version": "1.0.0"
            },
            "paths": {
                "/users": {
                    "get": {
                        "operationId": "getUsers",
                        "responses": {
                            "200": { "description": "Success" }
                        }
                    }
                }
            }
        });

        let _mock = server
            .mock("GET", "/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&swagger_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(result.unwrap(), "Should detect Swagger v2 document");
    });
}

#[test]
fn test_openapi_rejects_non_openapi_document() {
    run_async(|mut server| {
        let not_openapi = serde_json::json!({
            "api": "something",
            "data": "test"
        });

        let _mock = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&not_openapi.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(!result.unwrap(), "Should reject non-OpenAPI document");
    });
}

#[test]
fn test_openapi_tries_multiple_schema_endpoints() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {}
        });

        // First endpoint fails, second succeeds
        let _mock1 = server
            .mock("GET", "/swagger.json")
            .with_status(404)
            .create();

        let _mock2 = server
            .mock("GET", "/api-docs")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openapi_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(
            result.unwrap(),
            "Should try multiple endpoints and find schema"
        );
    });
}

// ============================================================================
// Operation Listing Tests
// ============================================================================

#[test]
fn test_openapi_lists_all_http_methods() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/resource": {
                    "get": {
                        "operationId": "getResource",
                        "responses": { "200": { "description": "OK" } }
                    },
                    "post": {
                        "operationId": "createResource",
                        "responses": { "201": { "description": "Created" } }
                    },
                    "put": {
                        "operationId": "updateResource",
                        "responses": { "200": { "description": "OK" } }
                    },
                    "patch": {
                        "operationId": "patchResource",
                        "responses": { "200": { "description": "OK" } }
                    },
                    "delete": {
                        "operationId": "deleteResource",
                        "responses": { "204": { "description": "No Content" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 5);
        assert!(operations
            .iter()
            .any(|op| op.operation_id == "get:/resource"));
        assert!(operations
            .iter()
            .any(|op| op.operation_id == "post:/resource"));
        assert!(operations
            .iter()
            .any(|op| op.operation_id == "put:/resource"));
        assert!(operations
            .iter()
            .any(|op| op.operation_id == "patch:/resource"));
        assert!(operations
            .iter()
            .any(|op| op.operation_id == "delete:/resource"));
    });
}

#[test]
fn test_openapi_operation_display_name_format() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users/{id}": {
                    "get": {
                        "summary": "Get user by ID",
                        "operationId": "getUserById",
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let op = &operations[0];
        assert_eq!(op.display_name, "GET /users/{id}");
        assert_eq!(op.operation_id, "get:/users/{id}");
    });
}

// ============================================================================
// Parameter Parsing Tests
// ============================================================================

#[test]
fn test_openapi_parses_path_parameters() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users/{userId}": {
                    "parameters": [
                        {
                            "name": "userId",
                            "in": "path",
                            "required": true,
                            "schema": { "type": "integer" },
                            "description": "User ID"
                        }
                    ],
                    "get": {
                        "operationId": "getUser",
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let params = &operations[0].parameters;
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "userId");
        assert_eq!(params[0].param_type, "integer");
        assert!(params[0].required);
        assert_eq!(params[0].description.as_ref().unwrap(), "User ID");
    });
}

#[test]
fn test_openapi_parses_query_parameters() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users": {
                    "get": {
                        "operationId": "listUsers",
                        "parameters": [
                            {
                                "name": "limit",
                                "in": "query",
                                "schema": { "type": "integer" },
                                "description": "Max results"
                            },
                            {
                                "name": "offset",
                                "in": "query",
                                "schema": { "type": "integer" }
                            }
                        ],
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let params = &operations[0].parameters;
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "limit");
        assert_eq!(params[1].name, "offset");
    });
}

#[test]
fn test_openapi_parses_header_parameters() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/data": {
                    "get": {
                        "operationId": "getData",
                        "parameters": [
                            {
                                "name": "Authorization",
                                "in": "header",
                                "required": true,
                                "schema": { "type": "string" }
                            },
                            {
                                "name": "X-Request-ID",
                                "in": "header",
                                "schema": { "type": "string" }
                            }
                        ],
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let params = &operations[0].parameters;
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "Authorization");
        assert!(params[0].required);
        assert_eq!(params[1].name, "X-Request-ID");
    });
}

#[test]
fn test_openapi_merges_path_and_operation_parameters() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/resource/{id}": {
                    "parameters": [
                        {
                            "name": "id",
                            "in": "path",
                            "required": true,
                            "schema": { "type": "string" }
                        }
                    ],
                    "get": {
                        "operationId": "getResource",
                        "parameters": [
                            {
                                "name": "verbose",
                                "in": "query",
                                "schema": { "type": "boolean" }
                            }
                        ],
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        let params = &operations[0].parameters;
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "id"));
        assert!(params.iter().any(|p| p.name == "verbose"));
    });
}

// ============================================================================
// Request Body Tests
// ============================================================================

#[test]
fn test_openapi_parses_request_body_schema() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users": {
                    "post": {
                        "operationId": "createUser",
                        "requestBody": {
                            "required": true,
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "required": ["name", "email"],
                                        "properties": {
                                            "name": { "type": "string" },
                                            "email": { "type": "string" }
                                        }
                                    }
                                }
                            }
                        },
                        "responses": { "201": { "description": "Created" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "post:/users")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert_eq!(schema["kind"], "openapi_request_body");
        assert_eq!(schema["required"], true);
        assert!(schema["content"]["application/json"]["schema"]["properties"]["name"].is_object());
    });
}

#[test]
fn test_openapi_request_body_with_multiple_content_types() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/upload": {
                    "post": {
                        "operationId": "uploadFile",
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": { "type": "object" }
                                },
                                "multipart/form-data": {
                                    "schema": { "type": "object", "format": "binary" }
                                }
                            }
                        },
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "post:/upload")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert!(schema["content"]["application/json"].is_object());
        assert!(schema["content"]["multipart/form-data"].is_object());
    });
}

// ============================================================================
// Schema Reference Tests
// ============================================================================

#[test]
fn test_openapi_resolves_local_refs() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users": {
                    "post": {
                        "operationId": "createUser",
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/User" }
                                }
                            }
                        },
                        "responses": { "201": { "description": "Created" } }
                    }
                }
            },
            "components": {
                "schemas": {
                    "User": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "integer" },
                            "name": { "type": "string" }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "post:/users")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        // Reference should be resolved and expanded
        let expanded = &schema["content"]["application/json"]["schema"];
        assert_eq!(expanded["type"], "object");
        assert!(expanded["properties"]["id"].is_object());
        assert!(expanded["properties"]["name"].is_object());
    });
}

#[test]
fn test_openapi_handles_nested_refs() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users": {
                    "post": {
                        "operationId": "createUser",
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/UserRequest" }
                                }
                            }
                        },
                        "responses": { "201": { "description": "Created" } }
                    }
                }
            },
            "components": {
                "schemas": {
                    "UserRequest": {
                        "type": "object",
                        "properties": {
                            "user": { "$ref": "#/components/schemas/User" }
                        }
                    },
                    "User": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "integer" }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "post:/users")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        let expanded = &schema["content"]["application/json"]["schema"];
        // Nested refs should be expanded
        assert_eq!(expanded["properties"]["user"]["type"], "object");
        assert_eq!(
            expanded["properties"]["user"]["properties"]["id"]["type"],
            "integer"
        );
    });
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_openapi_handles_missing_operation() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users": {
                    "get": {
                        "operationId": "getUsers",
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let result = rt.block_on(async {
            adapter
                .describe_operation(&server.url(), "get:/nonexistent")
                .await
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    });
}

#[test]
fn test_openapi_handles_invalid_operation_id_format() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {}
        });

        let _mock = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openapi_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();

        // Invalid: missing path
        let result = rt.block_on(async { adapter.describe_operation(&server.url(), "get").await });
        assert!(result.is_err());

        // Invalid: wrong format
        let result = rt.block_on(async {
            adapter
                .describe_operation(&server.url(), "GET /users")
                .await
        });
        assert!(result.is_err());
    });
}

#[test]
fn test_openapi_handles_unsupported_http_method() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {}
        });

        let _mock = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openapi_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let result = rt.block_on(async {
            adapter
                .describe_operation(&server.url(), "invalid:/path")
                .await
        });

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported HTTP method"));
    });
}

// ============================================================================
// Edge Cases and Complex Scenarios
// ============================================================================

#[test]
fn test_openapi_handles_empty_paths() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {}
        });

        let _mock = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&openapi_doc.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 0);
    });
}

#[test]
fn test_openapi_handles_path_with_trailing_slash() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users/": {
                    "get": {
                        "operationId": "getUsers",
                        "responses": { "200": { "description": "OK" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation_id, "get:/users/");
    });
}

#[test]
fn test_openapi_descriptions_fallback_to_summary() {
    run_async(|mut server| {
        let openapi_doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "API", "version": "1.0" },
            "paths": {
                "/users": {
                    "get": {
                        "summary": "List all users",
                        "operationId": "getUsers",
                        "responses": { "200": { "description": "OK" } }
                    },
                    "post": {
                        "description": "Create a new user",
                        "operationId": "createUser",
                        "responses": { "201": { "description": "Created" } }
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

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = OpenAPIAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 2);
        let get_op = operations
            .iter()
            .find(|op| op.operation_id == "get:/users")
            .unwrap();
        assert_eq!(get_op.description.as_ref().unwrap(), "List all users");

        let post_op = operations
            .iter()
            .find(|op| op.operation_id == "post:/users")
            .unwrap();
        assert_eq!(post_op.description.as_ref().unwrap(), "Create a new user");
    });
}
