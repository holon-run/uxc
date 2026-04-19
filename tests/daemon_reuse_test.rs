mod common;

use common::{start_test_server, test_server_binary, uxc_command, uxc_command_with_home};
use serial_test::serial;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::Duration;

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

fn daemon_stop_best_effort_with_home(home: &Path) {
    let _ = uxc_command_with_home(home)
        .arg("daemon")
        .arg("stop")
        .output();
}

fn mcp_http_endpoint(addr: &str) -> String {
    format!("http://{addr}/mcp")
}

#[cfg(unix)]
fn write_executable_script(path: &Path, body: &str) {
    fs::write(path, body).expect("script should be written");
    let mut perms = fs::metadata(path)
        .expect("script metadata should exist")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("script should be executable");
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
fn daemon_sessions_lists_live_stdio_session_diagnostics() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let call = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"inspect"}"#)
        .output()
        .expect("call should run");
    assert!(call.status.success());

    let sessions = uxc_command()
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(sessions.status.success());

    let json: serde_json::Value = serde_json::from_slice(&sessions.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_sessions");
    let sessions = json["data"]
        .as_array()
        .expect("session list should be an array");
    assert_eq!(sessions.len(), 1, "expected one stdio session");
    let session = &sessions[0];
    assert_eq!(session["transport"], "stdio");
    assert_eq!(session["protocol"], "mcp_stdio");
    assert_eq!(session["state"], "ready");
    assert_eq!(session["in_flight_requests"], 0);
    assert_eq!(session["reuse_eligible"], true);
    assert!(session["command_summary"]
        .as_str()
        .is_some_and(|value| value.contains("uxc-test-mcp-stdio-server")));
    assert!(session["idle_ttl_secs"].as_u64().is_some());
    assert!(session["idle_for_secs"].as_u64().is_some());
    assert!(session.get("expires_in_secs").is_some());
    assert!(session["daemon_exclusive"].as_array().is_some());
    assert!(session["recent_stderr"].as_array().is_some());

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn daemon_session_kill_removes_live_stdio_session_from_reuse_pool() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let first = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"seed"}"#)
        .output()
        .expect("first call should run");
    assert!(first.status.success());

    let sessions = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(sessions.status.success());
    let sessions_json: serde_json::Value =
        serde_json::from_slice(&sessions.stdout).expect("valid daemon sessions json");
    let entries = sessions_json["data"]
        .as_array()
        .expect("session list should be an array");
    assert_eq!(entries.len(), 1, "expected one stdio session");
    let session_key = entries[0]["session_key"]
        .as_str()
        .expect("session_key should be present")
        .to_string();
    let first_pid = entries[0]["child_pid"]
        .as_u64()
        .expect("child pid should be present");

    let kill = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("session")
        .arg("kill")
        .arg(&session_key)
        .output()
        .expect("daemon session kill should run");
    assert!(
        kill.status.success(),
        "kill should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&kill.stdout),
        String::from_utf8_lossy(&kill.stderr)
    );
    let kill_json: serde_json::Value =
        serde_json::from_slice(&kill.stdout).expect("valid daemon session kill json");
    assert_eq!(kill_json["ok"], true);
    assert_eq!(kill_json["kind"], "daemon_session_kill_result");
    assert_eq!(kill_json["data"]["session_key"], session_key);
    assert_eq!(kill_json["data"]["killed"], true);
    assert_eq!(
        kill_json["data"]["child_pid"]
            .as_u64()
            .expect("child pid should be returned"),
        first_pid
    );

    let sessions_after_kill = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions after kill should run");
    assert!(sessions_after_kill.status.success());
    let sessions_after_kill_json: serde_json::Value =
        serde_json::from_slice(&sessions_after_kill.stdout).expect("valid daemon sessions json");
    let entries_after_kill = sessions_after_kill_json["data"]
        .as_array()
        .expect("session list should be an array");
    assert!(
        entries_after_kill.is_empty(),
        "expected killed session to be removed from daemon sessions"
    );

    let second = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"after-kill"}"#)
        .output()
        .expect("second call should run");
    assert!(second.status.success());
    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second stdout should be valid JSON");
    assert_ne!(second_json["meta"]["daemon_session_reused"], true);

    let sessions_after_restart = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions after restart should run");
    assert!(sessions_after_restart.status.success());
    let sessions_after_restart_json: serde_json::Value =
        serde_json::from_slice(&sessions_after_restart.stdout).expect("valid daemon sessions json");
    let recreated_pid = sessions_after_restart_json["data"][0]["child_pid"]
        .as_u64()
        .expect("child pid should be present");
    assert_ne!(
        recreated_pid, first_pid,
        "expected a fresh MCP child process"
    );

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn daemon_sessions_surface_link_source_metadata() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let call = uxc_command()
        .env("UXC_LINK_NAME", "qmd-mcp-cli")
        .env("UXC_LINK_SKILL", "qmd-mcp-skill")
        .env(
            "UXC_LINK_SKILL_DOC",
            "https://uxc.holon.run/skills/qmd-mcp-skill/",
        )
        .env("UXC_LINK_SKILL_PATH", "skills/qmd-mcp-skill/SKILL.md")
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"inspect"}"#)
        .output()
        .expect("call should run");
    assert!(call.status.success());

    let sessions = uxc_command()
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(sessions.status.success());

    let json: serde_json::Value = serde_json::from_slice(&sessions.stdout).expect("valid json");
    let sessions = json["data"]
        .as_array()
        .expect("session list should be an array");
    assert_eq!(sessions.len(), 1, "expected one stdio session");
    let session = &sessions[0];
    assert_eq!(session["link_name"], "qmd-mcp-cli");
    assert_eq!(session["link_skill"], "qmd-mcp-skill");
    assert_eq!(
        session["link_skill_doc"],
        "https://uxc.holon.run/skills/qmd-mcp-skill/"
    );
    assert_eq!(session["link_skill_path"], "skills/qmd-mcp-skill/SKILL.md");

    daemon_stop_best_effort();
}

