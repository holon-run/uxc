use assert_cmd::Command;
use serial_test::serial;
use tempfile::TempDir;

#[allow(deprecated)]
fn isolated_uxc_command() -> (TempDir, Command) {
    let temp = TempDir::new().expect("temp dir");
    let runtime_dir = temp.path().join("runtime");
    std::fs::create_dir_all(&runtime_dir).expect("runtime dir");
    let mut command = Command::cargo_bin("uxc").expect("uxc binary should build");
    command.env("HOME", temp.path());
    command.env("USERPROFILE", temp.path());
    command.env("XDG_RUNTIME_DIR", &runtime_dir);
    (temp, command)
}

#[test]
#[serial]
fn subscribe_start_help_shows_subcommand_help() {
    let (_temp, mut command) = isolated_uxc_command();
    let output = command
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
    assert!(json["data"]["usage"]
        .as_str()
        .is_some_and(|usage| usage.contains("--ephemeral")));
    assert!(json["data"]["usage"]
        .as_str()
        .is_some_and(|usage| usage.contains("discord-gateway")));
}

#[test]
#[serial]
fn subscribe_rejects_schema_url_before_daemon_start() {
    let (_temp, mut command) = isolated_uxc_command();
    let output = command
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

#[test]
#[serial]
fn subscribe_rejects_poll_config_without_poll_mode() {
    let (temp, mut command) = isolated_uxc_command();
    let sink = format!("file:{}", temp.path().join("events.ndjson").display());
    let output = command
        .arg("subscribe")
        .arg("start")
        .arg("https://example.com/stream")
        .arg("--sink")
        .arg(&sink)
        .arg("--poll-config")
        .arg(r#"{"interval_secs":5,"extract_items_pointer":"/items","checkpoint_strategy":{"type":"content_hash"}}"#)
        .output()
        .expect("subscribe start should run");
    assert!(!output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "EXECUTION_FAILED");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|msg| msg.contains("--poll-config is only valid with --mode poll")));
}

#[test]
#[serial]
fn subscribe_rejects_websocket_transport_with_operation_id() {
    let (temp, mut command) = isolated_uxc_command();
    let sink = format!("file:{}", temp.path().join("events.ndjson").display());
    let output = command
        .arg("subscribe")
        .arg("start")
        .arg("wss://example.com/feed")
        .arg("eth_subscribe")
        .arg("--transport")
        .arg("websocket")
        .arg("--sink")
        .arg(&sink)
        .output()
        .expect("subscribe start should run");
    assert!(!output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"].as_str().is_some_and(
        |msg| msg.contains("--transport websocket cannot be combined with an operation_id")
    ));
}

#[test]
#[serial]
fn subscribe_rejects_slack_socket_mode_transport_with_operation_id() {
    let (temp, mut command) = isolated_uxc_command();
    let sink = format!("file:{}", temp.path().join("events.ndjson").display());
    let output = command
        .arg("subscribe")
        .arg("start")
        .arg("https://slack.com/api")
        .arg("post:/chat.postMessage")
        .arg("--transport")
        .arg("slack-socket-mode")
        .arg("--sink")
        .arg(&sink)
        .output()
        .expect("subscribe start should run");
    assert!(!output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|msg| msg
            .contains("--transport slack-socket-mode cannot be combined with an operation_id")));
}

#[test]
#[serial]
fn subscribe_rejects_discord_gateway_transport_with_operation_id() {
    let (temp, mut command) = isolated_uxc_command();
    let sink = format!("file:{}", temp.path().join("events.ndjson").display());
    let output = command
        .arg("subscribe")
        .arg("start")
        .arg("https://discord.com/api/v10")
        .arg("get:/gateway")
        .arg("--transport")
        .arg("discord-gateway")
        .arg("--sink")
        .arg(&sink)
        .output()
        .expect("subscribe start should run");
    assert!(!output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|msg| msg
            .contains("--transport discord-gateway cannot be combined with an operation_id")));
}

#[test]
#[serial]
fn subscribe_accepts_discord_gateway_config_without_operation_id() {
    let (temp, mut command) = isolated_uxc_command();
    let sink = format!("file:{}", temp.path().join("events.ndjson").display());
    let output = command
        .arg("subscribe")
        .arg("start")
        .arg("https://discord.com/api/v10")
        .arg(r#"{"intents":37377,"device":"uxc-test"}"#)
        .arg("--transport")
        .arg("discord-gateway")
        .arg("--sink")
        .arg(&sink)
        .output()
        .expect("subscribe start should run");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "subscribe_start_result");
    assert_eq!(json["data"]["protocol"], "discord_gateway");
    assert_eq!(json["data"]["status"], "running");
}

#[test]
#[serial]
fn subscribe_rejects_websocket_options_without_transport() {
    let (temp, mut command) = isolated_uxc_command();
    let sink = format!("file:{}", temp.path().join("events.ndjson").display());
    let output = command
        .arg("subscribe")
        .arg("start")
        .arg("wss://example.com/feed")
        .arg("--subprotocol")
        .arg("market.v1")
        .arg("--sink")
        .arg(&sink)
        .output()
        .expect("subscribe start should run");
    assert!(!output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|msg| msg
            .contains("--subprotocol and --init-frame require explicit --transport websocket")));
}
