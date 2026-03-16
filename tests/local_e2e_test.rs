//! Local E2E tests using test servers
//!
//! These tests verify that uxc can correctly interact with local controllable
//! test servers for each protocol.

mod common;

use common::{
    fresh_test_home_dir, run_uxc, run_uxc_in_home, start_test_server, test_server_binary,
};
use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn grpcurl_available() -> bool {
    Command::new("grpcurl")
        .arg("-help")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false)
}

fn mcp_http_endpoint(addr: &str) -> String {
    format!("http://{addr}/mcp")
}

fn wait_for_file_contains(path: &std::path::Path, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(path) {
            if contents.contains(needle) {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn wait_for_file_match_count(
    path: &std::path::Path,
    needle: &str,
    expected_count: usize,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(path) {
            if contents.matches(needle).count() >= expected_count {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[serial_test::serial]
fn test_openapi_host_help_lists_operations() {
    let _server = start_test_server("openapi", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "-h"]);

    assert!(
        result.is_ok(),
        "Failed to run OpenAPI host help: {:?}",
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
#[serial_test::serial]
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
#[serial_test::serial]
fn test_openapi_call_post_operation() {
    let _server = start_test_server("openapi", "ok");

    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "post:/users",
        "--input-json",
        r#"{"name":"Charlie","email":"charlie@example.com"}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call OpenAPI POST operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "openapi");
    assert_eq!(json["data"]["name"], "Charlie");
    assert_eq!(json["data"]["email"], "charlie@example.com");
}

#[test]
#[serial_test::serial]
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
#[serial_test::serial]
fn test_graphql_host_help_lists_operations() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "-h"]);

    assert!(
        result.is_ok(),
        "Failed to run GraphQL host help: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert!(
        json["data"]["operations"].is_array(),
        "Expected operations array in GraphQL host_help output"
    );
}

#[test]
#[serial_test::serial]
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
#[serial_test::serial]
fn test_graphql_call_with_args() {
    let _server = start_test_server("graphql", "ok");

    // Call the user query with an ID argument
    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "query/user",
        "--input-json",
        r#"{"id":"2"}"#,
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
    assert_eq!(json["data"]["user"]["id"], "2");
    assert_eq!(json["data"]["user"]["name"], "Bob");
}

#[test]
#[serial_test::serial]
fn test_graphql_call_mutation() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "mutation/createUser",
        "--input-json",
        r#"{"name":"Dave","email":"dave@example.com"}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call GraphQL mutation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert_eq!(json["data"]["createUser"]["name"], "Dave");
    assert_eq!(json["data"]["createUser"]["email"], "dave@example.com");
}

#[test]
#[serial_test::serial]
fn test_graphql_call_mutation_with_input_object_via_positional_json() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "mutation/issueCreate",
        r#"{"input":{"teamId":"T1","title":"GraphQL input object"}}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call GraphQL mutation with positional JSON object: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert_eq!(json["data"]["issueCreate"]["teamId"], "T1");
    assert_eq!(json["data"]["issueCreate"]["title"], "GraphQL input object");
}

#[test]
#[serial_test::serial]
fn test_graphql_call_mutation_rejects_unknown_argument() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "mutation/issueCreate",
        r#"{"unknown":"value"}"#,
    ]);

    assert!(result.is_err(), "Expected unknown argument error");
    let err = result.unwrap_err();
    assert!(
        err.contains("Unknown argument(s)") && err.contains("unknown"),
        "Expected unknown argument details, got: {}",
        err
    );
}

#[test]
#[serial_test::serial]
fn test_graphql_call_mutation_rejects_missing_required_argument() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "mutation/issueCreate",
        r#"{}"#,
    ]);

    assert!(result.is_err(), "Expected missing required argument error");
    let err = result.unwrap_err();
    assert!(
        err.contains("Missing required GraphQL argument(s)") && err.contains("input"),
        "Expected missing required argument details, got: {}",
        err
    );
}