#[cfg(unix)]
#[test]
#[serial]
fn daemon_start_preserves_request_cwd_for_relative_stdio_endpoint_commands() {
    let home = common::fresh_test_home_dir();
    daemon_stop_best_effort_with_home(&home);

    let workdir = tempfile::tempdir().expect("tempdir should be created");
    let server = test_server_binary("mcp-stdio");
    let relative_server = workdir.path().join("mcp-stdio-rel");
    std::os::unix::fs::symlink(&server, &relative_server).expect("symlink should be created");
    let endpoint = "./mcp-stdio-rel ok";

    let start = uxc_command_with_home(&home)
        .current_dir(workdir.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let cold = uxc_command_with_home(&home)
        .current_dir(workdir.path())
        .arg(endpoint)
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

    let warm = uxc_command_with_home(&home)
        .current_dir(workdir.path())
        .arg(endpoint)
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
    assert_eq!(warm_json["meta"]["daemon_session_reused"], true);

    daemon_stop_best_effort_with_home(&home);
}

#[test]
#[serial]
fn daemon_sessions_expose_stateful_lifecycle_contract_when_idle_reap_is_deferred() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} lifecycle_stateful_hold", bin.display());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

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

    std::thread::sleep(Duration::from_millis(1500));

    let second = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"reuse"}"#)
        .output()
        .expect("second call should run");
    assert!(second.status.success());

    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second stdout should be valid JSON");
    assert_eq!(second_json["meta"]["daemon_session_reused"], true);

    let sessions = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(sessions.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&sessions.stdout).expect("valid daemon sessions json");
    let sessions = json["data"]
        .as_array()
        .expect("session list should be an array");
    assert_eq!(sessions.len(), 1, "expected one stdio session");
    assert_eq!(sessions[0]["lifecycle_contract"]["reap_policy"], "stateful");
    assert_eq!(
        sessions[0]["last_lifecycle_snapshot"]["auto_reap_allowed"],
        false
    );
    assert_eq!(
        sessions[0]["last_lifecycle_snapshot"]["retention_reason"],
        "interactive"
    );
    assert_eq!(
        sessions[0]["last_lifecycle_snapshot"]["retry_after_secs"],
        30
    );
    assert_eq!(sessions[0]["last_lifecycle_update_at_unix"], 1700000001u64);

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn stateful_session_with_auto_reap_allowed_is_reaped_before_reuse() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} lifecycle_stateful_allow", bin.display());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

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

    let first_sessions = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(first_sessions.status.success());
    let first_json: serde_json::Value =
        serde_json::from_slice(&first_sessions.stdout).expect("valid daemon sessions json");
    let first_pid = first_json["data"][0]["child_pid"]
        .as_u64()
        .expect("child pid should be present");

    std::thread::sleep(Duration::from_millis(1500));

    let second = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"seed-again"}"#)
        .output()
        .expect("second call should run");
    assert!(second.status.success());

    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second stdout should be valid JSON");
    assert!(
        second_json["meta"]["daemon_session_reused"] != true,
        "expected second call to use a fresh session after auto_reap_allowed=true"
    );

    let second_sessions = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(second_sessions.status.success());
    let second_sessions_json: serde_json::Value =
        serde_json::from_slice(&second_sessions.stdout).expect("valid daemon sessions json");
    let second_pid = second_sessions_json["data"][0]["child_pid"]
        .as_u64()
        .expect("child pid should be present");
    assert_ne!(first_pid, second_pid, "expected a new MCP child process");
    assert_eq!(
        second_sessions_json["data"][0]["lifecycle_contract"]["reap_policy"],
        "stateful"
    );

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn stateful_session_without_snapshot_is_kept_for_reuse() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} lifecycle_stateful_no_snapshot", bin.display());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

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

    std::thread::sleep(Duration::from_millis(1500));

    let second = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"reuse"}"#)
        .output()
        .expect("second call should run");
    assert!(second.status.success());

    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second stdout should be valid JSON");
    assert_eq!(second_json["meta"]["daemon_session_reused"], true);

    let sessions = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("sessions")
        .output()
        .expect("daemon sessions should run");
    assert!(sessions.status.success());
    let sessions_json: serde_json::Value =
        serde_json::from_slice(&sessions.stdout).expect("valid daemon sessions json");
    assert_eq!(
        sessions_json["data"][0]["lifecycle_contract"]["reap_policy"],
        "stateful"
    );
    assert!(sessions_json["data"][0]["last_lifecycle_snapshot"].is_null());

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn daemon_sessions_reports_active_state_for_busy_stdio_session() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} tool_call_timeout", bin.display());

    let start = uxc_command()
        .env("UXC_TEST_TIMEOUT_MS", "3000")
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let busy = std::thread::spawn(move || {
        uxc_command()
            .arg(&endpoint)
            .arg("echo")
            .arg("--input-json")
            .arg(r#"{"message":"busy"}"#)
            .output()
            .expect("busy call should run")
    });

    let mut found_active = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        let sessions = uxc_command()
            .arg("daemon")
            .arg("sessions")
            .output()
            .expect("daemon sessions should run");
        if !sessions.status.success() {
            continue;
        }
        let json: serde_json::Value =
            serde_json::from_slice(&sessions.stdout).expect("valid daemon sessions json");
        let entries = json["data"]
            .as_array()
            .expect("session list should be an array");
        if entries.iter().any(|session| {
            session["state"].as_str() == Some("active")
                && session["in_flight_requests"].as_u64().unwrap_or(0) >= 1
        }) {
            found_active = true;
            break;
        }
    }
    assert!(
        found_active,
        "expected daemon sessions to show an active stdio session"
    );

    let _ = busy.join().expect("busy thread should join");
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

