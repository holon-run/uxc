//! Common utilities for local E2E tests

use anyhow::Result;
use assert_cmd::Command as AssertCommand;
use std::ffi::OsStr;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn cargo_target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
}

fn current_profile_bin_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let deps_dir = exe.parent()?;
    deps_dir.parent().map(PathBuf::from)
}

fn sibling_bin(name: &str) -> Option<PathBuf> {
    let candidate = current_profile_bin_dir()?.join(name);
    candidate.exists().then_some(candidate)
}

/// Path to the uxc binary
pub fn uxc_binary() -> PathBuf {
    if let Some(path) = sibling_bin("uxc") {
        return path;
    }
    let target_dir = cargo_target_dir();
    let debug_bin = target_dir.join("debug").join("uxc");
    let release_bin = target_dir.join("release").join("uxc");
    if debug_bin.exists() {
        debug_bin
    } else if release_bin.exists() {
        release_bin
    } else {
        // Build it first
        let status = Command::new("cargo")
            .args(["build", "--bin", "uxc"])
            .status()
            .expect("Failed to build uxc binary");
        assert!(status.success(), "Failed to build uxc binary");
        debug_bin
    }
}

#[allow(deprecated)]
pub fn uxc_command() -> AssertCommand {
    AssertCommand::cargo_bin("uxc").expect("uxc binary should build")
}

pub fn uxc_command_with_home(home: &std::path::Path) -> AssertCommand {
    let runtime_dir = home.join("runtime");
    fs::create_dir_all(&runtime_dir).expect("runtime dir should be created");
    let mut cmd = uxc_command();
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd.env("XDG_RUNTIME_DIR", &runtime_dir);
    cmd
}

/// Path to test server binaries
pub fn test_server_binary(name: &str) -> PathBuf {
    static BUILT_SERVERS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    let built = BUILT_SERVERS.get_or_init(|| Mutex::new(std::collections::HashSet::new()));
    let bin_name = format!("uxc-test-{}-server", name);

    // Build each protocol test server once per test process.
    {
        let mut guard = built.lock().expect("lock test server build cache");
        if !guard.contains(name) {
            let status = Command::new("cargo")
                .args(["build", "--bin", &bin_name, "--features", "test-server"])
                .status()
                .unwrap_or_else(|_| panic!("Failed to build {} test server", name));
            assert!(status.success(), "Failed to build {} test server", name);
            guard.insert(name.to_string());
        }
    }

    if let Some(path) = sibling_bin(&bin_name) {
        return path;
    }

    let target_dir = cargo_target_dir();
    let release_bin_path = target_dir.join("release").join(&bin_name);
    if release_bin_path.exists() {
        return release_bin_path;
    }

    target_dir.join("debug").join(bin_name)
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
    let test_home = fresh_test_home_dir();
    run_uxc_in_home(args, test_home.path())
}

/// Run uxc command using an explicit HOME path.
/// Useful when a single test intentionally needs shared cache/daemon state across calls.
pub fn run_uxc_in_home(args: &[&str], test_home: &std::path::Path) -> Result<String, String> {
    let uxc = uxc_binary();
    let runtime_dir = test_home.join("runtime");
    fs::create_dir_all(&runtime_dir).expect("Failed to create test runtime dir");
    let output = Command::new(&uxc)
        .args(args)
        .env("HOME", test_home)
        .env("USERPROFILE", test_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
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

/// Temporary HOME for tests that may auto-start a daemon.
///
/// The daemon intentionally detaches from the invoking process, so dropping the
/// directory alone is not enough: tests must stop the daemon for this HOME before
/// the path disappears. This guard keeps cleanup tied to the HOME lifetime.
pub struct TestHome {
    path: PathBuf,
}

impl TestHome {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for TestHome {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

impl AsRef<OsStr> for TestHome {
    fn as_ref(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let runtime_dir = self.path.join("runtime");
        let _ = fs::create_dir_all(&runtime_dir);
        let _ = Command::new(uxc_binary())
            .args(["daemon", "stop"])
            .env("HOME", &self.path)
            .env("USERPROFILE", &self.path)
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .output();
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Create a fresh HOME directory for test isolation.
pub fn fresh_test_home_dir() -> TestHome {
    static TEST_HOME_SEQ: AtomicU64 = AtomicU64::new(0);
    let n = TEST_HOME_SEQ.fetch_add(1, Ordering::SeqCst);
    // Keep path short to avoid Unix socket path length limits for daemon socket.
    let dir = PathBuf::from(format!("/tmp/uxcth-{}-{}", std::process::id(), n));
    fs::create_dir_all(&dir).expect("Failed to create test HOME");
    TestHome { path: dir }
}