#[test]
#[serial_test::serial]
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
#[serial_test::serial]
fn test_jsonrpc_host_help_lists_operations() {
    let _server = start_test_server("jsonrpc", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "-h"]);

    assert!(
        result.is_ok(),
        "Failed to run JSON-RPC host help: {:?}",
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
#[serial_test::serial]
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
#[serial_test::serial]
fn test_jsonrpc_call_with_args() {
    let _server = start_test_server("jsonrpc", "ok");

    // Call the get_user method with an ID argument
    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "get_user",
        "--input-json",
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
#[serial_test::serial]
fn test_jsonrpc_call_create_user() {
    let _server = start_test_server("jsonrpc", "ok");

    let result = run_uxc(&[
        &format!("http://{}/", _server.addr),
        "create_user",
        "--input-json",
        r#"{"name":"Erin","email":"erin@example.com"}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call JSON-RPC create_user: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["data"]["name"], "Erin");
    assert_eq!(json["data"]["email"], "erin@example.com");
}

#[test]
#[serial_test::serial]
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
#[serial_test::serial]
fn test_openapi_malformed_response() {
    let _server = start_test_server("openapi", "malformed");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "get:/health"]);
    assert!(
        result.is_err(),
        "Expected malformed OpenAPI response to fail, got success"
    );
}

#[test]
#[serial_test::serial]
fn test_graphql_malformed_response() {
    let _server = start_test_server("graphql", "malformed");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "query/health"]);
    assert!(
        result.is_err(),
        "Expected GraphQL malformed scenario to fail with invalid schema/introspection response"
    );
}

#[test]
#[serial_test::serial]
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
#[serial_test::serial]
fn test_grpc_host_help_lists_operations() {
    let _server = start_test_server("grpc", "ok");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "-h"]);

    assert!(result.is_ok(), "Failed to run gRPC host help: {:?}", result);

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
#[serial_test::serial]
fn test_grpc_call_method() {
    if !grpcurl_available() {
        eprintln!("Skipping gRPC call test because grpcurl is not installed");
        return;
    }

    let _server = start_test_server("grpc", "ok");

    let result = run_uxc(&[
        &format!("http://{}", _server.addr),
        "addsvc.Add/Sum",
        "--input-json",
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
#[serial_test::serial]
fn test_grpc_call_unknown_method_fails() {
    if !grpcurl_available() {
        eprintln!("Skipping gRPC unknown method test because grpcurl is not installed");
        return;
    }

    let _server = start_test_server("grpc", "ok");

    let result = run_uxc(&[
        &format!("http://{}", _server.addr),
        "addsvc.Add/Unknown",
        "--input-json",
        r#"{"a":1,"b":2}"#,
    ]);

    assert!(
        result.is_err(),
        "Expected unknown gRPC method to fail, got success"
    );
}

#[test]
#[serial_test::serial]
fn test_grpc_auth_required() {
    let _server = start_test_server("grpc", "auth_required");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "-h"]);

    assert!(
        result.is_err(),
        "Expected gRPC auth/reflection error, got success"
    );
}

#[test]
#[serial_test::serial]
fn test_mcp_http_host_help_lists_operations() {
    let _server = start_test_server("mcp-http", "ok");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&[&endpoint, "-h"]);

    assert!(
        result.is_ok(),
        "Failed to run MCP HTTP host help: {:?}",
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
#[serial_test::serial]
fn test_mcp_http_call_tool() {
    let _server = start_test_server("mcp-http", "ok");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&[
        &endpoint,
        "echo",
        "--input-json",
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
#[serial_test::serial]
fn test_mcp_http_call_tool_includes_structured_content() {
    let _server = start_test_server("mcp-http", "structured_content");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&[
        &endpoint,
        "echo",
        "--input-json",
        r#"{"message":"hello structured"}"#,
    ]);

    assert!(result.is_ok(), "Failed to call MCP HTTP tool: {:?}", result);

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "hello structured");
    assert_eq!(
        json["data"]["structuredContent"]["message"],
        "hello structured"
    );
    assert_eq!(json["data"]["structuredContent"]["source"], "mcp-http");
    assert_eq!(json["data"]["structuredContent"]["length"], 16);
}

#[test]
#[serial_test::serial]
fn test_mcp_http_auth_required() {
    let _server = start_test_server("mcp-http", "auth_required");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&["--refresh-schema", &endpoint, "-h"]);

    let err = result.expect_err("Expected MCP HTTP auth error, got success");
    let err_lower = err.to_ascii_lowercase();
    assert!(
        err_lower.contains("401")
            || err_lower.contains("unauthorized")
            || err_lower.contains("oauth")
            || err_lower.contains("auth"),
        "Expected auth-related error, got: {}",
        err
    );
}

#[test]
#[serial_test::serial]
fn test_mcp_http_host_help_includes_service_metadata() {
    let _server = start_test_server("mcp-http", "ok");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&[&endpoint, "-h"]);
    assert!(result.is_ok(), "Failed to run MCP HTTP help: {:?}", result);

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "host_help");
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["service"]["name"], "uxc-test-mcp-http");
    assert_eq!(
        json["data"]["service"]["description"],
        "MCP HTTP test server for local e2e"
    );
}