#[test]
#[serial]
fn daemon_status_not_blocked_by_stuck_mcp_invoke() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} timeout", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let endpoint_first = endpoint.clone();
    let first = std::thread::spawn(move || {
        uxc_command()
            .env("UXC_TEST_TIMEOUT_MS", "4000")
            .arg(&endpoint_first)
            .arg("echo")
            .arg("--input-json")
            .arg(r#"{"message":"first"}"#)
            .output()
            .expect("first timeout call should run")
    });

    std::thread::sleep(Duration::from_millis(200));

    let endpoint_second = endpoint.clone();
    let second = std::thread::spawn(move || {
        uxc_command()
            .env("UXC_TEST_TIMEOUT_MS", "4000")
            .arg(&endpoint_second)
            .arg("echo")
            .arg("--input-json")
            .arg(r#"{"message":"second"}"#)
            .output()
            .expect("second timeout call should run")
    });

    std::thread::sleep(Duration::from_millis(200));

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = uxc_command()
            .arg("daemon")
            .arg("status")
            .output()
            .expect("daemon status should run");
        let _ = tx.send(out);
    });

    let status = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("daemon status should not block behind stuck mcp invoke");
    assert!(status.status.success());
    let json: serde_json::Value = serde_json::from_slice(&status.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "daemon_status");
    assert_eq!(json["data"]["running"], true);

    let _first_output = first.join().expect("first timeout thread panicked");
    // The timeout calls are expected to succeed (they timeout after 4s)
    // We don't assert on status.success() because the test is about
    // daemon status being responsive, not about the timeout calls themselves
    let _second_output = second.join().expect("second timeout thread panicked");

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn mcp_stdio_execute_does_not_relist_tools_on_reused_session() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} tools_list_fail_after_first", bin.display());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let first = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"first"}"#)
        .output()
        .expect("first call should run");
    assert!(
        first.status.success(),
        "first call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let second = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"second"}"#)
        .output()
        .expect("second call should run");
    assert!(
        second.status.success(),
        "second call should succeed even when tools/list would fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&second.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "second");
    assert_eq!(json["meta"]["daemon_session_reused"], true);

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_http_host_help_autostarts_daemon_and_sends_initialized_ack() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let server = start_test_server("mcp-http", "requires_initialized_ack");
    let endpoint = mcp_http_endpoint(&server.addr);

    let help = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("-h")
        .output()
        .expect("host help should run");
    assert!(
        help.status.success(),
        "daemon-autostarted host help should succeed once initialized ack is sent\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&help.stdout).expect("help stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["meta"]["daemon_used"], true);
    assert_eq!(json["meta"]["daemon_autostarted"], true);

    let ops: Vec<&str> = json["data"]["operations"]
        .as_array()
        .expect("operations array in host_help")
        .iter()
        .filter_map(|v| v["operation_id"].as_str())
        .collect();
    assert!(
        ops.contains(&"echo"),
        "expected echo operation, got {ops:?}"
    );

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_http_daemon_session_sends_initialized_ack_before_first_request() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let server = start_test_server("mcp-http", "requires_initialized_ack");
    let endpoint = mcp_http_endpoint(&server.addr);

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let cold = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"cold ack"}"#)
        .output()
        .expect("cold call should run");
    assert!(
        cold.status.success(),
        "cold daemon-backed HTTP MCP call should succeed once initialized ack is sent\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cold.stdout),
        String::from_utf8_lossy(&cold.stderr)
    );
    let cold_json: serde_json::Value =
        serde_json::from_slice(&cold.stdout).expect("cold stdout should be valid JSON");
    assert_eq!(cold_json["ok"], true);
    assert_eq!(cold_json["protocol"], "mcp");
    assert_eq!(cold_json["meta"]["daemon_used"], true);
    assert_eq!(cold_json["data"]["content"][0]["text"], "cold ack");

    let warm = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"warm ack"}"#)
        .output()
        .expect("warm call should run");
    assert!(
        warm.status.success(),
        "warm daemon-backed HTTP MCP call should also succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&warm.stdout),
        String::from_utf8_lossy(&warm.stderr)
    );
    let warm_json: serde_json::Value =
        serde_json::from_slice(&warm.stdout).expect("warm stdout should be valid JSON");
    assert_eq!(warm_json["ok"], true);
    assert_eq!(warm_json["protocol"], "mcp");
    assert_eq!(warm_json["data"]["content"][0]["text"], "warm ack");
    assert_eq!(warm_json["meta"]["daemon_session_reused"], true);

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_stdio_execute_uses_live_session_tool_catalog_for_arg_coercion() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let scripts_dir = temp_home.path().join("scripts");
    fs::create_dir_all(&scripts_dir).expect("scripts dir should exist");
    let log_path = temp_home.path().join("mcp-live-session.log");
    let script_path = scripts_dir.join("mcp_live_schema.py");

    write_executable_script(
        &script_path,
        r#"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

log_path = Path(sys.argv[1])
with log_path.open("a", encoding="utf-8") as f:
    f.write("start\n")

for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "live-schema", "version": "1.0.0"}
            }
        }), flush=True)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [{
                    "name": "measure",
                    "description": "Require integer count",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"count": {"type": "integer"}},
                        "required": ["count"]
                    }
                }]
            }
        }), flush=True)
    elif method == "tools/call":
        args = req.get("params", {}).get("arguments") or {}
        count = args.get("count")
        if not isinstance(count, int):
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32602, "message": "count must be integer"}
            }), flush=True)
            continue
        starts = log_path.read_text(encoding="utf-8").count("start\n")
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "content": [{"type": "text", "text": f"count={count};starts={starts}"}],
                "structuredContent": {"count": count, "starts": starts}
            }
        }), flush=True)
    else:
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": "Method not found"}
        }), flush=True)
