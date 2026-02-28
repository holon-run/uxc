use mockito::Matcher;
use serde_json::Value;
use std::process::Command;

fn uxc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uxc"))
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).expect("stdout should be valid JSON")
}

fn openrpc_document() -> &'static str {
    r#"{
  "openrpc": "1.3.2",
  "info": {
    "title": "Math RPC",
    "version": "1.0.0"
  },
  "methods": [
    {
      "name": "subtract",
      "description": "Subtract two numbers",
      "paramStructure": "either",
      "params": [
        {
          "name": "minuend",
          "required": true,
          "schema": { "type": "number" }
        },
        {
          "name": "subtrahend",
          "required": true,
          "schema": { "type": "number" }
        }
      ],
      "result": {
        "name": "difference",
        "schema": { "type": "number" }
      }
    }
  ]
}"#
}

#[test]
fn host_help_lists_operations_from_rpc_discover() {
    let mut server = mockito::Server::new();
    let _discover = server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""method":"rpc\.discover""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{}}}"#,
            openrpc_document()
        ))
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("-h")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");

    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["kind"], "host_help");
    assert!(json["data"]["operations"].as_array().is_some_and(|ops| {
        ops.iter()
            .any(|op| op["operation_id"] == "subtract" && op["protocol_kind"] == "rpc_method")
    }));
}

#[test]
fn operation_help_from_openrpc() {
    let mut server = mockito::Server::new();
    let _discover = server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""method":"rpc\.discover""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{}}}"#,
            openrpc_document()
        ))
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("subtract")
        .arg("-h")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");

    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["kind"], "operation_detail");
    assert_eq!(json["data"]["operation_id"], "subtract");
    assert_eq!(json["data"]["input_schema"]["kind"], "openrpc_method");
}

#[test]
fn call_operation_uses_positional_params() {
    let mut server = mockito::Server::new();
    let _discover = server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""method":"rpc\.discover""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{}}}"#,
            openrpc_document()
        ))
        .create();

    let _call = server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""method":"subtract""#.to_string()))
        .match_body(Matcher::Regex(r#""params":\[42,23\]"#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"jsonrpc":"2.0","id":2,"result":19}"#)
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("subtract")
        .arg("--input-json")
        .arg(r#"{"minuend":42,"subtrahend":23}"#)
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");

    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "jsonrpc");
    assert_eq!(json["kind"], "call_result");
    assert_eq!(json["operation"], "subtract");
    assert_eq!(json["data"], 19);
}