#[test]
#[serial_test::serial]
fn test_mcp_http_text_help_prints_service_summary() {
    let _server = start_test_server("mcp-http", "ok");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&["--text", &endpoint, "-h"]);
    assert!(result.is_ok(), "Failed to run MCP HTTP help: {:?}", result);

    let output = result.unwrap();
    assert!(output.contains("Service:"));
    assert!(output.contains("Name: uxc-test-mcp-http"));
    assert!(output.contains("Description: MCP HTTP test server for local e2e"));
}

#[test]
#[serial_test::serial]
fn test_mcp_http_help_uses_cached_schema_when_tools_list_fails_after_first() {
    let server = start_test_server("mcp-http", "tools_list_fail_after_first");
    let endpoint = mcp_http_endpoint(&server.addr.to_string());
    let test_home = fresh_test_home_dir();

    let first = run_uxc_in_home(&[&endpoint, "-h"], &test_home);
    assert!(
        first.is_ok(),
        "Initial MCP HTTP help should succeed: {:?}",
        first
    );

    let second = run_uxc_in_home(&[&endpoint, "-h"], &test_home);
    assert!(
        second.is_ok(),
        "Second MCP HTTP help should use cache and succeed: {:?}",
        second
    );

    let output = second.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "host_help");
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["meta"]["cache_source"], "schema_cache");
}

#[test]
#[serial_test::serial]
fn test_mcp_http_execute_does_not_depend_on_tools_list() {
    let server = start_test_server("mcp-http", "tools_list_fail_after_first");
    let endpoint = mcp_http_endpoint(&server.addr.to_string());
    let test_home = fresh_test_home_dir();

    let prime_help = run_uxc_in_home(&[&endpoint, "-h"], &test_home);
    assert!(
        prime_help.is_ok(),
        "Initial MCP HTTP help should succeed: {:?}",
        prime_help
    );

    let call = run_uxc_in_home(
        &[
            &endpoint,
            "echo",
            "--input-json",
            r#"{"message":"hello cache"}"#,
        ],
        &test_home,
    );
    assert!(
        call.is_ok(),
        "MCP HTTP execute should succeed without tools/list dependency: {:?}",
        call
    );

    let output = call.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "hello cache");
}

#[test]
#[serial_test::serial]
fn test_mcp_stdio_host_help_lists_operations() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let result = run_uxc(&[&endpoint, "-h"]);

    assert!(
        result.is_ok(),
        "Failed to run MCP stdio host help: {:?}",
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
#[serial_test::serial]
fn test_mcp_stdio_call_tool() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let result = run_uxc(&[
        &endpoint,
        "echo",
        "--input-json",
        r#"{"message":"from stdio"}"#,
    ]);

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
#[serial_test::serial]
fn test_mcp_stdio_call_tool_includes_structured_content() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} structured_content", bin.display());

    let result = run_uxc(&[
        &endpoint,
        "echo",
        "--input-json",
        r#"{"message":"stdio structured"}"#,
    ]);

    assert!(
        result.is_ok(),
        "Failed to call MCP stdio tool: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "stdio structured");
    assert_eq!(
        json["data"]["structuredContent"]["message"],
        "stdio structured"
    );
    assert_eq!(json["data"]["structuredContent"]["source"], "mcp-stdio");
    assert_eq!(json["data"]["structuredContent"]["length"], 16);
}