"#,
    );

    let endpoint = format!("{} {}", script_path.display(), log_path.display());

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let cold = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("measure")
        .arg("--input-json")
        .arg(r#"{"count":"7"}"#)
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
    assert_eq!(cold_json["data"]["structuredContent"]["count"], 7);
    assert_eq!(cold_json["data"]["structuredContent"]["starts"], 1);

    let warm = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("measure")
        .arg("--input-json")
        .arg(r#"{"count":"9"}"#)
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
    assert_eq!(warm_json["data"]["structuredContent"]["count"], 9);
    assert_eq!(warm_json["data"]["structuredContent"]["starts"], 1);
    assert_eq!(warm_json["meta"]["daemon_session_reused"], true);

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_stdio_execute_refreshes_live_tool_catalog_after_tools_list_changed() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let scripts_dir = temp_home.path().join("scripts");
    fs::create_dir_all(&scripts_dir).expect("scripts dir should exist");
    let script_path = scripts_dir.join("mcp_dynamic_schema.py");

    write_executable_script(
        &script_path,
        r#"#!/usr/bin/env python3
import json
import sys

dynamic = False

for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": True}},
                "serverInfo": {"name": "dynamic-schema", "version": "1.0.0"}
            }
        }), flush=True)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        tools = [{
            "name": "navigate",
            "description": "Switch toolset",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }
        }]
        if dynamic:
            tools.append({
                "name": "render",
                "description": "Require integer frames",
                "inputSchema": {
                    "type": "object",
                    "properties": {"frames": {"type": "integer"}},
                    "required": ["frames"]
                }
            })
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"tools": tools}
        }), flush=True)
    elif method == "tools/call":
        name = req.get("params", {}).get("name")
        args = req.get("params", {}).get("arguments") or {}
        if name == "navigate":
            dynamic = True
            print(json.dumps({
                "jsonrpc": "2.0",
                "method": "notifications/tools/list_changed"
            }), flush=True)
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {"content": [{"type": "text", "text": "navigated"}]}
            }), flush=True)
        elif name == "render":
            frames = args.get("frames")
            if not isinstance(frames, int):
                print(json.dumps({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": -32602, "message": "frames must be integer"}
                }), flush=True)
            else:
                print(json.dumps({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{"type": "text", "text": f"frames={frames}"}],
                        "structuredContent": {"frames": frames}
                    }
                }), flush=True)
        else:
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32601, "message": "tool not found"}
            }), flush=True)
    else:
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": "Method not found"}
        }), flush=True)
