mod common;

use serde_json::{json, Value};
use serial_test::serial;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn runtime_root(test_home: &Path) -> PathBuf {
    test_home.join("runtime")
}

fn daemon_socket_path(test_home: &Path) -> PathBuf {
    test_home.join(".uxc").join("daemon").join("uxc.sock")
}

fn read_frame(stream: &mut UnixStream) -> Value {
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let n = stream.read(&mut byte).expect("read frame header");
        assert!(n > 0, "unexpected EOF while reading header");
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8(header).expect("header utf8");
    let len = header_str
        .split("\r\n")
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .map(|v| v.trim().parse::<usize>().expect("content length"))
        .expect("content length header");
    let mut body = vec![0_u8; len];
    stream.read_exact(&mut body).expect("read frame body");
    serde_json::from_slice(&body).expect("json body")
}

fn write_frame(stream: &mut UnixStream, value: &Value) {
    let body = serde_json::to_vec(value).expect("serialize frame");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stream
        .write_all(header.as_bytes())
        .expect("write frame header");
    stream.write_all(&body).expect("write frame body");
    stream.flush().expect("flush frame");
}

struct FakeOldDaemon {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    done_rx: mpsc::Receiver<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl FakeOldDaemon {
    fn start(test_home: &Path, version: Option<&str>, shutdown_fails: bool) -> Self {
        let socket_path = daemon_socket_path(test_home);
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).expect("create fake daemon dir");
        }
        let _ = fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).expect("bind fake daemon socket");
        listener
            .set_nonblocking(true)
            .expect("set fake daemon listener nonblocking");
        let version = version.map(|v| v.to_string());
        let socket_path_for_thread = socket_path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let (done_tx, done_rx) = mpsc::channel();

        let join = thread::spawn(move || loop {
            if stop_for_thread.load(Ordering::SeqCst) {
                break;
            }
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(_) => break,
            };
            if stop_for_thread.load(Ordering::SeqCst) {
                break;
            }
            let req = read_frame(&mut stream);
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let method = req.get("method").and_then(Value::as_str).unwrap_or("");
            let response = match method {
                "daemon.status" => {
                    let mut result = json!({
                        "running": true,
                        "pid": 99999,
                        "socket": socket_path_for_thread.display().to_string(),
                        "started_at_unix": 1,
                        "request_count": 0,
                        "mcp_stdio_sessions": 0,
                        "mcp_http_sessions": 0,
                        "mcp_reuse_hits": 0
                    });
                    if let Some(version) = &version {
                        result["version"] = Value::String(version.clone());
                    }
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    })
                }
                "daemon.shutdown" if shutdown_fails => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": "shutdown refused" }
                }),
                "daemon.shutdown" => {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": true }
                    });
                    write_frame(&mut stream, &response);
                    let _ = done_tx.send(());
                    let _ = fs::remove_file(&socket_path_for_thread);
                    break;
                }
                _ => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Method not found: {}", method) }
                }),
            };
            write_frame(&mut stream, &response);
        });

        Self {
            socket_path,
            stop,
            done_rx,
            join: Some(join),
        }
    }

    fn wait_for_shutdown(&self) {
        self.done_rx.recv().expect("fake daemon shutdown signal");
    }
}

impl Drop for FakeOldDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        let _ = fs::remove_file(&self.socket_path);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[test]
#[serial]
fn endpoint_request_restarts_old_daemon_before_invoke() {
    let test_home = common::fresh_test_home_dir();
    let fake = FakeOldDaemon::start(&test_home, Some("0.8.0"), false);

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

    let stdout = common::run_uxc_in_home(&[&server.url(), "--no-cache", "-h"], &test_home)
        .expect("host help should succeed");
    fake.wait_for_shutdown();

    let json: Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["meta"]["daemon_used"], true);
    assert_eq!(json["meta"]["daemon_autostarted"], false);
    assert_eq!(json["meta"]["daemon_restarted_for_version_mismatch"], true);

    let status_stdout =
        common::run_uxc_in_home(&["daemon", "status"], &test_home).expect("status should succeed");
    let status_json: Value = serde_json::from_str(&status_stdout).expect("valid status json");
    assert_eq!(status_json["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status_json["data"]["version_mismatch"], false);

    let _ = common::run_uxc_in_home(&["daemon", "stop"], &test_home);
}

#[test]
#[serial]
fn daemon_status_reports_version_mismatch_without_restarting() {
    let test_home = common::fresh_test_home_dir();
    let _fake = FakeOldDaemon::start(&test_home, Some("0.8.0"), false);

    let stdout =
        common::run_uxc_in_home(&["daemon", "status"], &test_home).expect("status should succeed");
    let json: Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["running"], true);
    assert_eq!(json["data"]["version"], "0.8.0");
    assert_eq!(json["data"]["client_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["data"]["version_mismatch"], true);
}

#[test]
#[serial]
fn daemon_start_restarts_old_daemon_and_reports_previous_version() {
    let test_home = common::fresh_test_home_dir();
    let fake = FakeOldDaemon::start(&test_home, Some("0.8.0"), false);

    let stdout =
        common::run_uxc_in_home(&["daemon", "start"], &test_home).expect("start should succeed");
    fake.wait_for_shutdown();

    let json: Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["started_now"], true);
    assert_eq!(json["data"]["already_running"], false);
    assert_eq!(json["data"]["restarted_for_version_mismatch"], true);
    assert_eq!(json["data"]["previous_version"], "0.8.0");
    assert_eq!(json["data"]["version"], env!("CARGO_PKG_VERSION"));

    let _ = common::run_uxc_in_home(&["daemon", "stop"], &test_home);
}

#[test]
#[serial]
fn daemon_start_reports_error_when_version_mismatch_restart_fails() {
    let test_home = common::fresh_test_home_dir();
    let _fake = FakeOldDaemon::start(&test_home, Some("0.8.0"), true);

    let uxc = common::uxc_binary();
    let output = std::process::Command::new(&uxc)
        .args(["daemon", "start"])
        .env("HOME", &test_home)
        .env("USERPROFILE", &test_home)
        .env("XDG_RUNTIME_DIR", runtime_root(&test_home))
        .output()
        .expect("daemon start should run");
    assert!(!output.status.success(), "daemon start should fail");

    let json: Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "DAEMON_VERSION_MISMATCH");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("daemon=0.8.0"));
    assert!(message.contains(env!("CARGO_PKG_VERSION")));
    assert!(message.contains("uxc daemon restart"));
}
