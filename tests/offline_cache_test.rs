use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serial_test::serial;

fn uxc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uxc"))
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

fn uxc_command_with_home(home: &Path) -> Command {
    let mut cmd = uxc_command();
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd
}

#[test]
#[serial]
fn operation_help_uses_stale_cache_fallback_with_meta() {
    daemon_stop_best_effort();
    let temp_home = tempfile::tempdir().expect("temp home should be created");

    let endpoint = {
        let mut server = mockito::Server::new();
        let _schema = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r##"{
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
}"##,
            )
            .create();

        let prime = uxc_command_with_home(temp_home.path())
            .arg(server.url())
            .arg("--cache-ttl")
            .arg("1")
            .arg("get:/pets")
            .arg("-h")
            .output()
            .expect("prime cache should succeed");
        assert!(prime.status.success(), "prime run should succeed");

        server.url()
    };

    std::thread::sleep(Duration::from_secs(2));

    let output = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("--cache-ttl")
        .arg("1")
        .arg("get:/pets")
        .arg("-h")
        .output()
        .expect("offline fallback should run");

    assert!(
        output.status.success(),
        "offline fallback should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "operation_detail");
    assert_eq!(json["meta"]["cache_fallback"], true);
    assert_eq!(json["meta"]["cache_stale"], true);
    assert_eq!(json["meta"]["cache_source"], "schema_cache");
    assert!(
        json["meta"]["cache_age_ms"].as_u64().is_some(),
        "cache_age_ms should be present"
    );
    daemon_stop_best_effort();
}

#[test]
#[serial]
fn host_help_offline_cache_miss_returns_actionable_error() {
    daemon_stop_best_effort();
    let temp_home = tempfile::tempdir().expect("temp home should be created");

    let output = uxc_command_with_home(temp_home.path())
        .arg("http://127.0.0.1:9")
        .arg("-h")
        .output()
        .expect("host help should run");

    assert!(!output.status.success(), "cache miss should fail");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "PROTOCOL_DETECTION_FAILED");

    let message = json["error"]["message"]
        .as_str()
        .expect("error.message should be a string");
    assert!(
        message.contains("No adapter found for URL"),
        "error message should be actionable, got: {message}"
    );
    daemon_stop_best_effort();
}

#[test]
#[serial]
fn operation_help_offline_cache_miss_returns_actionable_error() {
    daemon_stop_best_effort();
    let temp_home = tempfile::tempdir().expect("temp home should be created");

    let output = uxc_command_with_home(temp_home.path())
        .arg("http://127.0.0.1:9")
        .arg("get:/pets")
        .arg("-h")
        .output()
        .expect("operation help should run");

    assert!(!output.status.success(), "cache miss should fail");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "PROTOCOL_DETECTION_FAILED");

    let message = json["error"]["message"]
        .as_str()
        .expect("error.message should be a string");
    assert!(
        message.contains("No adapter found for URL"),
        "error message should be actionable, got: {message}"
    );
    daemon_stop_best_effort();
}
