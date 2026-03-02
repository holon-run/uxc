mod common;

use assert_cmd::Command;
use common::test_server_binary;
use serial_test::serial;
use std::sync::{Arc, Barrier};

fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

#[test]
#[serial]
fn mcp_stdio_daemon_session_reuse_signal_validation() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let cold = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"first"}"#)
        .output()
        .expect("cold call should run");
    assert!(
        cold.status.success(),
        "cold call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cold.stdout),
        String::from_utf8_lossy(&cold.stderr)
    );

    let cold_json: serde_json::Value =
        serde_json::from_slice(&cold.stdout).expect("cold stdout should be valid JSON");
    assert_eq!(cold_json["ok"], true);
    assert_eq!(cold_json["protocol"], "mcp");
    assert_eq!(cold_json["meta"]["daemon_used"], true);

    let warm = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"second"}"#)
        .output()
        .expect("warm call should run");
    assert!(
        warm.status.success(),
        "warm call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&warm.stdout),
        String::from_utf8_lossy(&warm.stderr)
    );

    let warm_json: serde_json::Value =
        serde_json::from_slice(&warm.stdout).expect("warm stdout should be valid JSON");
    assert_eq!(warm_json["ok"], true);
    assert_eq!(warm_json["protocol"], "mcp");
    assert_eq!(warm_json["meta"]["daemon_session_reused"], true);

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn daemon_status_exposes_reuse_counter() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let _ = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"seed"}"#)
        .output()
        .expect("seed call should run");

    let _ = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"warm"}"#)
        .output()
        .expect("warm call should run");

    let status = uxc_command()
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());

    let json: serde_json::Value = serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_status");

    assert_eq!(json["data"]["running"], true);
    assert!(json["data"]["pid"].as_u64().is_some());
    assert!(json["data"]["socket"]
        .as_str()
        .is_some_and(|s| s.contains("uxc.sock")));
    assert!(json["data"]["started_at_unix"].as_u64().is_some());
    assert!(json["data"]["request_count"].as_u64().is_some());
    assert!(json["data"]["mcp_stdio_sessions"].as_u64().is_some());
    assert!(json["data"]["mcp_http_sessions"].as_u64().is_some());

    let reuse_hits = json["data"]["mcp_reuse_hits"]
        .as_u64()
        .expect("mcp_reuse_hits should be u64");
    assert!(reuse_hits >= 1, "expected at least one reuse hit");

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn concurrent_cold_calls_share_stdio_session() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let workers = 6;
    let barrier = Arc::new(Barrier::new(workers));
    let mut joins = Vec::new();
    for i in 0..workers {
        let endpoint = endpoint.clone();
        let barrier = barrier.clone();
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            uxc_command()
                .arg(&endpoint)
                .arg("echo")
                .arg("--input-json")
                .arg(format!(r#"{{"message":"cold-{i}"}}"#))
                .output()
                .expect("concurrent cold call should run")
        }));
    }

    for output in joins {
        let output = output.join().expect("thread should join");
        assert!(
            output.status.success(),
            "concurrent call should succeed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let status = uxc_command()
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());
    let json: serde_json::Value = serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_status");
    assert_eq!(json["data"]["running"], true);

    let stdio_sessions = json["data"]["mcp_stdio_sessions"]
        .as_u64()
        .expect("mcp_stdio_sessions should be u64");
    assert_eq!(stdio_sessions, 1, "expected a single stdio session");

    let reuse_hits = json["data"]["mcp_reuse_hits"]
        .as_u64()
        .expect("mcp_reuse_hits should be u64");
    assert!(
        reuse_hits >= 1,
        "expected at least one reuse hit under concurrent cold calls"
    );

    daemon_stop_best_effort();
}
