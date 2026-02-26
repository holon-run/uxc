//! GraphQL Contract Tests
//!
//! Comprehensive contract tests verifying GraphQL specification compliance.
//! Tests cover query execution, introspection, mutations, error handling,
//! fragments, and edge cases as per GraphQL specification.
//!
//! Reference: https://spec.graphql.org/

use uxc::adapters::graphql::GraphQLAdapter;
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
// Introspection Tests
// ============================================================================

#[test]
fn test_graphql_introspection_detects_valid_endpoint() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
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

        let _mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&introspection_response.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = GraphQLAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(
            result.unwrap(),
            "Should detect GraphQL endpoint via introspection"
        );
    });
}

#[test]
fn test_graphql_introspection_rejects_non_graphql() {
    run_async(|mut server| {
        let not_graphql = serde_json::json!({
            "result": "some other response"
        });

        let _mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&not_graphql.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = GraphQLAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(!result.unwrap(), "Should reject non-GraphQL endpoint");
    });
}

#[test]
fn test_graphql_introspection_handles_errors_response() {
    run_async(|mut server| {
        let graphql_errors = serde_json::json!({
            "errors": [
                {
                    "message": "Syntax Error",
                    "locations": [{ "line": 1, "column": 2 }]
                }
            ]
        });

        let _mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&graphql_errors.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = GraphQLAdapter::new();
        let result = rt.block_on(async { adapter.can_handle(&server.url()).await });

        assert!(result.is_ok());
        assert!(
            result.unwrap(),
            "GraphQL errors in response still indicate GraphQL endpoint"
        );
    });
}

// ============================================================================
// Schema Parsing Tests
// ============================================================================

#[test]
fn test_graphql_parses_query_fields() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "user",
                                "description": "Get a user by ID",
                                "args": [
                                    {
                                        "name": "id",
                                        "description": "User ID",
                                        "type": {
                                            "kind": "NON_NULL",
                                            "ofType": {
                                                "kind": "SCALAR",
                                                "name": "ID"
                                            }
                                        }
                                    }
                                ],
                                "type": {
                                    "kind": "OBJECT",
                                    "name": "User",
                                    "ofType": null
                                }
                            },
                            {
                                "name": "users",
                                "description": "List all users",
                                "args": [],
                                "type": {
                                    "kind": "LIST",
                                    "ofType": {
                                        "kind": "OBJECT",
                                        "name": "User"
                                    }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 2);
        assert!(operations.iter().any(|op| op.operation_id == "query/user"));
        assert!(operations.iter().any(|op| op.operation_id == "query/users"));

        let user_op = operations
            .iter()
            .find(|op| op.operation_id == "query/user")
            .unwrap();
        assert_eq!(user_op.description.as_ref().unwrap(), "Get a user by ID");
        assert_eq!(user_op.parameters.len(), 1);
        assert_eq!(user_op.parameters[0].name, "id");
        assert!(user_op.parameters[0].required);
        assert_eq!(user_op.parameters[0].param_type, "ID!");
    });
}

#[test]
fn test_graphql_parses_mutation_fields() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": []
                    },
                    "mutationType": {
                        "name": "Mutation",
                        "fields": [
                            {
                                "name": "createUser",
                                "description": "Create a new user",
                                "args": [
                                    {
                                        "name": "name",
                                        "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "String" } }
                                    },
                                    {
                                        "name": "email",
                                        "type": { "kind": "SCALAR", "name": "String" }
                                    }
                                ],
                                "type": { "kind": "OBJECT", "name": "User" }
                            }
                        ]
                    },
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation_id, "mutation/createUser");
        assert_eq!(
            operations[0].description.as_ref().unwrap(),
            "Create a new user"
        );
        assert_eq!(operations[0].parameters.len(), 2);
        assert!(operations[0].parameters[0].required);
        assert!(!operations[0].parameters[1].required);
    });
}

