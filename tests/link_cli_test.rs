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

fn link_script_path(link_dir: &std::path::Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        return link_dir.join(format!("{}.cmd", name));
    }
    #[cfg(not(windows))]
    {
        link_dir.join(name)
    }
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
    let script_path = link_script_path(&link_dir, "petcli");

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
    #[cfg(unix)]
    assert!(
        script.contains("UXC_LINK_NAME='petcli' exec uxc 'petstore3.swagger.io/api/v3' \"$@\""),
        "script should contain bound host invocation"
    );
    #[cfg(windows)]
    assert!(
        script.contains("uxc \"petstore3.swagger.io/api/v3\" %*"),
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
    let script_path = link_script_path(&link_dir, "petcli");

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
    #[cfg(unix)]
    assert!(
        script.contains("UXC_LINK_NAME='petcli' exec uxc 'countries.trevorblades.com' \"$@\""),
        "script should be overwritten with latest host"
    );
    #[cfg(windows)]
    assert!(
        script.contains("uxc \"countries.trevorblades.com\" %*"),
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

#[cfg(unix)]
#[test]
fn link_shortcut_is_runnable_and_forwards_args() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let link_dir = temp_dir.path().join("bin");
    let script_path = link_script_path(&link_dir, "petcli");
    let fake_uxc_path = link_dir.join("uxc");

    let create = uxc_command()
        .arg("link")
        .arg("petcli")
        .arg("petstore3.swagger.io/api/v3")
        .arg("--dir")
        .arg(&link_dir)
        .output()
        .expect("uxc link should run");
    assert!(create.status.success(), "link creation should succeed");

    fs::write(&fake_uxc_path, "#!/usr/bin/env sh\nprintf '%s\\n' \"$@\"\n")
        .expect("fake uxc should be written");
    let mut perms = fs::metadata(&fake_uxc_path)
        .expect("fake uxc metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_uxc_path, perms).expect("fake uxc should be executable");

    let output = Command::new(&script_path)
        .env("PATH", prepend_path(&link_dir))
        .arg("describe")
        .arg("get:/pet/{petId}")
        .output()
        .expect("shortcut should run");

    assert!(
        output.status.success(),
        "shortcut invocation should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("petstore3.swagger.io/api/v3"),
        "bound host should be passed as first argument"
    );
    assert!(
        stdout.contains("describe") && stdout.contains("get:/pet/{petId}"),
        "user arguments should be forwarded"
    );
}
