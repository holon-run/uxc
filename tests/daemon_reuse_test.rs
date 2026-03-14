mod common;

use assert_cmd::Command;
use common::test_server_binary;
use serial_test::serial;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::Duration;

#[allow(deprecated)]
fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

fn uxc_command_with_home(home: &Path) -> Command {
    let runtime_dir = home.join("runtime");
    fs::create_dir_all(&runtime_dir).expect("runtime dir should be created");
    let mut cmd = uxc_command();
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd.env("XDG_RUNTIME_DIR", &runtime_dir);
    cmd
}

fn daemon_stop_best_effort_with_home(home: &Path) {
    let _ = uxc_command_with_home(home)
        .arg("daemon")
        .arg("stop")
        .output();
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

    let first_output = first.join().expect("first timeout thread panicked");
    // The timeout calls are expected to succeed (they timeout after 4s)
    // We don't assert on status.success() because the test is about
    // daemon status being responsive, not about the timeout calls themselves
    let second_output = second.join().expect("second timeout thread panicked");

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn mcp_stdio_execute_does_not_relist_tools_on_reused_session() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} tools_list_fail_after_first", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let first = uxc_command()
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

    let second = uxc_command()
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

    daemon_stop_best_effort();
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
