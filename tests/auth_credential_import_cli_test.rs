use serde_json::Value;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct AuthFiles {
    _temp_dir: TempDir,
    credentials_file: PathBuf,
    bindings_file: PathBuf,
    bin_dir: PathBuf,
}

impl AuthFiles {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let bin_dir = temp_dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir should be created");
        Self {
            credentials_file: temp_dir.path().join("credentials.json"),
            bindings_file: temp_dir.path().join("auth_bindings.json"),
            bin_dir,
            _temp_dir: temp_dir,
        }
    }
}

fn uxc_command(files: &AuthFiles) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_uxc"));
    cmd.env("UXC_CREDENTIALS_FILE", &files.credentials_file);
    cmd.env("UXC_AUTH_BINDINGS_FILE", &files.bindings_file);
    cmd
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn prepend_path(dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).expect("PATH should be joinable")
}

fn fake_executable_path(dir: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        dir.join(format!("{}.cmd", name))
    }
    #[cfg(not(windows))]
    {
        dir.join(name)
    }
}

fn write_fake_gh_success(dir: &Path, expected_hostname: &str, token: &str) {
    let path = fake_executable_path(dir, "gh");
    #[cfg(windows)]
    let script = format!(
        "@echo off\r\nif \"%1\"==\"auth\" if \"%2\"==\"token\" if \"%3\"==\"--hostname\" if \"%4\"==\"{expected_hostname}\" (\r\n  <nul set /p ={token}\r\n  exit /b 0\r\n)\r\necho unexpected gh invocation 1>&2\r\nexit /b 1\r\n"
    );
    #[cfg(not(windows))]
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"token\" ] && [ \"$3\" = \"--hostname\" ] && [ \"$4\" = \"{expected_hostname}\" ]; then\n  printf '{token}'\n  exit 0\nfi\necho 'unexpected gh invocation' >&2\nexit 1\n"
    );
    fs::write(&path, script).expect("fake gh should be written");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&path)
            .expect("fake gh metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("fake gh should be executable");
    }
}

fn write_fake_gh_failure(dir: &Path, message: &str) {
    let path = fake_executable_path(dir, "gh");
    #[cfg(windows)]
    let script = format!("@echo off\r\necho {message} 1>&2\r\nexit /b 1\r\n");
    #[cfg(not(windows))]
    let script = format!("#!/bin/sh\necho '{message}' >&2\nexit 1\n");
    fs::write(&path, script).expect("fake gh should be written");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&path)
            .expect("fake gh metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("fake gh should be executable");
    }
}

#[test]
fn auth_credential_import_from_gh_creates_bearer_credential_and_binding() {
    let files = AuthFiles::new();
    write_fake_gh_success(&files.bin_dir, "github.com", "gho_test_import_token");

    let output = uxc_command(&files)
        .env("PATH", prepend_path(&files.bin_dir))
        .arg("auth")
        .arg("credential")
        .arg("import")
        .arg("github")
        .arg("--from")
        .arg("gh")
        .output()
        .expect("credential import should run");

    assert!(
        output.status.success(),
        "credential import should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "auth_import_result");
    assert_eq!(json["data"]["source"], "gh");
    assert_eq!(json["data"]["hostname"], "github.com");
    assert_eq!(json["data"]["credential"]["name"], "github");
    assert_eq!(json["data"]["credential"]["auth_type"], "bearer");
    assert_eq!(json["data"]["binding_created"], true);
    assert_eq!(json["data"]["binding"]["id"], "github-api");
    assert_eq!(json["data"]["binding"]["host"], "api.github.com");
    assert_eq!(json["data"]["binding"]["path_prefix"], "/");
    assert_eq!(json["data"]["binding"]["scheme"], "https");
}

#[test]
fn auth_credential_import_from_gh_supports_skip_binding() {
    let files = AuthFiles::new();
    write_fake_gh_success(&files.bin_dir, "github.com", "gho_skip_binding_token");

    let output = uxc_command(&files)
        .env("PATH", prepend_path(&files.bin_dir))
        .arg("auth")
        .arg("credential")
        .arg("import")
        .arg("github")
        .arg("--from")
        .arg("gh")
        .arg("--skip-binding")
        .output()
        .expect("credential import should run");

    assert!(output.status.success(), "credential import should succeed");
    let json = parse_stdout_json(&output);
    assert_eq!(json["data"]["binding_created"], false);
    assert!(json["data"]["binding"].is_null());

    let bindings = fs::read_to_string(&files.bindings_file).unwrap_or_default();
    assert!(
        bindings.trim().is_empty(),
        "binding file should stay absent"
    );
}

#[test]
fn auth_credential_import_from_gh_fails_when_credential_exists_without_force() {
    let files = AuthFiles::new();
    write_fake_gh_success(&files.bin_dir, "github.com", "gho_existing_token");

    let first = uxc_command(&files)
        .env("PATH", prepend_path(&files.bin_dir))
        .args(["auth", "credential", "import", "github", "--from", "gh"])
        .output()
        .expect("first credential import should run");
    assert!(
        first.status.success(),
        "first credential import should succeed"
    );

    let second = uxc_command(&files)
        .env("PATH", prepend_path(&files.bin_dir))
        .args(["auth", "credential", "import", "github", "--from", "gh"])
        .output()
        .expect("second credential import should run");

    assert!(!second.status.success(), "second import should fail");
    let json = parse_stdout_json(&second);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn auth_credential_import_from_gh_supports_github_enterprise_binding_shape() {
    let files = AuthFiles::new();
    write_fake_gh_success(&files.bin_dir, "github.example.com", "gho_enterprise_token");

    let output = uxc_command(&files)
        .env("PATH", prepend_path(&files.bin_dir))
        .args([
            "auth",
            "credential",
            "import",
            "github-enterprise",
            "--from",
            "gh",
            "--hostname",
            "github.example.com",
        ])
        .output()
        .expect("credential import should run");

    assert!(output.status.success(), "credential import should succeed");
    let json = parse_stdout_json(&output);
    assert_eq!(json["data"]["hostname"], "github.example.com");
    assert_eq!(
        json["data"]["binding"]["id"],
        "github-enterprise-github-api"
    );
    assert_eq!(json["data"]["binding"]["host"], "github.example.com");
    assert_eq!(json["data"]["binding"]["path_prefix"], "/api");
}

#[test]
fn auth_credential_import_from_gh_reports_missing_cli() {
    let files = AuthFiles::new();

    let output = uxc_command(&files)
        .env("PATH", &files.bin_dir)
        .args(["auth", "credential", "import", "github", "--from", "gh"])
        .output()
        .expect("credential import should run");

    assert!(!output.status.success(), "import should fail without gh");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("'gh' CLI was not found"));
}

#[test]
fn auth_credential_import_from_gh_reports_gh_auth_failure() {
    let files = AuthFiles::new();
    write_fake_gh_failure(&files.bin_dir, "not logged in");

    let output = uxc_command(&files)
        .env("PATH", prepend_path(&files.bin_dir))
        .args(["auth", "credential", "import", "github", "--from", "gh"])
        .output()
        .expect("credential import should run");

    assert!(!output.status.success(), "import should fail");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("not logged in"));
}
