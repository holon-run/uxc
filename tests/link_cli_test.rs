use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn uxc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uxc"))
}

fn prepend_path(dir: &PathBuf) -> std::ffi::OsString {
    let mut paths = vec![dir.clone()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).expect("PATH should be joinable")
}

#[test]
fn link_create_outputs_json_default() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");

    let output = uxc_command()
        .env("PATH", prepend_path(&link_dir))
        .arg("link")
        .arg("petcli")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("uxc link should run");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "link_create_result");
    assert_eq!(json["protocol"], "cli");
    assert_eq!(json["data"]["name"], "petcli");
    assert_eq!(json["data"]["host"], "petstore3.swagger.io/api/v3");
    assert_eq!(json["data"]["dir_in_path"], true);
}

#[test]
fn link_create_writes_executable_script() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");
    let script_path = link_dir.join("petcli");

    let output = uxc_command()
        .arg("link")
        .arg("petcli")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("uxc link should run");
    assert!(output.status.success(), "command should succeed");

    assert!(script_path.exists(), "script should be created");
    let script = fs::read_to_string(&script_path).expect("script should be readable");
    assert!(
        script.contains("exec uxc 'petstore3.swagger.io/api/v3' \"$@\""),
        "script should contain bound host invocation"
    );

    #[cfg(unix)]
    {
        let mode = fs::metadata(&script_path)
            .expect("metadata should be readable")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "script should be executable");
    }
}

#[test]
fn link_create_refuses_overwrite_without_force() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");

    let first = uxc_command()
        .arg("link")
        .arg("petcli")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("initial create should run");
    assert!(first.status.success(), "initial create should succeed");

    let second = uxc_command()
        .arg("link")
        .arg("petcli")
        .arg("countries.trevorblades.com")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("second create should run");

    assert!(!second.status.success(), "second create should fail");
    let json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn link_create_overwrites_with_force() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");
    let script_path = link_dir.join("petcli");

    let first = uxc_command()
        .arg("link")
        .arg("petcli")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("initial create should run");
    assert!(first.status.success(), "initial create should succeed");

    let second = uxc_command()
        .arg("link")
        .arg("petcli")
        .arg("countries.trevorblades.com")
        .arg("--dir")
        .arg(&link_dir)
        .arg("--force")
        .output()
        .expect("overwrite create should run");
    assert!(second.status.success(), "overwrite create should succeed");

    let script = fs::read_to_string(&script_path).expect("script should be readable");
    assert!(
        script.contains("exec uxc 'countries.trevorblades.com' \"$@\""),
        "script should be overwritten with latest host"
    );

    let json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["data"]["overwritten"], true);
}

#[test]
fn link_create_rejects_invalid_name() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");

    let output = uxc_command()
        .arg("link")
        .arg("bad/name")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("uxc link should run");

    assert!(!output.status.success(), "command should fail");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn link_create_supports_text_output() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");

    let output = uxc_command()
        .arg("--text")
        .arg("link")
        .arg("petcli")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("uxc link should run");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created shortcut 'petcli' -> petstore3.swagger.io/api/v3"));
    assert!(stdout.contains("Path:"));
}
