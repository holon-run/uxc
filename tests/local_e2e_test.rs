//! Local E2E tests using test servers
//!
//! These tests verify that uxc can correctly interact with local controllable
//! test servers for each protocol.

mod common;

use common::{run_uxc, start_test_server, test_server_binary};
use std::process::Command;

fn grpcurl_available() -> bool {
    Command::new("grpcurl")
        .arg("-help")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false)
}

#[test]
fn test_openapi_list_operations() {
    let _server = start_test_server("openapi", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "list"]);

    assert!(
        result.is_ok(),
        "Failed to list OpenAPI operations: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "openapi");
    assert!(json["data"]["operations"].as_array().unwrap().len() > 0);

    // Check for expected operations
    let ops: Vec<&str> = json["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["operation_id"].as_str())
        .collect();

    assert!(
        ops.contains(&"get:/health"),
        "Expected get:/health operation"
    );
    assert!(ops.contains(&"get:/users"), "Expected get:/users operation");
    assert!(
        ops.contains(&"get:/users/{id}"),
        "Expected get:/users/{{id}} operation"
    );
}

#[test]
fn test_openapi_call_operation() {
    let _server = start_test_server("openapi", "ok");

    // Call the health check endpoint
    let result = run_uxc(&[&format!("http://{}/", _server.addr), "get:/health"]);

    assert!(
        result.is_ok(),
        "Failed to call OpenAPI operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "openapi");
    assert_eq!(json["data"]["status"], "ok");
}

#[test]
fn test_openapi_auth_required() {
    let _server = start_test_server("openapi", "auth_required");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "get:/health"]);

    assert!(result.is_err(), "Expected auth error, but got success");

    let err = result.unwrap_err();
    assert!(
        err.contains("401") || err.contains("Unauthorized"),
        "Expected 401 error, got: {}",
        err
    );
}

#[test]
fn test_graphql_list_operations() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "list"]);

    assert!(
        result.is_ok(),
        "Failed to list GraphQL operations: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert!(
        json["data"]["operations"].is_array(),
        "Expected operations array in GraphQL list output"
    );
}

#[test]
fn test_graphql_call_query() {
    let _server = start_test_server("graphql", "ok");

    // Call the health query
    let result = run_uxc(&[&format!("http://{}/", _server.addr), "query/health"]);

    assert!(result.is_ok(), "Failed to call GraphQL query: {:?}", result);

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert_eq!(json["data"]["health"]["status"], "ok");
}

#[test]
fn test_graphql_call_with_args() {
    let _server = start_test_server("graphql", "ok");

    // Call the user query with an ID argument
    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "query/user",
        "--json",
        r#"{"id":"1"}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call GraphQL query with args: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert_eq!(json["data"]["user"]["id"], "1");
    assert_eq!(json["data"]["user"]["name"], "Alice");
}

#[test]
fn test_graphql_auth_required() {
    let _server = start_test_server("graphql", "auth_required");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "query/health"]);

    assert!(result.is_err(), "Expected auth error, but got success");

    let err = result.unwrap_err();
    assert!(
        err.contains("401") || err.contains("Unauthorized"),
        "Expected 401 error, got: {}",
        err
    );
}

#[test]
fn test_jsonrpc_list_operations() {
    let _server = start_test_server("jsonrpc", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "list"]);

    assert!(
        result.is_ok(),
        "Failed to list JSON-RPC operations: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert!(json["data"]["operations"].as_array().unwrap().len() > 0);

    // Check for expected operations
    let ops: Vec<&str> = json["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["operation_id"].as_str())
        .collect();

    assert!(ops.contains(&"health"), "Expected health operation");
    assert!(ops.contains(&"list_users"), "Expected list_users operation");
    assert!(ops.contains(&"get_user"), "Expected get_user operation");
}

#[test]
fn test_jsonrpc_call_method() {
    let _server = start_test_server("jsonrpc", "ok");

    // Call the health method
    let result = run_uxc(&[&format!("http://{}/", _server.addr), "health"]);

    assert!(
        result.is_ok(),
        "Failed to call JSON-RPC method: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["data"]["status"], "ok");
}

#[test]
fn test_jsonrpc_call_with_args() {
    let _server = start_test_server("jsonrpc", "ok");

    // Call the get_user method with an ID argument
    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "get_user",
        "--json",
        r#"{"id":1}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call JSON-RPC method with args: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["data"]["id"], 1);
    assert_eq!(json["data"]["name"], "Alice");
}

#[test]
fn test_jsonrpc_auth_required() {
    let _server = start_test_server("jsonrpc", "auth_required");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "health"]);

    assert!(result.is_err(), "Expected auth error, but got success");

    let err = result.unwrap_err();
    assert!(
        err.contains("401") || err.contains("Unauthorized"),
        "Expected 401 error, got: {}",
        err
    );
}

