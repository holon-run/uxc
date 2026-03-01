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

    let stop = uxc_command()
        .arg("daemon")
        .arg("stop")
        .output()
        .expect("daemon stop should run");
    assert!(stop.status.success());
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

    daemon_stop_best_effort();
}