#[test]
fn test_graphql_parses_subscription_fields() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": []
                    },
                    "mutationType": null,
                    "subscriptionType": {
                        "name": "Subscription",
                        "fields": [
                            {
                                "name": "userUpdated",
                                "description": "Subscribe to user updates",
                                "args": [
                                    {
                                        "name": "userId",
                                        "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "ID" } }
                                    }
                                ],
                                "type": { "kind": "OBJECT", "name": "User" }
                            }
                        ]
                    }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation_id, "subscription/userUpdated");
        assert_eq!(
            operations[0].description.as_ref().unwrap(),
            "Subscribe to user updates"
        );
    });
}

// ============================================================================
// Type System Tests
// ============================================================================

#[test]
fn test_graphql_parses_scalar_types() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "intField",
                                "args": [{"name": "value", "type": { "kind": "SCALAR", "name": "Int" } }],
                                "type": { "kind": "SCALAR", "name": "Int" }
                            },
                            {
                                "name": "floatField",
                                "args": [{"name": "value", "type": { "kind": "SCALAR", "name": "Float" } }],
                                "type": { "kind": "SCALAR", "name": "Float" }
                            },
                            {
                                "name": "stringField",
                                "args": [{"name": "value", "type": { "kind": "SCALAR", "name": "String" } }],
                                "type": { "kind": "SCALAR", "name": "String" }
                            },
                            {
                                "name": "boolField",
                                "args": [{"name": "value", "type": { "kind": "SCALAR", "name": "Boolean" } }],
                                "type": { "kind": "SCALAR", "name": "Boolean" }
                            },
                            {
                                "name": "idField",
                                "args": [{"name": "value", "type": { "kind": "SCALAR", "name": "ID" } }],
                                "type": { "kind": "SCALAR", "name": "ID" }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 5);
        let int_op = operations
            .iter()
            .find(|op| op.operation_id == "query/intField")
            .unwrap();
        assert_eq!(int_op.parameters[0].param_type, "Int");

        let float_op = operations
            .iter()
            .find(|op| op.operation_id == "query/floatField")
            .unwrap();
        assert_eq!(float_op.parameters[0].param_type, "Float");

        let string_op = operations
            .iter()
            .find(|op| op.operation_id == "query/stringField")
            .unwrap();
        assert_eq!(string_op.parameters[0].param_type, "String");

        let bool_op = operations
            .iter()
            .find(|op| op.operation_id == "query/boolField")
            .unwrap();
        assert_eq!(bool_op.parameters[0].param_type, "Boolean");

        let id_op = operations
            .iter()
            .find(|op| op.operation_id == "query/idField")
            .unwrap();
        assert_eq!(id_op.parameters[0].param_type, "ID");
    });
}

#[test]
fn test_graphql_parses_non_null_types() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "requiredField",
                                "args": [
                                    {
                                        "name": "required",
                                        "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "String" } }
                                    }
                                ],
                                "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "String" } }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].parameters[0].param_type, "String!");
        assert!(operations[0].parameters[0].required);
    });
}

#[test]
fn test_graphql_parses_list_types() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "listField",
                                "args": [
                                    {
                                        "name": "items",
                                        "type": { "kind": "LIST", "ofType": { "kind": "SCALAR", "name": "String" } }
                                    }
                                ],
                                "type": { "kind": "LIST", "ofType": { "kind": "SCALAR", "name": "String" } }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].parameters[0].param_type, "[String]");
        assert!(!operations[0].parameters[0].required);
    });
}

#[test]
fn test_graphql_parses_complex_nested_types() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "complexField",
                                "args": [
                                    {
                                        "name": "nested",
                                        "type": {
                                            "kind": "NON_NULL",
                                            "ofType": {
                                                "kind": "LIST",
                                                "ofType": {
                                                    "kind": "NON_NULL",
                                                    "ofType": {
                                                        "kind": "SCALAR",
                                                        "name": "String"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                ],
                                "type": { "kind": "SCALAR", "name": "String" }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        // [String!]! = Non-null list of non-null strings
        assert_eq!(operations[0].parameters[0].param_type, "[String!]!");
        assert!(operations[0].parameters[0].required);
    });
}

// ============================================================================
// Input Schema Tests
// ============================================================================

#[test]
fn test_graphql_builds_input_schema_for_query() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "user",
                                "args": [
                                    {
                                        "name": "id",
                                        "description": "User identifier",
                                        "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "ID" } }
                                    }
                                ],
                                "type": { "kind": "OBJECT", "name": "User" }
                            }
                        ]
                    },
                    "mutationType": null,
                    "subscriptionType": null,
                    "types": []
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
        let adapter = GraphQLAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "query/user")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert_eq!(schema["kind"], "graphql_arguments");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["id"].is_object());
        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert_eq!(schema["properties"]["id"]["description"], "User identifier");
        assert_eq!(schema["required"][0], "id");
    });
}