"#,
    );

    let endpoint = script_path.display().to_string();

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let navigate = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("navigate")
        .arg("--input-json")
        .arg(r#"{"path":"/next"}"#)
        .output()
        .expect("navigate call should run");
    assert!(
        navigate.status.success(),
        "navigate call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&navigate.stdout),
        String::from_utf8_lossy(&navigate.stderr)
    );

    let render = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("render")
        .arg("--input-json")
        .arg(r#"{"frames":"12"}"#)
        .output()
        .expect("render call should run");
    assert!(
        render.status.success(),
        "render call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&render.stdout),
        String::from_utf8_lossy(&render.stderr)
    );
    let render_json: serde_json::Value =
        serde_json::from_slice(&render.stdout).expect("render stdout should be valid JSON");
    assert_eq!(render_json["ok"], true);
    assert_eq!(render_json["data"]["structuredContent"]["frames"], 12);
    assert_eq!(render_json["meta"]["daemon_session_reused"], true);

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_stdio_execute_falls_back_to_raw_args_when_live_tool_catalog_is_unavailable() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let scripts_dir = temp_home.path().join("scripts");
    fs::create_dir_all(&scripts_dir).expect("scripts dir should exist");
    let script_path = scripts_dir.join("mcp_no_tools_list.py");

    write_executable_script(
        &script_path,
        r#"#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "no-tools-list", "version": "1.0.0"}
            }
        }), flush=True)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": "Method not found"}
        }), flush=True)
    elif method == "tools/call":
        args = req.get("params", {}).get("arguments") or {}
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "content": [{"type": "text", "text": args.get("message", "")}],
                "structuredContent": {"message": args.get("message", "")}
            }
        }), flush=True)
    else:
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": "Method not found"}
        }), flush=True)
