//! Daemon logging integration tests
//!
//! Tests for daemon troubleshooting logs feature.

use serial_test::serial;

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

fn uxc_command() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap()
}