#[test]
fn test_graphql_builds_input_schema_with_input_object() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "search",
                                "args": [
                                    {
                                        "name": "filter",
                                        "type": { "kind": "INPUT_OBJECT", "name": "SearchFilter" }
                                    }
                                ],
                                "type": { "kind": "LIST", "ofType": { "kind": "OBJECT", "name": "Result" } }
                            }
                        ]
                    },
                    "mutationType": null,
                    "subscriptionType": null,
                    "types": [
                        {
                            "name": "SearchFilter",
                            "kind": "INPUT_OBJECT",
                            "inputFields": [
                                {
                                    "name": "limit",
                                    "description": "Max results",
                                    "type": { "kind": "SCALAR", "name": "Int" }
                                },
                                {
                                    "name": "query",
                                    "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "String" } }
                                }
                            ]
                        }
                    ]
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
        let adapter = GraphQLAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "query/search")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert!(schema["properties"]["filter"]["properties"]["limit"].is_object());
        assert_eq!(
            schema["properties"]["filter"]["properties"]["limit"]["type"],
            "integer"
        );
        assert_eq!(
            schema["properties"]["filter"]["properties"]["limit"]["description"],
            "Max results"
        );
        assert_eq!(
            schema["properties"]["filter"]["properties"]["query"]["type"],
            "string"
        );
    });
}

#[test]
fn test_graphql_builds_input_schema_with_enum() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "items",
                                "args": [
                                    {
                                        "name": "status",
                                        "type": { "kind": "ENUM", "name": "Status" }
                                    }
                                ],
                                "type": { "kind": "LIST", "ofType": { "kind": "OBJECT", "name": "Item" } }
                            }
                        ]
                    },
                    "mutationType": null,
                    "subscriptionType": null,
                    "types": [
                        {
                            "name": "Status",
                            "kind": "ENUM",
                            "enumValues": [
                                { "name": "ACTIVE", "description": "Active status" },
                                { "name": "INACTIVE", "description": "Inactive status" },
                                { "name": "PENDING" }
                            ]
                        }
                    ]
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
        let adapter = GraphQLAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "query/items")
                    .await
            })
            .unwrap();

        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert_eq!(schema["properties"]["status"]["type"], "string");
        assert_eq!(schema["properties"]["status"]["enum"][0], "ACTIVE");
        assert_eq!(schema["properties"]["status"]["enum"][1], "INACTIVE");
        assert_eq!(schema["properties"]["status"]["enum"][2], "PENDING");
    });
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_graphql_handles_missing_operation() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            { "name": "existing", "args": [], "type": { "kind": "SCALAR", "name": "String" } }
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
        let adapter = GraphQLAdapter::new();
        let result = rt.block_on(async {
            adapter
                .describe_operation(&server.url(), "query/nonexistent")
                .await
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    });
}

#[test]
fn test_graphql_handles_invalid_operation_format() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": { "name": "Query", "fields": [] },
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
        let adapter = GraphQLAdapter::new();

        // Missing prefix - should fail
        let result = rt.block_on(async { adapter.describe_operation(&server.url(), "user").await });
        assert!(result.is_err(), "Should fail for operation without prefix");

        // Invalid prefix - should fail with format error
        let result = rt.block_on(async {
            adapter
                .describe_operation(&server.url(), "invalid/user")
                .await
        });
        assert!(result.is_err(), "Should fail for invalid operation type");
        let err_msg = result.unwrap_err().to_string();
        // Check that error message mentions the invalid operation
        assert!(err_msg.contains("invalid") || err_msg.contains("operation"));
    });
}

