//! CLI error output integration tests
//!
//! Tests that CLI failures return structured JSON error envelopes

use assert_cmd::Command;
use mockito::Server;

fn uxc() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("uxc"))
}

#[test]
fn protocol_detection_failure_uses_error_envelope() {
    let output = uxc()
        .arg("http://127.0.0.1:9")
        .arg("-h")
        .assert()
        .failure()
        .stdout(predicates::str::contains("PROTOCOL_DETECTION_FAILED"))
        .stderr(predicates::str::is_empty());

    // Verify JSON error envelope structure
    let stdout = output.get_output().stdout.clone();
    let json: serde_json::Value =
        serde_json::from_slice(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["ok"], false, "ok should be false");
    assert_eq!(json["error"]["code"], "PROTOCOL_DETECTION_FAILED");
    assert!(
        json["error"]["message"].is_string(),
        "error.message should be a string"
    );
}

#[test]
fn operation_execution_failure_uses_error_envelope() {
    let mut server = Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
  "openapi": "3.0.0",
  "info": { "title": "test", "version": "1.0.0" },
  "paths": {
    "/pets": {
      "get": {
        "summary": "list pets",
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"#,
        )
        .create();

    // Host help should succeed
    uxc().arg(server.url()).arg("-h").assert().success();

    // Call with non-existent operation should fail with error envelope
    let output = uxc()
        .arg(server.url())
        .arg("nonexistent")
        .assert()
        .failure();

    // Verify JSON error envelope structure
    let stdout = output.get_output().stdout.clone();
    let json: serde_json::Value =
        serde_json::from_slice(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["ok"], false, "ok should be false");
    assert!(json["error"].is_object(), "error should be an object");
    assert!(
        json["error"]["code"].is_string(),
        "error.code should be a string"
    );
    assert!(
        json["error"]["message"].is_string(),
        "error.message should be a string"
    );
}

#[test]
fn no_cache_and_refresh_schema_conflict_returns_invalid_argument() {
    let output = uxc()
        .arg("--no-cache")
        .arg("--refresh-schema")
        .arg("http://127.0.0.1:9")
        .arg("-h")
        .assert()
        .failure();

    let stdout = output.get_output().stdout.clone();
    let json: serde_json::Value =
        serde_json::from_slice(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["ok"], false, "ok should be false");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    let message = json["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("--no-cache"));
    assert!(message.contains("--refresh-schema"));
}
