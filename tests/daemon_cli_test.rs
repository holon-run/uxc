use assert_cmd::Command;
use serial_test::serial;

fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

#[test]
#[serial]
fn daemon_start_status_stop_lifecycle() {
    daemon_stop_best_effort();

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

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
    assert_eq!(json["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["data"]["client_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["data"]["version_mismatch"], false);

    let stop = uxc_command()
        .arg("daemon")
        .arg("stop")
        .output()
        .expect("daemon stop should run");
    assert!(stop.status.success());

    // Stop path should wait for daemon to become unreachable.
    let status_after_stop = uxc_command()
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status after stop should run");
    assert!(status_after_stop.status.success());
    let json_after_stop: serde_json::Value =
        serde_json::from_slice(&status_after_stop.stdout).expect("valid json");
    assert_eq!(json_after_stop["ok"], true);
    assert_eq!(json_after_stop["data"]["running"], false);
    assert!(json_after_stop["data"]["error"]["message"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
}

#[test]
#[serial]
fn endpoint_host_help_autostarts_daemon_and_sets_meta() {
    daemon_stop_best_effort();

    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
  "openapi": "3.0.0",
  "info": { "title": "test", "version": "1.0.0" },
  "paths": { "/health": { "get": { "responses": { "200": { "description": "ok" } } } } }
}"#,
        )
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("-h")
        .output()
        .expect("host help should run");

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["meta"]["daemon_used"], true);
    assert_eq!(json["meta"]["daemon_autostarted"], true);
    assert_eq!(json["meta"]["daemon_restarted_for_version_mismatch"], false);

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn daemon_start_reports_started_now_and_already_running() {
    daemon_stop_best_effort();

    let first = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("first daemon start should run");
    assert!(first.status.success());
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).expect("valid json");
    assert_eq!(first_json["ok"], true);
    assert_eq!(first_json["kind"], "daemon_start_result");
    assert_eq!(first_json["data"]["started_now"], true);
    assert_eq!(first_json["data"]["already_running"], false);
    assert_eq!(first_json["data"]["restarted_for_version_mismatch"], false);
    assert_eq!(first_json["data"]["version"], env!("CARGO_PKG_VERSION"));

    let second = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("second daemon start should run");
    assert!(second.status.success());
    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("valid json");
    assert_eq!(second_json["ok"], true);
    assert_eq!(second_json["kind"], "daemon_start_result");
    assert_eq!(second_json["data"]["started_now"], false);
    assert_eq!(second_json["data"]["already_running"], true);
    assert_eq!(second_json["data"]["restarted_for_version_mismatch"], false);
    assert_eq!(second_json["data"]["version"], env!("CARGO_PKG_VERSION"));

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn daemon_restart_when_running() {
    daemon_stop_best_effort();

    // Start daemon first
    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    // Restart should stop and start
    let restart = uxc_command()
        .arg("daemon")
        .arg("restart")
        .output()
        .expect("daemon restart should run");
    assert!(restart.status.success());
    let restart_json: serde_json::Value =
        serde_json::from_slice(&restart.stdout).expect("valid json");
    assert_eq!(restart_json["ok"], true);
    assert_eq!(restart_json["kind"], "daemon_restart_result");
    assert_eq!(restart_json["data"]["stopped"], true);
    assert_eq!(restart_json["data"]["started_now"], true);
    assert!(restart_json["data"]["socket"].as_str().is_some());

    // Verify daemon is running after restart
    let status = uxc_command()
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(status_json["data"]["running"], true);

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn daemon_restart_when_not_running() {
    daemon_stop_best_effort();

    // Restart when daemon is not running should just start it
    let restart = uxc_command()
        .arg("daemon")
        .arg("restart")
        .output()
        .expect("daemon restart should run");
    assert!(restart.status.success());
    let restart_json: serde_json::Value =
        serde_json::from_slice(&restart.stdout).expect("valid json");
    assert_eq!(restart_json["ok"], true);
    assert_eq!(restart_json["kind"], "daemon_restart_result");
    assert_eq!(restart_json["data"]["stopped"], false);
    assert_eq!(restart_json["data"]["started_now"], true);
    assert!(restart_json["data"]["socket"].as_str().is_some());

    // Verify daemon is running after restart
    let status = uxc_command()
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(status_json["data"]["running"], true);

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn daemon_restart_help_shows_restart_subcommand_help() {
    let help = uxc_command()
        .arg("daemon")
        .arg("restart")
        .arg("-h")
        .output()
        .expect("daemon restart help should run");
    assert!(help.status.success());

    let help_json: serde_json::Value = serde_json::from_slice(&help.stdout).expect("valid json");
    assert_eq!(help_json["ok"], true);
    assert_eq!(help_json["kind"], "subcommand_help");
    assert_eq!(help_json["data"]["path"], "uxc daemon restart");

    // Check help content contains restart-specific information
    let data = &help_json["data"];
    assert!(data["about"].as_str().is_some());
    assert!(data["usage"].as_str().is_some());
    assert_eq!(data["usage"], "uxc daemon restart");
}

#[test]
#[serial]
fn daemon_restart_text_output_renders_correctly() {
    daemon_stop_best_effort();

    // Test restart when daemon is not running
    let restart = uxc_command()
        .arg("daemon")
        .arg("restart")
        .arg("--text")
        .output()
        .expect("daemon restart --text should run");
    assert!(restart.status.success());

    let stdout = String::from_utf8_lossy(&restart.stdout);
    assert!(stdout.contains("Daemon was not running."));
    assert!(stdout.contains("Daemon started."));
    assert!(stdout.contains("Socket:"));

    daemon_stop_best_effort();
}
