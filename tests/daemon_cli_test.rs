mod common;

use assert_cmd::Command;
use fs2::FileExt;
use serial_test::serial;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::time::{Duration, Instant};

use common::{uxc_binary, uxc_command_with_home};

#[allow(deprecated)]
fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort_with_home(home: &Path) {
    let _ = uxc_command_with_home(home)
        .arg("daemon")
        .arg("stop")
        .output();
}

fn daemon_runtime_dir(home: &Path) -> PathBuf {
    home.join("runtime").join("uxc")
}

fn daemon_socket_path(home: &Path) -> PathBuf {
    daemon_runtime_dir(home).join("uxc.sock")
}

fn daemon_lock_path(home: &Path) -> PathBuf {
    daemon_runtime_dir(home).join("daemon.lock")
}

fn spawn_daemon_serve(home: &Path) -> Child {
    let runtime_dir = home.join("runtime");
    fs::create_dir_all(&runtime_dir).expect("runtime dir should exist");
    StdCommand::new(uxc_binary())
        .arg("daemon")
        .arg("_serve")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon _serve should spawn")
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("path did not appear: {}", path.display());
}

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait should succeed").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("child did not exit in time");
}

#[test]
#[serial]
fn daemon_start_status_stop_lifecycle() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let status = uxc_command_with_home(temp_home.path())
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
    assert!(json["data"]["managed_sources"].is_number());
    assert!(json["data"]["managed_sources_running"].is_number());
    assert!(json["data"]["managed_streams"].is_number());

    let stop = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("stop")
        .output()
        .expect("daemon stop should run");
    assert!(stop.status.success());

    // Stop path should wait for daemon to become unreachable.
    let status_after_stop = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status after stop should run");
    assert!(status_after_stop.status.success());
    let json_after_stop: serde_json::Value =
        serde_json::from_slice(&status_after_stop.stdout).expect("valid json");
    assert_eq!(json_after_stop["ok"], true);
    assert_eq!(json_after_stop["data"]["running"], false);
    assert_eq!(json_after_stop["data"]["managed_sources"], 0);
    assert_eq!(json_after_stop["data"]["managed_sources_running"], 0);
    assert_eq!(json_after_stop["data"]["managed_streams"], 0);
    assert!(json_after_stop["data"]["error"]["message"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
}

#[test]
#[serial]
fn endpoint_host_help_autostarts_daemon_and_sets_meta() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

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

    let output = uxc_command_with_home(temp_home.path())
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

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn daemon_start_reports_started_now_and_already_running() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let first = uxc_command_with_home(temp_home.path())
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

    let second = uxc_command_with_home(temp_home.path())
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

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn daemon_restart_when_running() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    // Start daemon first
    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    // Restart should stop and start
    let restart = uxc_command_with_home(temp_home.path())
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
    let status = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(status_json["data"]["running"], true);

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn daemon_restart_when_not_running() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    // Restart when daemon is not running should just start it
    let restart = uxc_command_with_home(temp_home.path())
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
    let status = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(status_json["data"]["running"], true);

    daemon_stop_best_effort_with_home(temp_home.path());
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
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    // Test restart when daemon is not running
    let restart = uxc_command_with_home(temp_home.path())
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

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn daemon_doctor_repairs_stale_socket_and_owner_metadata() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let daemon_dir = daemon_runtime_dir(temp_home.path());
    fs::create_dir_all(&daemon_dir).expect("daemon dir should exist");
    fs::write(daemon_socket_path(temp_home.path()), b"stale").expect("stale socket marker");
    fs::write(
        daemon_lock_path(temp_home.path()),
        r#"{"pid":999999,"version":"0.0.0","socket":"/tmp/old.sock","started_at_unix":1}"#,
    )
    .expect("stale owner metadata");

    let output = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("doctor")
        .output()
        .expect("daemon doctor should run");
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_doctor_result");
    assert_eq!(json["data"]["status"], "repaired");
    assert_eq!(json["data"]["repaired"], true);
    assert_eq!(json["data"]["socket_removed"], true);
    assert_eq!(json["data"]["owner_metadata_cleared"], true);
    assert!(!daemon_socket_path(temp_home.path()).exists());
    assert!(!daemon_lock_path(temp_home.path()).exists());
}