#[test]
#[serial_test::serial]
fn test_mcp_stdio_preserves_explicit_empty_object_arguments() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} empty_object_required", bin.display());

    let result = run_uxc(&[&endpoint, "empty_check", "--input-json", r#"{}"#]);

    assert!(
        result.is_ok(),
        "Failed to call MCP stdio tool with empty object args: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "received-empty-object");
    assert_eq!(
        json["data"]["structuredContent"]["hasArgumentsObject"],
        true
    );
    assert_eq!(json["data"]["structuredContent"]["argumentCount"], 0);
}

#[test]
#[serial_test::serial]
fn test_mcp_stdio_auth_required() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} auth_required", bin.display());

    let result = run_uxc(&[&endpoint, "echo", "--input-json", r#"{"message":"x"}"#]);

    assert!(result.is_err(), "Expected MCP stdio auth error");
}

#[test]
#[serial_test::serial]
fn test_http_subscribe_start_status_stop_writes_file() {
    let server = start_test_server("openapi", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("http-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("http://{}/stream", server.addr);

    let start = run_uxc_in_home(
        &["subscribe", "start", &endpoint, "--sink", &sink_spec],
        &test_home,
    );
    assert!(start.is_ok(), "HTTP subscribe start failed: {:?}", start);
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""value":2"#, Duration::from_secs(5)),
        "HTTP subscribe sink did not receive expected events"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(status.is_ok(), "HTTP subscribe status failed: {:?}", status);
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "http");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "HTTP subscribe stop failed: {:?}", stop);
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_durable_subscribe_auto_resumes_after_daemon_restart() {
    let server = start_test_server("openapi", "ok");
    let endpoint = format!("http://{}/stream", server.addr);
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("http-subscribe-durable.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());

    let start = run_uxc_in_home(
        &["subscribe", "start", &endpoint, "--sink", &sink_spec],
        &test_home,
    );
    assert!(start.is_ok(), "durable subscribe start failed: {:?}", start);
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""value":2"#, Duration::from_secs(5)),
        "durable sink did not receive initial events"
    );

    let stop_daemon = run_uxc_in_home(&["daemon", "stop"], &test_home);
    assert!(stop_daemon.is_ok(), "daemon stop failed: {:?}", stop_daemon);
    let start_daemon = run_uxc_in_home(&["daemon", "start"], &test_home);
    assert!(
        start_daemon.is_ok(),
        "daemon start failed: {:?}",
        start_daemon
    );

    assert!(
        wait_for_file_match_count(&sink_path, r#""value":1"#, 2, Duration::from_secs(5)),
        "durable sink did not receive resumed events after daemon restart"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "durable subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["data"]["durable"], true);
    assert_eq!(status_json["data"]["auto_resume"], true);
    assert_eq!(status_json["data"]["restart_count"], 1);
    assert!(status_json["data"]["last_resume_at_unix"].is_number());

    let cleanup = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(
        cleanup.is_ok(),
        "durable subscribe cleanup failed: {:?}",
        cleanup
    );
}

#[test]
#[serial_test::serial]
fn test_poll_subscribe_start_status_stop_writes_file() {
    let server = start_test_server("openapi", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("poll-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("http://{}/", server.addr);
    let poll_config = r#"{"interval_secs":1,"extract_items_pointer":"/items","request_cursor_arg":"cursor","response_cursor_pointer":"/next_cursor","checkpoint_strategy":{"type":"cursor_only"}}"#;

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "get:/poll/events",
            "--mode",
            "poll",
            "--poll-config",
            poll_config,
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(start.is_ok(), "Poll subscribe start failed: {:?}", start);
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "openapi");
    assert_eq!(start_json["data"]["mode"], "poll");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""id":3"#, Duration::from_secs(5)),
        "Poll subscribe sink did not receive expected events"
    );
    assert!(
        wait_for_file_contains(
            &sink_path,
            r#""event_kind":"checkpoint""#,
            Duration::from_secs(5)
        ),
        "Poll subscribe sink did not record checkpoint events"
    );

    let sink_body = std::fs::read_to_string(&sink_path).expect("read sink file");
    assert_eq!(sink_body.matches(r#""id":1"#).count(), 1);
    assert_eq!(sink_body.matches(r#""id":2"#).count(), 1);
    assert_eq!(sink_body.matches(r#""id":3"#).count(), 1);

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(status.is_ok(), "Poll subscribe status failed: {:?}", status);
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "openapi");
    assert_eq!(status_json["data"]["mode"], "poll");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "Poll subscribe stop failed: {:?}", stop);
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_ephemeral_subscribe_is_not_resumed_after_daemon_restart() {
    let server = start_test_server("openapi", "ok");
    let endpoint = format!("http://{}/stream", server.addr);
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("http-subscribe-ephemeral.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "--ephemeral",
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "ephemeral subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""value":2"#, Duration::from_secs(5)),
        "ephemeral sink did not receive initial events"
    );

    let stop_daemon = run_uxc_in_home(&["daemon", "stop"], &test_home);
    assert!(stop_daemon.is_ok(), "daemon stop failed: {:?}", stop_daemon);
    let start_daemon = run_uxc_in_home(&["daemon", "start"], &test_home);
    assert!(
        start_daemon.is_ok(),
        "daemon start failed: {:?}",
        start_daemon
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "ephemeral subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["data"]["durable"], false);
    assert_eq!(status_json["data"]["auto_resume"], false);
    assert_eq!(status_json["data"]["status"], "stopped_after_restart");
    assert!(status_json["data"]["last_resume_at_unix"].is_null());

    let cleanup = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(
        cleanup.is_ok(),
        "ephemeral subscribe cleanup failed: {:?}",
        cleanup
    );
}

#[test]
#[serial_test::serial]
fn test_poll_subscribe_supports_item_derived_cursor_for_telegram_style_offsets() {
    let server = start_test_server("openapi", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("telegram-poll-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("http://{}/", server.addr);
    let poll_config = r#"{"interval_secs":1,"extract_items_pointer":"/result","request_cursor_arg":"offset","cursor_from_item_pointer":"/update_id","cursor_transform":"increment","checkpoint_strategy":{"type":"item_key","item_key_pointer":"/update_id"}}"#;

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "get:/poll/telegram-updates",
            "--mode",
            "poll",
            "--poll-config",
            poll_config,
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "Telegram-style poll subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "openapi");
    assert_eq!(start_json["data"]["mode"], "poll");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""update_id":102"#, Duration::from_secs(5)),
        "Telegram-style poll subscribe sink did not receive expected updates"
    );

    let sink_body = std::fs::read_to_string(&sink_path).expect("read sink file");
    assert_eq!(sink_body.matches(r#""update_id":100"#).count(), 1);
    assert_eq!(sink_body.matches(r#""update_id":101"#).count(), 1);
    assert_eq!(sink_body.matches(r#""update_id":102"#).count(), 1);
    assert!(
        sink_body.contains(r#""event_kind":"checkpoint""#),
        "Telegram-style poll subscribe sink did not record checkpoint events"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "Telegram-style poll subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "openapi");
    assert_eq!(status_json["data"]["mode"], "poll");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(
        stop.is_ok(),
        "Telegram-style poll subscribe stop failed: {:?}",
        stop
    );
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_graphql_subscribe_start_status_stop_writes_file() {
    let server = start_test_server("graphql", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("graphql-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("http://{}/", server.addr);

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "subscription/messageAdded",
            r#"{"roomId":"room-42","_select":"id roomId body"}"#,
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(start.is_ok(), "GraphQL subscribe start failed: {:?}", start);
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "graphql");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""roomId":"room-42""#, Duration::from_secs(5)),
        "GraphQL subscribe sink did not receive expected events"
    );
    assert!(
        wait_for_file_contains(&sink_path, r#""reason":"complete""#, Duration::from_secs(5)),
        "GraphQL subscribe sink did not record completion"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "GraphQL subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "graphql");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "GraphQL subscribe stop failed: {:?}", stop);
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_graphql_subscribe_falls_back_to_legacy_profile() {
    let server = start_test_server("graphql", "legacy");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("graphql-legacy-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("http://{}/", server.addr);

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "subscription/messageAdded",
            r#"{"roomId":"room-legacy","_select":"id roomId body"}"#,
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "GraphQL legacy subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "graphql");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(
            &sink_path,
            r#""graphql_profile":"graphql_ws_legacy""#,
            Duration::from_secs(5)
        ),
        "GraphQL legacy subscribe sink did not record legacy profile data"
    );
    assert!(
        wait_for_file_contains(
            &sink_path,
            r#""compatibility_fallback":true"#,
            Duration::from_secs(5)
        ),
        "GraphQL legacy subscribe sink did not record compatibility fallback"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "GraphQL legacy subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "graphql");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(
        stop.is_ok(),
        "GraphQL legacy subscribe stop failed: {:?}",
        stop
    );
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_raw_websocket_subscribe_start_status_stop_writes_frames() {
    let server = start_test_server("websocket", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("websocket-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("ws://{}/frames", server.addr);

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "--transport",
            "websocket",
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "raw websocket subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "websocket");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""symbol":"BTCUSDT""#, Duration::from_secs(5)),
        "raw websocket sink did not record json text frame"
    );
    assert!(
        wait_for_file_contains(&sink_path, r#""text":"tick""#, Duration::from_secs(5)),
        "raw websocket sink did not record plain text frame"
    );
    assert!(
        wait_for_file_contains(&sink_path, r#""base64":"AQID""#, Duration::from_secs(5)),
        "raw websocket sink did not record binary frame"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "raw websocket subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "websocket");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(
        stop.is_ok(),
        "raw websocket subscribe stop failed: {:?}",
        stop
    );
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_raw_websocket_subscribe_sends_init_frames_and_subprotocol() {
    let server = start_test_server("websocket", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("websocket-init.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("ws://{}/init", server.addr);

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "--transport",
            "websocket",
            "--init-frame",
            r#"{"op":"subscribe","channel":"trades"}"#,
            "--init-frame",
            "ping",
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "raw websocket init subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "websocket");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(
            &sink_path,
            r#""received_frames":["{\"op\":\"subscribe\",\"channel\":\"trades\"}","ping"]"#,
            Duration::from_secs(5)
        ),
        "raw websocket sink did not record init frames"
    );

    let subprotocol_endpoint = format!("ws://{}/subprotocol", server.addr);
    let second_sink_path = test_home.join("websocket-subprotocol.ndjson");
    let second_sink_spec = format!("file:{}", second_sink_path.display());
    let second_start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &subprotocol_endpoint,
            "--transport",
            "websocket",
            "--subprotocol",
            "market.v1",
            "--sink",
            &second_sink_spec,
        ],
        &test_home,
    );
    assert!(
        second_start.is_ok(),
        "raw websocket subprotocol subscribe start failed: {:?}",
        second_start
    );
    let second_json: serde_json::Value = serde_json::from_str(&second_start.unwrap()).unwrap();
    let second_job_id = second_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(
            &second_sink_path,
            r#""subprotocol":"market.v1""#,
            Duration::from_secs(5)
        ),
        "raw websocket sink did not record negotiated subprotocol"
    );

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "raw websocket init stop failed: {:?}", stop);
    let second_stop = run_uxc_in_home(&["subscribe", "stop", &second_job_id], &test_home);
    assert!(
        second_stop.is_ok(),
        "raw websocket subprotocol stop failed: {:?}",
        second_stop
    );
}

#[test]
#[serial_test::serial]
fn test_jsonrpc_subscribe_start_status_stop_writes_file() {
    let server = start_test_server("jsonrpc", "ok");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("jsonrpc-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("ws://{}/", server.addr);

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "eth_subscribe",
            r#"{"params":["newHeads"]}"#,
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "JSON-RPC subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "jsonrpc");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""hash":"0xabc""#, Duration::from_secs(5)),
        "JSON-RPC subscribe sink did not receive expected events"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "JSON-RPC subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "jsonrpc");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "JSON-RPC subscribe stop failed: {:?}", stop);
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_jsonrpc_sui_style_subscribe_start_status_stop_writes_file() {
    let server = start_test_server("jsonrpc", "sui_pubsub");
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("jsonrpc-sui-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());
    let endpoint = format!("ws://{}/", server.addr);

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "suix_subscribeEvent",
            r#"{"params":[{"Package":"0x2"}]}"#,
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "Sui-style JSON-RPC subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    assert_eq!(start_json["data"]["protocol"], "jsonrpc");
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""packageId":"0x2""#, Duration::from_secs(5)),
        "Sui-style JSON-RPC subscribe sink did not receive expected events"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "Sui-style JSON-RPC subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "jsonrpc");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(
        stop.is_ok(),
        "Sui-style JSON-RPC subscribe stop failed: {:?}",
        stop
    );
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_mcp_stdio_subscribe_start_status_stop_writes_file() {
    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("mcp-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "--resource-uri",
            "test://resource",
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(start.is_ok(), "MCP subscribe start failed: {:?}", start);
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""value":2"#, Duration::from_secs(5)),
        "MCP subscribe sink did not receive expected events"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(status.is_ok(), "MCP subscribe status failed: {:?}", status);
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "mcp");
    assert_eq!(status_json["data"]["resource_uri"], "test://resource");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "MCP subscribe stop failed: {:?}", stop);
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_mcp_http_subscribe_start_status_stop_writes_file() {
    let server = start_test_server("mcp-http", "ok");
    let endpoint = mcp_http_endpoint(&server.addr);
    let test_home = fresh_test_home_dir();
    let sink_path = test_home.join("mcp-http-subscribe.ndjson");
    let sink_spec = format!("file:{}", sink_path.display());

    let start = run_uxc_in_home(
        &[
            "subscribe",
            "start",
            &endpoint,
            "--resource-uri",
            "test://resource",
            "--sink",
            &sink_spec,
        ],
        &test_home,
    );
    assert!(
        start.is_ok(),
        "MCP HTTP subscribe start failed: {:?}",
        start
    );
    let start_json: serde_json::Value = serde_json::from_str(&start.unwrap()).unwrap();
    assert_eq!(start_json["ok"], true);
    let job_id = start_json["data"]["job_id"].as_str().unwrap().to_string();

    assert!(
        wait_for_file_contains(&sink_path, r#""value":2"#, Duration::from_secs(5)),
        "MCP HTTP subscribe sink did not receive expected events"
    );

    let status = run_uxc_in_home(&["subscribe", "status", &job_id], &test_home);
    assert!(
        status.is_ok(),
        "MCP HTTP subscribe status failed: {:?}",
        status
    );
    let status_json: serde_json::Value = serde_json::from_str(&status.unwrap()).unwrap();
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["protocol"], "mcp");
    assert_eq!(status_json["data"]["resource_uri"], "test://resource");

    let stop = run_uxc_in_home(&["subscribe", "stop", &job_id], &test_home);
    assert!(stop.is_ok(), "MCP HTTP subscribe stop failed: {:?}", stop);
    let stop_json: serde_json::Value = serde_json::from_str(&stop.unwrap()).unwrap();
    assert_eq!(stop_json["ok"], true);
    assert_eq!(stop_json["data"]["stopped"], true);
}

#[test]
#[serial_test::serial]
fn test_openapi_describe_operation() {
    let _server = start_test_server("openapi", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "get:/health", "-h"]);

    assert!(
        result.is_ok(),
        "Failed to describe OpenAPI operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "openapi");
    assert_eq!(json["data"]["operation_id"], "get:/health");
}

#[test]
#[serial_test::serial]
fn test_graphql_describe_operation() {
    let _server = start_test_server("graphql", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "query/user", "-h"]);

    assert!(
        result.is_ok(),
        "Failed to describe GraphQL operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "graphql");
    assert_eq!(json["data"]["operation_id"], "query/user");
}

#[test]
#[serial_test::serial]
fn test_jsonrpc_describe_operation() {
    let _server = start_test_server("jsonrpc", "ok");

    let result = run_uxc(&[&format!("http://{}/", _server.addr), "get_user", "-h"]);

    assert!(
        result.is_ok(),
        "Failed to describe JSON-RPC operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["data"]["operation_id"], "get_user");
}

#[test]
#[serial_test::serial]
fn test_grpc_describe_operation() {
    let _server = start_test_server("grpc", "ok");

    let result = run_uxc(&[&format!("http://{}", _server.addr), "addsvc.Add/Sum", "-h"]);

    assert!(
        result.is_ok(),
        "Failed to describe gRPC operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "grpc");
    assert_eq!(json["data"]["operation_id"], "addsvc.Add/Sum");
}

#[test]
#[serial_test::serial]
fn test_mcp_http_describe_operation() {
    let _server = start_test_server("mcp-http", "ok");
    let endpoint = mcp_http_endpoint(&_server.addr);

    let result = run_uxc(&[&endpoint, "echo", "-h"]);

    assert!(
        result.is_ok(),
        "Failed to describe MCP HTTP operation: {:?}",
        result
    );

    let output = result.unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["operation_id"], "echo");
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