"#,
    );

    let endpoint = script_path.display().to_string();

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let call = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"fallback-ok"}"#)
        .output()
        .expect("call should run");
    assert!(
        call.status.success(),
        "call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&call.stdout),
        String::from_utf8_lossy(&call.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&call.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["structuredContent"]["message"], "fallback-ok");

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_stdio_execute_includes_structured_content_via_daemon() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} structured_content", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let call = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"daemon structured"}"#)
        .output()
        .expect("call should run");
    assert!(
        call.status.success(),
        "call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&call.stdout),
        String::from_utf8_lossy(&call.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&call.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["content"][0]["text"], "daemon structured");
    assert_eq!(
        json["data"]["structuredContent"]["message"],
        "daemon structured"
    );
    assert_eq!(json["data"]["structuredContent"]["source"], "mcp-stdio");

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn mcp_stdio_dynamic_toolset_refreshes_live_help_and_invalidates_disk_cache() {
    let temp_home = tempfile::Builder::new()
        .prefix("uxcth-")
        .tempdir_in("/tmp")
        .expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} dynamic_toolset", bin.display());

    let first_help = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("-h")
        .output()
        .expect("initial help should run");
    assert!(
        first_help.status.success(),
        "initial help should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first_help.stdout),
        String::from_utf8_lossy(&first_help.stderr)
    );

    let first_help_json: serde_json::Value =
        serde_json::from_slice(&first_help.stdout).expect("valid json");
    let first_ops = first_help_json["data"]["operations"]
        .as_array()
        .expect("operations should be an array");
    assert!(first_ops
        .iter()
        .any(|op| op["operation_id"] == "home_status"));
    assert!(!first_ops
        .iter()
        .any(|op| op["operation_id"] == "graph3d_render"));

    let cache_after_first_help = uxc_command_with_home(temp_home.path())
        .arg("cache")
        .arg("list")
        .output()
        .expect("cache list should run");
    assert!(cache_after_first_help.status.success());
    let cache_list_json: serde_json::Value =
        serde_json::from_slice(&cache_after_first_help.stdout).expect("valid json");
    let entries = cache_list_json["data"]["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(
        entries.len(),
        1,
        "expected initial host help to prime cache"
    );

    let navigate = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("navigate")
        .arg("path=/graph")
        .output()
        .expect("navigate should run");
    assert!(
        navigate.status.success(),
        "navigate should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&navigate.stdout),
        String::from_utf8_lossy(&navigate.stderr)
    );

    let second_help = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("-h")
        .output()
        .expect("updated help should run");
    assert!(
        second_help.status.success(),
        "updated help should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second_help.stdout),
        String::from_utf8_lossy(&second_help.stderr)
    );

    let second_help_json: serde_json::Value =
        serde_json::from_slice(&second_help.stdout).expect("valid json");
    let second_ops = second_help_json["data"]["operations"]
        .as_array()
        .expect("operations should be an array");
    assert!(second_ops
        .iter()
        .any(|op| op["operation_id"] == "graph3d_render"));
    assert!(!second_ops
        .iter()
        .any(|op| op["operation_id"] == "home_status"));

    let graph_help = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("graph3d_render")
        .arg("-h")
        .output()
        .expect("graph help should run");
    assert!(
        graph_help.status.success(),
        "graph help should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&graph_help.stdout),
        String::from_utf8_lossy(&graph_help.stderr)
    );
    let graph_help_json: serde_json::Value =
        serde_json::from_slice(&graph_help.stdout).expect("valid json");
    assert_eq!(graph_help_json["data"]["operation_id"], "graph3d_render");
    assert_eq!(
        graph_help_json["data"]["parameters"][0]["name"],
        "expression"
    );

    let cache_after_refresh = uxc_command_with_home(temp_home.path())
        .arg("cache")
        .arg("list")
        .output()
        .expect("cache list after refresh should run");
    assert!(cache_after_refresh.status.success());
    let cache_after_refresh_json: serde_json::Value =
        serde_json::from_slice(&cache_after_refresh.stdout).expect("valid json");
    let entries_after_refresh = cache_after_refresh_json["data"]["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(
        entries_after_refresh.len(),
        0,
        "expected dynamic refresh to invalidate disk cache without repopulating it"
    );

    daemon_stop_best_effort_with_home(temp_home.path());
}

