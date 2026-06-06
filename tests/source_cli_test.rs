mod common;

use serial_test::serial;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use common::{start_test_server, uxc_command_with_home};

fn daemon_stop_best_effort_with_home(home: &Path) {
    let _ = uxc_command_with_home(home)
        .arg("daemon")
        .arg("stop")
        .output();
}

fn daemon_runtime_dir(home: &Path) -> PathBuf {
    home.join(".uxc").join("daemon")
}

fn read_json_output(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn run_source_status(home: &Path, namespace: &str, source_key: &str) -> serde_json::Value {
    let output = uxc_command_with_home(home)
        .arg("source")
        .arg("status")
        .arg(namespace)
        .arg(source_key)
        .output()
        .expect("uxc source status should run");
    assert!(
        output.status.success(),
        "source status should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    read_json_output(&output)
}

fn run_source_doctor(home: &Path, namespace: &str, source_key: &str) -> serde_json::Value {
    let output = uxc_command_with_home(home)
        .arg("source")
        .arg("doctor")
        .arg(namespace)
        .arg(source_key)
        .output()
        .expect("uxc source doctor should run");
    assert!(
        output.status.success(),
        "source doctor should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    read_json_output(&output)
}

fn wait_for_source_progress(home: &Path, namespace: &str, source_key: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let json = run_source_status(home, namespace, source_key);
        let data = &json["data"];
        let has_success = data["last_success_at_unix"].as_u64().is_some();
        let event_count = data["stream"]["event_count"].as_u64().unwrap_or(0);
        let has_checkpoint_kind = data["checkpoint"]["kind"].as_str().is_some();
        if has_success && event_count > 0 && has_checkpoint_kind {
            return json;
        }
        assert!(
            Instant::now() < deadline,
            "managed source did not make observable progress in time: {}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[serial]
fn source_status_surfaces_progress_and_checkpoint_for_poll_sources() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());
    let server = start_test_server("openapi", "ok");

    let ensure = uxc_command_with_home(temp_home.path())
        .arg("source")
        .arg("ensure")
        .arg("test")
        .arg("poll-events")
        .arg(&server.addr)
        .arg("get:/poll/events")
        .arg("--mode")
        .arg("poll")
        .arg("--poll-config")
        .arg(
            r#"{"interval_secs":1,"extract_items_pointer":"/items","request_cursor_arg":"cursor","response_cursor_pointer":"/next_cursor","checkpoint_strategy":{"type":"cursor_only"}}"#,
        )
        .output()
        .expect("uxc source ensure should run");
    assert!(
        ensure.status.success(),
        "source ensure should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ensure.stdout),
        String::from_utf8_lossy(&ensure.stderr)
    );

    let json = wait_for_source_progress(temp_home.path(), "test", "poll-events");
    let data = &json["data"];
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "source_status");
    assert_eq!(data["namespace"], "test");
    assert_eq!(data["source_key"], "poll-events");
    assert_eq!(data["mode"], "poll");
    assert_eq!(data["endpoint"], format!("http://{}", server.addr));
    assert_eq!(data["poll_interval_secs"], 1);
    assert!(data["last_success_at_unix"].as_u64().is_some());
    assert!(data["last_event_at_unix"].as_u64().is_some());
    assert!(data["written_events"].as_u64().unwrap_or(0) > 0);
    assert_eq!(data["reconnect_count"], 0);
    assert_eq!(data["checkpoint"]["kind"], "cursor_only");
    assert!(data["checkpoint"]["cursor"].is_string());
    assert!(data["stream"]["event_count"].as_u64().unwrap_or(0) > 0);
    assert!(data["stream"]["latest_offset"].as_u64().is_some());
    assert!(data["stream"]["latest_event_at_unix"].as_u64().is_some());

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn source_doctor_warns_on_legacy_cursor_file() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());
    let server = start_test_server("openapi", "ok");

    let ensure = uxc_command_with_home(temp_home.path())
        .arg("source")
        .arg("ensure")
        .arg("test")
        .arg("doctor-cursor")
        .arg(&server.addr)
        .arg("get:/poll/events")
        .arg("--mode")
        .arg("poll")
        .arg("--poll-config")
        .arg(
            r#"{"interval_secs":1,"extract_items_pointer":"/items","request_cursor_arg":"cursor","response_cursor_pointer":"/next_cursor","checkpoint_strategy":{"type":"cursor_only"}}"#,
        )
        .output()
        .expect("uxc source ensure should run");
    assert!(ensure.status.success(), "source ensure should succeed");

    let status_json = wait_for_source_progress(temp_home.path(), "test", "doctor-cursor");
    let run_id = status_json["data"]["run_id"]
        .as_str()
        .expect("run_id should be present");

    let healthy = run_source_doctor(temp_home.path(), "test", "doctor-cursor");
    assert_eq!(healthy["kind"], "source_doctor_result");
    assert_eq!(healthy["data"]["status"], "healthy");
    assert_eq!(healthy["data"]["legacy_cursor_file_present"], false);

    let cursor_path = daemon_runtime_dir(temp_home.path())
        .join("managed-source-cursors")
        .join(format!("{run_id}.cursor.json"));
    fs::create_dir_all(
        cursor_path
            .parent()
            .expect("legacy cursor path should have a parent"),
    )
    .expect("cursor parent dir should exist");
    fs::write(&cursor_path, br#"{"after_seq":123}"#).expect("legacy cursor file should be written");

    let warned = run_source_doctor(temp_home.path(), "test", "doctor-cursor");
    let issue_codes = warned["data"]["issues"]
        .as_array()
        .expect("issues should be an array")
        .iter()
        .filter_map(|issue| issue.get("code").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(warned["data"]["status"], "warn");
    assert_eq!(warned["data"]["legacy_cursor_file_present"], true);
    assert!(issue_codes.contains(&"legacy_cursor_file_present"));

    daemon_stop_best_effort_with_home(temp_home.path());
}
