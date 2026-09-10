//! Regression tests for issue #459: test runs must not leak
//! `uxc daemon _serve` processes.
//!
//! Covers the two daemon-side self-termination paths:
//! - the orphan watchdog: the daemon exits once its state dir (socket path)
//!   disappears, even though no client can address it anymore;
//! - the idle timeout: with `UXC_DAEMON_IDLE_TIMEOUT_SECS` set, the daemon
//!   shuts itself down after the window passes with no client connections,
//!   which also bounds leaks from test runs that are killed mid-run and never
//!   run their teardown.

mod common;

use common::{fresh_test_home_dir, uxc_command_with_home, DAEMON_IDLE_TIMEOUT_ENV};
use serial_test::serial;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

fn daemon_lock_path(home: &Path) -> std::path::PathBuf {
    home.join(".uxc").join("daemon").join("daemon.lock")
}

fn read_owner_pid(home: &Path) -> u32 {
    let raw = fs::read_to_string(daemon_lock_path(home))
        .expect("daemon.lock should exist after daemon start");
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("daemon.lock should contain JSON metadata");
    value["pid"]
        .as_u64()
        .expect("daemon.lock should record the owner pid") as u32
}

/// Whether `pid` is still a live (non-zombie) process.
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(_) => return false,
    };
    // The second field (process state) follows the comm name in parentheses.
    match stat.rsplit(')').next() {
        Some(rest) => !rest.trim_start().starts_with('Z'),
        None => true,
    }
}

#[cfg(not(target_os = "linux"))]
fn pid_alive(pid: u32) -> bool {
    use std::process::Command;
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
    panic!(
        "daemon pid {} was still alive after {:?}; it leaked",
        pid, timeout
    );
}

fn start_daemon_and_read_pid(home: &Path) -> u32 {
    let output = uxc_command_with_home(home)
        .arg("daemon")
        .arg("start")
        .output()
        .expect("uxc daemon start should run");
    assert!(
        output.status.success(),
        "daemon start should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    read_owner_pid(home)
}

#[test]
#[serial]
fn daemon_exits_when_its_state_dir_is_deleted() {
    let home = fresh_test_home_dir();
    let pid = start_daemon_and_read_pid(home.path());
    assert!(pid_alive(pid), "daemon should be running after start");

    // Simulate the leak scenario: the test HOME is removed without stopping
    // the daemon first. The socket and daemon.lock vanish with it, so no
    // client can address or kill the daemon by metadata anymore.
    fs::remove_dir_all(home.path()).expect("test HOME should be removable");

    // The orphan watchdog checks every DAEMON_WATCHDOG_TICK_MS.
    wait_for_pid_exit(pid, Duration::from_secs(15));
}

#[test]
#[serial]
fn daemon_self_terminates_after_idle_timeout() {
    let home = fresh_test_home_dir();
    let output = uxc_command_with_home(home.path())
        .env(DAEMON_IDLE_TIMEOUT_ENV, "2")
        .arg("daemon")
        .arg("start")
        .output()
        .expect("uxc daemon start should run");
    assert!(
        output.status.success(),
        "daemon start should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let pid = read_owner_pid(home.path());
    assert!(pid_alive(pid), "daemon should be running after start");

    // No client connections are made after start; the daemon must shut
    // itself down once the 2s idle window passes.
    wait_for_pid_exit(pid, Duration::from_secs(15));
}