#[test]
#[serial]
fn mcp_stdio_exclusive_key_allows_switching_endpoints_without_daemon_restart() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint_a = format!("{} ok --tag=a", bin.display());
    let endpoint_b = format!("{} ok --tag=b", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let first = uxc_command()
        .env("UXC_DAEMON_EXCLUSIVE", "~/.uxc/test-exclusive")
        .arg(&endpoint_a)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"a"}"#)
        .output()
        .expect("first call should run");
    assert!(
        first.status.success(),
        "first call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let second = uxc_command()
        .env("UXC_DAEMON_EXCLUSIVE", "~/.uxc/test-exclusive")
        .arg(&endpoint_b)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"b"}"#)
        .output()
        .expect("second call should run");
    assert!(
        second.status.success(),
        "second call should succeed after switching endpoints\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );

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

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn mcp_stdio_exclusive_key_refuses_to_evict_busy_session() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint_busy = format!("{} tool_call_timeout --tag=busy", bin.display());
    let endpoint_other = format!("{} ok --tag=other", bin.display());

    // Start daemon with the timeout env so the MCP stdio test server (spawned by daemon)
    // inherits it and sleeps for a predictable duration.
    let start = uxc_command()
        .env("UXC_TEST_TIMEOUT_MS", "4000")
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let busy = std::thread::spawn(move || {
        uxc_command()
            .env("UXC_DAEMON_EXCLUSIVE", "~/.uxc/test-exclusive")
            .arg(&endpoint_busy)
            .arg("echo")
            .arg("--input-json")
            .arg(r#"{"message":"busy"}"#)
            .output()
            .expect("busy call should run")
    });

    // Wait for session to be created so the exclusive owner is registered.
    let mut ready = false;
    for _ in 0..120 {
        std::thread::sleep(Duration::from_millis(100));
        let status = uxc_command()
            .arg("daemon")
            .arg("status")
            .output()
            .expect("daemon status should run");
        if !status.status.success() {
            continue;
        }
        let json: serde_json::Value = serde_json::from_slice(&status.stdout).expect("valid json");
        if json["data"]["running"].as_bool().unwrap_or(false)
            && json["data"]["mcp_stdio_sessions"].as_u64().unwrap_or(0) >= 1
        {
            ready = true;
            break;
        }
    }
    assert!(ready, "daemon did not become ready in time");

    let other = uxc_command()
        .env("UXC_DAEMON_EXCLUSIVE", "~/.uxc/test-exclusive")
        .arg(&endpoint_other)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"other"}"#)
        .output()
        .expect("other call should run");
    assert!(
        !other.status.success(),
        "expected other call to fail when busy session holds exclusive key\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&other.stdout),
        String::from_utf8_lossy(&other.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&other.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    let msg = json["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("daemon exclusive key"),
        "expected error message to mention daemon exclusive key, got: {}",
        msg
    );

    let _busy_out = busy.join().expect("busy thread should join");
    daemon_stop_best_effort();
}

#[test]
#[cfg(unix)]
#[serial]
fn mcp_stdio_exclusive_key_waits_for_evicted_child_exit() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    daemon_stop_best_effort_with_home(temp_home.path());

    let scripts_dir = temp_home.path().join("scripts");
    fs::create_dir_all(&scripts_dir).expect("scripts dir should exist");
    let lock_path = temp_home.path().join("profile.lock");
    let script_a = scripts_dir.join("mcp_a.sh");
    let script_b = scripts_dir.join("mcp_b.sh");

    write_executable_script(
        &script_a,
        r#"#!/usr/bin/env python3
import json
import sys
import time
from pathlib import Path

lock_path = Path(sys.argv[1])
lock_path.write_text("owner-a\n")

for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wait-a", "version": "1.0.0"}
            }
        }), flush=True)
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [{
                    "name": "echo",
                    "description": "Echo text back",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"message": {"type": "string"}},
                        "required": ["message"]
                    }
                }]
            }
        }), flush=True)
    elif method == "tools/call":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"content": [{"type": "text", "text": "a"}]}
        }), flush=True)