#[test]
fn test_openapi_malformed_response() {
    let _server = start_test_server("openapi", "malformed");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "get:/health"]);
    assert!(
        result.is_err(),
        "Expected malformed OpenAPI response to fail, got success"
    );
}

#[test]
fn test_graphql_malformed_response() {
    let _server = start_test_server("graphql", "malformed");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "query/health"]);
    assert!(
        result.is_ok(),
        "Expected GraphQL malformed scenario to return response envelope"
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert!(json["data"]["invalid"].is_string());
}

#[test]
fn test_jsonrpc_malformed_response() {
    let _server = start_test_server("jsonrpc", "malformed");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "health"]);
    assert!(
        result.is_ok(),
        "Expected JSON-RPC malformed scenario to return response envelope"
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["invalid"], "malformed");
}

#[test]
fn test_grpc_list_operations() {
    let _server = start_test_server("grpc", "ok");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "list"]);

    assert!(
        result.is_ok(),
        "Failed to list gRPC operations: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "grpc");

    let ops: Vec<&str> = json["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["operation_id"].as_str())
        .collect();

    assert!(
        ops.contains(&"addsvc.Add/Sum"),
        "Expected addsvc.Add/Sum operation"
    );
}

#[test]
fn test_grpc_call_method() {
    if !grpcurl_available() {
        eprintln!("Skipping gRPC call test because grpcurl is not installed");
        return;
    }

    let _server = start_test_server("grpc", "ok");

    let result = run_uxc(&[
        &format!("http://{}", _server.addr),
        "addsvc.Add/Sum",
        "--json",
        r#"{"a":1,"b":2}"#,
    ]);

    assert!(result.is_ok(), "Failed to call gRPC method: {:?}", result);

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "grpc");
    assert!(json["data"]["v"] == 3 || json["data"]["v"] == "3");
}

#[test]
fn test_grpc_auth_required() {
    let _server = start_test_server("grpc", "auth_required");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "list"]);

    assert!(
        result.is_err(),
        "Expected gRPC auth/reflection error, got success"
    );
}

#[test]
fn test_mcp_http_list_operations() {
    let _server = start_test_server("mcp-http", "ok");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "list"]);

    assert!(
        result.is_ok(),
        "Failed to list MCP HTTP tools: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");

    let ops: Vec<&str> = json["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["operation_id"].as_str())
        .collect();

    assert!(ops.contains(&"echo"), "Expected echo MCP tool");
}

#[test]
fn test_mcp_http_call_tool() {
    let _server = start_test_server("mcp-http", "ok");

    let result = run_uxc(&[
        &format!("http://{}", _server.addr),
        "echo",
        "--json",
        r#"{"message":"hello mcp"}"#,
    ]);

    assert!(result.is_ok(), "Failed to call MCP HTTP tool: {:?}", result);

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "hello mcp");
}

#[test]
fn test_mcp_http_auth_required() {
    let _server = start_test_server("mcp-http", "auth_required");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "list"]);

    assert!(result.is_err(), "Expected MCP HTTP auth error, got success");
}

#[test]
fn test_mcp_stdio_list_operations() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let result = run_uxc(&[&endpoint, "list"]);

    assert!(
        result.is_ok(),
        "Failed to list MCP stdio tools: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");

    let ops: Vec<&str> = json["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["operation_id"].as_str())
        .collect();

    assert!(ops.contains(&"echo"), "Expected echo MCP stdio tool");
}

#[test]
fn test_mcp_stdio_call_tool() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let result = run_uxc(&[&endpoint, "echo", "--json", r#"{"message":"from stdio"}"#]);

    assert!(
        result.is_ok(),
        "Failed to call MCP stdio tool: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "from stdio");
}

#[test]
fn test_mcp_stdio_auth_required() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} auth_required", bin.display());

    let result = run_uxc(&[&endpoint, "echo", "--json", r#"{"message":"x"}"#]);

    assert!(result.is_err(), "Expected MCP stdio auth error");
}

// Note: Timeout scenario tests are not included here to keep the suite fast.
// Timeout behavior is implemented in all test servers and can be validated
// manually (or in dedicated tests) with UXC_TEST_TIMEOUT_MS.
// Example:
//   uxc-test-openapi-server timeout
//   uxc-test-graphql-server timeout
//   uxc-test-jsonrpc-server timeout
//   uxc-test-grpc-server timeout
//   uxc-test-mcp-http-server timeout
//   uxc-test-mcp-stdio-server timeout
// and then attempting to make requests with a short client timeout.
