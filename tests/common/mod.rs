//! Common utilities for local E2E tests

use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Path to the uxc binary
pub fn uxc_binary() -> PathBuf {
    if std::path::Path::new("target/debug/uxc").exists() {
        PathBuf::from("target/debug/uxc")
    } else if std::path::Path::new("target/release/uxc").exists() {
        PathBuf::from("target/release/uxc")
    } else {
        // Build it first
        let status = Command::new("cargo")
            .args(["build", "--bin", "uxc"])
            .status()
            .expect("Failed to build uxc binary");
        assert!(status.success(), "Failed to build uxc binary");
        PathBuf::from("target/debug/uxc")
    }
}

/// Path to test server binaries
pub fn test_server_binary(name: &str) -> PathBuf {
    static BUILT_SERVERS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    let built = BUILT_SERVERS.get_or_init(|| Mutex::new(std::collections::HashSet::new()));

    // Build each protocol test server once per test process.
    {
        let mut guard = built.lock().expect("lock test server build cache");
        if !guard.contains(name) {
            let status = Command::new("cargo")
                .args([
                    "build",
                    "--bin",
                    &format!("uxc-test-{}-server", name),
                    "--features",
                    "test-server",
                ])
                .status()
                .unwrap_or_else(|_| panic!("Failed to build {} test server", name));
            assert!(status.success(), "Failed to build {} test server", name);
            guard.insert(name.to_string());
        }
    }

    let release_bin_path = format!("target/release/uxc-test-{}-server", name);
    if std::path::Path::new(&release_bin_path).exists() {
        return PathBuf::from(release_bin_path);
    }

    PathBuf::from(format!("target/debug/uxc-test-{}-server", name))
}

/// Handle to a running test server process
pub struct TestServerHandle {
    pub child: Child,
    pub addr: String,
}

impl Drop for TestServerHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start a test server
pub fn start_test_server(protocol: &str, scenario: &str) -> TestServerHandle {
    let bin = test_server_binary(protocol);

    // Use a unique addr file per server process to avoid cross-test interference.
    let temp_dir = std::env::temp_dir();
    let addr_file = temp_dir.join(format!(
        "{}-{}-{}.addr",
        protocol,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    ));
    let _ = fs::remove_file(&addr_file);

    let mut cmd = Command::new(bin);
    cmd.env("UXC_TEST_SERVER_DIR", &temp_dir);
    cmd.env("UXC_TEST_SERVER_ADDR_FILE", &addr_file);
    cmd.arg(scenario);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let child = cmd
        .spawn()
        .expect(&format!("Failed to start {} test server", protocol));

    // Wait a bit for server to start
    std::thread::sleep(Duration::from_millis(500));

    // Wait for address file to appear (server might be starting)
    let mut attempts = 0;
    while !addr_file.exists() && attempts < 10 {
        std::thread::sleep(Duration::from_millis(200));
        attempts += 1;
    }

    let addr = fs::read_to_string(&addr_file)
        .unwrap_or_else(|_| panic!("Failed to read server address from {:?}", addr_file))
        .trim()
        .to_string();
    let _ = fs::remove_file(&addr_file);

    tracing::info!("{} test server started at {}", protocol, addr);

    TestServerHandle { child, addr }
}

/// Run uxc command and check result
pub fn run_uxc(args: &[&str]) -> Result<String, String> {
    let uxc = uxc_binary();
    let output = Command::new(&uxc)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run uxc: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!(
            "uxc failed with exit code {:?}\nstdout: {}\nstderr: {}",
            output.status.code(),
            stdout,
            stderr
        ))
    }
}