time.sleep(1)
lock_path.unlink(missing_ok=True)
"#,
    );

    write_executable_script(
        &script_b,
        r#"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

lock_path = Path(sys.argv[1])
if lock_path.exists():
    print("lock still present", file=sys.stderr, flush=True)
    sys.exit(23)

for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wait-b", "version": "1.0.0"}
            }
        }), flush=True)
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [{
                    "name": "echo",
                    "description": "Echo text back",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"message": {"type": "string"}},
                        "required": ["message"]
                    }
                }]
            }
        }), flush=True)
    elif method == "tools/call":
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"content": [{"type": "text", "text": "b"}]}
        }), flush=True)
"#,
    );

    let start = uxc_command_with_home(temp_home.path())
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let endpoint_a = format!("{} {}", script_a.display(), lock_path.display());
    let endpoint_b = format!("{} {}", script_b.display(), lock_path.display());

    let first = uxc_command_with_home(temp_home.path())
        .env(
            "UXC_DAEMON_EXCLUSIVE",
            lock_path.to_string_lossy().to_string(),
        )
        .arg(&endpoint_a)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"first"}"#)
        .output()
        .expect("first call should run");
    assert!(
        first.status.success(),
        "first call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        lock_path.exists(),
        "first session should hold the shared lock"
    );

    let second = uxc_command_with_home(temp_home.path())
        .env(
            "UXC_DAEMON_EXCLUSIVE",
            lock_path.to_string_lossy().to_string(),
        )
        .arg(&endpoint_b)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"second"}"#)
        .output()
        .expect("second call should run");
    assert!(
        second.status.success(),
        "second call should succeed after waiting for evicted child exit\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );

    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second stdout should be valid json");
    assert_eq!(second_json["ok"], true);
    assert_eq!(second_json["data"]["content"][0]["text"], "b");

    daemon_stop_best_effort_with_home(temp_home.path());
}