#[test]
#[serial]
fn daemon_doctor_refuses_repair_when_owner_lock_is_held() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let daemon_dir = daemon_runtime_dir(temp_home.path());
    fs::create_dir_all(&daemon_dir).expect("daemon dir should exist");
    fs::write(daemon_socket_path(temp_home.path()), b"stale").expect("stale socket marker");

    let mut lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(daemon_lock_path(temp_home.path()))
        .expect("daemon lock should open");
    lock_file
        .try_lock_exclusive()
        .expect("daemon lock should be acquirable");
    writeln!(
        lock_file,
        "{}",
        serde_json::json!({
            "pid": 999999_u32,
            "version": "0.0.0",
            "socket": daemon_socket_path(temp_home.path()).display().to_string(),
            "started_at_unix": 1_u64,
        })
    )
    .expect("owner metadata should write");
    lock_file.flush().expect("owner metadata should flush");

    let output = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("doctor")
        .output()
        .expect("daemon doctor should run");
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_doctor_result");
    assert_eq!(json["data"]["status"], "owner_held");
    assert_eq!(json["data"]["repaired"], false);
    assert_eq!(json["data"]["socket_removed"], false);
    assert_eq!(json["data"]["owner_metadata_cleared"], false);
    assert!(daemon_socket_path(temp_home.path()).exists());
    assert!(daemon_lock_path(temp_home.path()).exists());
}

#[test]
#[serial]
fn daemon_start_fails_closed_when_owner_lock_is_held() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let daemon_dir = daemon_runtime_dir(temp_home.path());
    fs::create_dir_all(&daemon_dir).expect("daemon dir should exist");
    let lock_path = daemon_lock_path(temp_home.path());
    let mut lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("daemon lock should open");
    lock_file
        .try_lock_exclusive()
        .expect("test process should hold daemon owner lock");
    writeln!(
        lock_file,
        "{{\"pid\":{},\"version\":\"{}\",\"socket\":\"{}\",\"started_at_unix\":1}}",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        daemon_socket_path(temp_home.path()).display()
    )
    .expect("owner metadata should write");
    lock_file.flush().expect("owner metadata should flush");

    let output = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(!output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "DAEMON_OWNER_UNREACHABLE");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Refusing to start a second daemon")));
}

#[test]
#[serial]
fn daemon_status_surfaces_owner_diagnostics_when_owner_is_unreachable() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let daemon_dir = daemon_runtime_dir(temp_home.path());
    fs::create_dir_all(&daemon_dir).expect("daemon dir should exist");
    let lock_path = daemon_lock_path(temp_home.path());
    let mut lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("daemon lock should open");
    lock_file
        .try_lock_exclusive()
        .expect("test process should hold daemon owner lock");
    writeln!(
        lock_file,
        "{{\"pid\":{},\"version\":\"{}\",\"socket\":\"{}\",\"started_at_unix\":1}}",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        daemon_socket_path(temp_home.path()).display()
    )
    .expect("owner metadata should write");
    lock_file.flush().expect("owner metadata should flush");

    let output = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["running"], false);
    assert_eq!(json["data"]["owner_lock_held"], true);
    assert_eq!(json["data"]["owner_pid"], std::process::id());
    assert_eq!(json["data"]["owner_pid_alive"], true);
    assert_eq!(json["data"]["error"]["code"], "DAEMON_OWNER_UNREACHABLE");
}

#[test]
#[serial]
fn daemon_stop_falls_back_to_owner_pid_when_socket_is_missing() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let mut child = spawn_daemon_serve(temp_home.path());
    wait_for_path(&daemon_socket_path(temp_home.path()));
    fs::remove_file(daemon_socket_path(temp_home.path())).expect("socket path should be removed");

    let output = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("stop")
        .output()
        .expect("daemon stop should run");
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["stopped"], true);

    wait_for_exit(&mut child);
    assert!(!daemon_socket_path(temp_home.path()).exists());

    let status_output = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("status")
        .output()
        .expect("daemon status should run");
    assert!(status_output.status.success());

    let status_json: serde_json::Value =
        serde_json::from_slice(&status_output.stdout).expect("valid status json");
    assert_eq!(status_json["ok"], true);
    assert_eq!(status_json["data"]["running"], false);
    assert_eq!(status_json["data"]["owner_lock_held"], false);

    if daemon_lock_path(temp_home.path()).exists() {
        let lock_contents = fs::read_to_string(daemon_lock_path(temp_home.path()))
            .expect("lock should be readable");
        assert!(lock_contents.trim().is_empty());
    }
}
