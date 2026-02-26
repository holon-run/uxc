//! Common utilities for local E2E tests

use anyhow::Result;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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
    // Build test server if needed
    let bin_path = format!("target/debug/uxc-test-{}-server", name);
    if !std::path::Path::new(&bin_path).exists() {
        let status = Command::new("cargo")
            .args([
                "build",
                "--bin",
                &format!("uxc-test-{}-server", name),
                "--features",
                "test-server",
            ])
            .status()
            .expect(&format!("Failed to build {} test server", name));
        assert!(status.success(), "Failed to build {} test server", name);
    }
    PathBuf::from(bin_path)
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

    // Set temp dir for server address files
    let temp_dir = std::env::temp_dir();
    let mut cmd = Command::new(bin);
    cmd.env("UXC_TEST_SERVER_DIR", &temp_dir);
    cmd.arg(scenario);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let child = cmd.spawn().expect(&format!("Failed to start {} test server", protocol));

    // Wait a bit for server to start
    std::thread::sleep(Duration::from_millis(500));

    // Read server address from file
    let addr_file = temp_dir.join(format!("{}.addr", protocol));

    // Wait for address file to appear (server might be starting)
    let mut attempts = 0;
    while !addr_file.exists() && attempts < 10 {
        std::thread::sleep(Duration::from_millis(200));
        attempts += 1;
    }

    let addr = std::fs::read_to_string(&addr_file)
        .unwrap_or_else(|_| panic!("Failed to read server address from {:?}", addr_file));

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