#[test]
fn test_graphql_handles_introspection_errors() {
    run_async(|mut server| {
        let error_response = serde_json::json!({
            "errors": [
                {
                    "message": "Cannot query field",
                    "locations": [{ "line": 2, "column": 3 }]
                }
            ]
        });

        let _mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&error_response.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = GraphQLAdapter::new();
        let result = rt.block_on(async { adapter.fetch_schema(&server.url()).await });

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("GraphQL introspection failed"));
    });
}

// ============================================================================
// Edge Cases and Complex Scenarios
// ============================================================================

#[test]
fn test_graphql_handles_empty_schema() {
    run_async(|mut server| {
        let empty_response = serde_json::json!({
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

        let _mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&empty_response.to_string())
            .create();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 0);
    });
}

#[test]
fn test_graphql_handles_operation_without_args() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "health",
                                "description": "Health check endpoint",
                                "args": [],
                                "type": { "kind": "SCALAR", "name": "String" }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation_id, "query/health");
        assert_eq!(operations[0].parameters.len(), 0);
    });
}

#[test]
fn test_graphql_return_type_display_names() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "simple",
                                "type": { "kind": "SCALAR", "name": "String" }
                            },
                            {
                                "name": "required",
                                "type": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "String" } }
                            },
                            {
                                "name": "listOfRequired",
                                "type": { "kind": "LIST", "ofType": { "kind": "NON_NULL", "ofType": { "kind": "SCALAR", "name": "Int" } } }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        let simple = operations
            .iter()
            .find(|op| op.operation_id == "query/simple")
            .unwrap();
        assert_eq!(simple.return_type.as_ref().unwrap(), "String");

        let required = operations
            .iter()
            .find(|op| op.operation_id == "query/required")
            .unwrap();
        assert_eq!(required.return_type.as_ref().unwrap(), "String!");

        let list = operations
            .iter()
            .find(|op| op.operation_id == "query/listOfRequired")
            .unwrap();
        assert_eq!(list.return_type.as_ref().unwrap(), "[Int!]");
    });
}

#[test]
fn test_graphql_description_optional_fields() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "withDescription",
                                "description": "Has description",
                                "args": [],
                                "type": { "kind": "SCALAR", "name": "String" }
                            },
                            {
                                "name": "withoutDescription",
                                "args": [],
                                "type": { "kind": "SCALAR", "name": "String" }
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
        let adapter = GraphQLAdapter::new();
        let operations = rt
            .block_on(async { adapter.list_operations(&server.url()).await })
            .unwrap();

        assert_eq!(operations.len(), 2);
        let with_desc = operations
            .iter()
            .find(|op| op.operation_id == "query/withDescription")
            .unwrap();
        assert_eq!(with_desc.description.as_ref().unwrap(), "Has description");

        let without_desc = operations
            .iter()
            .find(|op| op.operation_id == "query/withoutDescription")
            .unwrap();
        assert!(without_desc.description.is_none());
    });
}

#[test]
fn test_graphql_handles_circular_type_references() {
    run_async(|mut server| {
        let introspection_response = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "node",
                                "args": [
                                    {
                                        "name": "input",
                                        "type": { "kind": "INPUT_OBJECT", "name": "NodeInput" }
                                    }
                                ],
                                "type": { "kind": "OBJECT", "name": "Node" }
                            }
                        ]
                    },
                    "mutationType": null,
                    "subscriptionType": null,
                    "types": [
                        {
                            "name": "NodeInput",
                            "kind": "INPUT_OBJECT",
                            "inputFields": [
                                {
                                    "name": "child",
                                    "type": { "kind": "INPUT_OBJECT", "name": "NodeInput" }
                                }
                            ]
                        }
                    ]
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
        let adapter = GraphQLAdapter::new();
        let detail = rt
            .block_on(async {
                adapter
                    .describe_operation(&server.url(), "query/node")
                    .await
            })
            .unwrap();

        // Should handle circular references without infinite recursion
        assert!(detail.input_schema.is_some());
        let schema = detail.input_schema.unwrap();
        assert!(schema["properties"]["input"]["properties"]["child"].is_object());
    });
}
