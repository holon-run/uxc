use assert_cmd::Command;
use serial_test::serial;

fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

#[test]
#[serial]
fn subscribe_start_help_shows_subcommand_help() {
    let output = uxc_command()
        .arg("subscribe")
        .arg("start")
        .arg("-h")
        .output()
        .expect("subscribe help should run");
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "subcommand_help");
    assert_eq!(json["data"]["path"], "uxc subscribe start");
}

#[test]
#[serial]
fn subscribe_rejects_schema_url_before_daemon_start() {
    let output = uxc_command()
        .arg("--schema-url")
        .arg("https://example.com/schema.json")
        .arg("subscribe")
        .arg("start")
        .arg("https://example.com/stream")
        .arg("--sink")
        .arg("file:/tmp/events.ndjson")
        .output()
        .expect("subscribe start should run");
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|msg| msg.contains("--schema-url is not supported")));
}
