//! CLI tests for basic-auth credentials whose secret lives in password-class
//! fields instead of the main secret (issue #447).

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

struct AuthFiles {
    _temp_dir: TempDir,
    credentials_file: PathBuf,
    bindings_file: PathBuf,
}

impl AuthFiles {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        Self {
            credentials_file: temp_dir.path().join("credentials.json"),
            bindings_file: temp_dir.path().join("auth_bindings.json"),
            _temp_dir: temp_dir,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_uxc"));
        cmd.env("UXC_CREDENTIALS_FILE", &self.credentials_file);
        cmd.env("UXC_AUTH_BINDINGS_FILE", &self.bindings_file);
        cmd
    }
}

fn run(cmd: &mut Command) -> Value {
    let output = cmd.output().expect("uxc command should run");
    serde_json::from_slice(&output.stdout).expect("stdout should be a JSON envelope")
}

#[test]
fn credential_set_basic_accepts_password_field_without_main_secret() {
    let files = AuthFiles::new();

    let envelope = run(files.command().args([
        "auth",
        "credential",
        "set",
        "email-basic",
        "--auth-type",
        "basic",
        "--field",
        "username=literal:user@example.com",
        "--field",
        "password=literal:app-password",
    ]));

    assert_eq!(envelope["ok"], true, "set should succeed: {}", envelope);
    assert_eq!(envelope["kind"], "auth_set_result");
    assert_eq!(envelope["data"]["api_key_masked"], "");
    let hint = envelope["data"]["secret_hint"]
        .as_str()
        .expect("set result should carry a secret hint");
    assert!(
        hint.contains("password/app_password field"),
        "hint: {}",
        hint
    );

    let info = run(files
        .command()
        .args(["auth", "credential", "info", "email-basic"]));
    assert_eq!(info["ok"], true);
    assert_eq!(info["kind"], "auth_info");
    assert_eq!(info["data"]["api_key_masked"], "");
    assert!(info["data"]["secret_hint"]
        .as_str()
        .expect("info should carry a secret hint")
        .contains("password/app_password field"));
}

#[test]
fn credential_set_basic_without_any_secret_fails_fast() {
    let files = AuthFiles::new();

    let output = files
        .command()
        .args([
            "auth",
            "credential",
            "set",
            "email-basic",
            "--auth-type",
            "basic",
            "--field",
            "username=literal:user@example.com",
        ])
        .output()
        .expect("uxc command should run");

    assert!(
        !output.status.success(),
        "set without any secret should fail"
    );
    let envelope: Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be a JSON envelope");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENT");
    assert!(envelope["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("password/app_password field"));
}

#[test]
fn credential_set_basic_accepts_secret_field() {
    let files = AuthFiles::new();

    let envelope = run(files.command().args([
        "auth",
        "credential",
        "set",
        "email-basic",
        "--auth-type",
        "basic",
        "--field",
        "secret=literal:app-password",
    ]));

    assert_eq!(envelope["ok"], true, "set should succeed: {}", envelope);
    assert_ne!(envelope["data"]["api_key_masked"], "");
    assert_eq!(envelope["data"]["secret_hint"], Value::Null);
}
