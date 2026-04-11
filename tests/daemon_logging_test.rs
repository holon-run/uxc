//! Daemon logging integration tests
//!
//! Tests for daemon troubleshooting logs feature.

mod common;

use common::{test_server_binary, uxc_command, uxc_command_with_home};
use serial_test::serial;
use std::fs;

#[test]
#[serial]
fn daemon_status_includes_log_file_path() {
    // Stop any running daemon first
    let _ = uxc_command().arg("daemon").arg("stop").output();

    // Start daemon
    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    // Check status includes log_file
    let status = uxc_command()
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());

    let json: serde_json::Value = serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_status");

    // Verify log_file is present and points to daemon.log
    let log_file = json["data"]["log_file"].as_str();
    assert!(log_file.is_some(), "log_file should be present in status");
    assert!(
        log_file.unwrap().contains("daemon.log"),
        "log_file path should contain daemon.log"
    );

    // Cleanup
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

#[test]
#[serial]
fn daemon_creates_log_file() {
    use std::fs;
    use std::path::PathBuf;

    // Stop any running daemon first
    let _ = uxc_command().arg("daemon").arg("stop").output();

    // Determine log file location
    let log_dir = if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("uxc")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".uxc").join("daemon")
    } else {
        return; // Skip test if we can't determine log location
    };

    // Remove existing log file if present
    let log_file = log_dir.join("daemon.log");
    if log_file.exists() {
        let _ = fs::remove_file(&log_file);
    }

    // Start daemon
    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    // Give daemon time to initialize and write logs
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Check that log file was created
    assert!(
        log_file.exists(),
        "daemon.log should be created after daemon start"
    );

    // Verify log file contains JSON Lines format
    let content = fs::read_to_string(&log_file).expect("should be able to read log file");

    // Each line should be valid JSON
    for line in content.lines() {
        if !line.is_empty() {
            let _: serde_json::Value =
                serde_json::from_str(line).expect("each log line should be valid JSON");
        }
    }

    // Cleanup
    let _ = uxc_command().arg("daemon").arg("stop").output();
    let _ = fs::remove_file(&log_file);
}

#[test]
#[serial]
fn daemon_log_contains_start_event() {
    use std::fs;
    use std::path::PathBuf;

    // Stop any running daemon first
    let _ = uxc_command().arg("daemon").arg("stop").output();

    // Determine log file location
    let log_dir = if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("uxc")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".uxc").join("daemon")
    } else {
        return; // Skip test if we can't determine log location
    };

    let log_file = log_dir.join("daemon.log");
    if log_file.exists() {
        let _ = fs::remove_file(&log_file);
    }

    // Start daemon
    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    // Give daemon time to write logs
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Read and check log file
    let content = fs::read_to_string(&log_file).expect("should be able to read log file");

    // Look for daemon_start event
    assert!(
        content.contains("daemon_start"),
        "log should contain daemon_start event"
    );

    // Verify redaction is working (no raw secrets should be present)
    assert!(
        !content.contains("\"api_key\"") || content.contains("***"),
        "if api_key is logged, value should be redacted"
    );

    // Cleanup
    let _ = uxc_command().arg("daemon").arg("stop").output();
    let _ = fs::remove_file(&log_file);
}

#[test]
#[serial]
fn daemon_log_contains_stdio_session_lifecycle_events() {
    use std::fs;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    let _ = uxc_command().arg("daemon").arg("stop").output();

    let log_dir = if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("uxc")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".uxc").join("daemon")
    } else {
        return;
    };

    let log_file = log_dir.join("daemon.log");
    if log_file.exists() {
        let _ = fs::remove_file(&log_file);
    }

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());
    let first = uxc_command()
        .arg("--daemon-idle-ttl")
        .arg("1")
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"seed"}"#)
        .output()
        .expect("first call should run");
    assert!(first.status.success());

    thread::sleep(Duration::from_millis(1500));

    let second = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"trigger"}"#)
        .output()
        .expect("second call should run");
    assert!(second.status.success());

    thread::sleep(Duration::from_millis(200));

    let content = fs::read_to_string(&log_file).expect("should be able to read daemon log");
    assert!(
        content.contains("daemon_session_created"),
        "log should contain daemon_session_created event"
    );
    assert!(
        content.contains("daemon_session_removed"),
        "log should contain daemon_session_removed event"
    );
    assert!(
        content.contains("idle_reaped"),
        "log should include idle_reaped removal reason"
    );

    let _ = uxc_command().arg("daemon").arg("stop").output();
    let _ = fs::remove_file(&log_file);
}

#[test]
#[serial]
fn daemon_log_contains_lifecycle_snapshot_metadata_for_stateful_stdio_sessions() {
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    let temp_home = tempfile::tempdir().expect("temp home should be created");
    let _ = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("stop")
        .output();

    let log_dir = PathBuf::from(temp_home.path()).join("runtime").join("uxc");

    let log_file = log_dir.join("daemon.log");
    if log_file.exists() {
        let _ = fs::remove_file(&log_file);
    }

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} lifecycle_stateful_hold", bin.display());
    let first = uxc_command_with_home(temp_home.path())
        .arg("--daemon-idle-ttl")
        .arg("1")
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"seed"}"#)
        .output()
        .expect("first call should run");
    assert!(first.status.success());

    thread::sleep(Duration::from_millis(1500));

    let second = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"trigger"}"#)
        .output()
        .expect("second call should run");
    assert!(second.status.success());

    thread::sleep(Duration::from_millis(200));

    let content = fs::read_to_string(&log_file).expect("should be able to read daemon log");
    let entries = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("valid log json"))
        .collect::<Vec<_>>();
    assert!(
        entries.iter().any(|entry| {
            entry["meta"]["lifecycle_contract"]["reap_policy"] == "stateful"
                && entry["meta"]["last_lifecycle_snapshot"]["retention_reason"] == "interactive"
        }),
        "log should include lifecycle metadata for a stateful interactive session"
    );

    let _ = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("stop")
        .output();
    let _ = fs::remove_file(&log_file);
}
