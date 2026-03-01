use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

struct AuthFiles {
    _temp_dir: TempDir,
    credentials_file: std::path::PathBuf,
    bindings_file: std::path::PathBuf,
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

fn create_test_credential(files: &AuthFiles, id: &str) {
    let output = uxc_command(files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg(id)
        .arg("--auth-type")
        .arg("bearer")
        .arg("--secret")
        .arg("test-token")
        .output()
        .expect("credential set should run");

    assert!(
        output.status.success(),
        "credential set should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn auth_binding_lifecycle_outputs_json_envelopes() {
    let files = AuthFiles::new();
    create_test_credential(&files, "deepwiki");

    let add_output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("add")
        .arg("--id")
        .arg("deepwiki-mcp")
        .arg("--host")
        .arg("mcp.deepwiki.com")
        .arg("--path-prefix")
        .arg("/mcp")
        .arg("--scheme")
        .arg("https")
        .arg("--credential")
        .arg("deepwiki")
        .arg("--priority")
        .arg("100")
        .output()
        .expect("binding add should run");
    assert!(add_output.status.success(), "binding add should succeed");

    let add_json = parse_stdout_json(&add_output);
    assert_eq!(add_json["ok"], true);
    assert_eq!(add_json["kind"], "auth_binding_set_result");
    assert_eq!(add_json["data"]["id"], "deepwiki-mcp");
    assert_eq!(add_json["data"]["credential"], "deepwiki");

    let list_output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("list")
        .output()
        .expect("binding list should run");
    assert!(list_output.status.success(), "binding list should succeed");

    let list_json = parse_stdout_json(&list_output);
    assert_eq!(list_json["ok"], true);
    assert_eq!(list_json["kind"], "auth_binding_list");
    assert_eq!(list_json["data"]["count"], 1);

    let match_output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("match")
        .arg("mcp.deepwiki.com/mcp")
        .output()
        .expect("binding match should run");
    assert!(
        match_output.status.success(),
        "binding match should succeed"
    );

    let match_json = parse_stdout_json(&match_output);
    assert_eq!(match_json["ok"], true);
    assert_eq!(match_json["kind"], "auth_binding_match");
    assert_eq!(
        match_json["data"]["endpoint"],
        "https://mcp.deepwiki.com/mcp"
    );
    assert_eq!(match_json["data"]["matched"], true);
    assert_eq!(match_json["data"]["binding"]["id"], "deepwiki-mcp");

    let remove_output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("remove")
        .arg("deepwiki-mcp")
        .output()
        .expect("binding remove should run");
    assert!(
        remove_output.status.success(),
        "binding remove should succeed"
    );

    let remove_json = parse_stdout_json(&remove_output);
    assert_eq!(remove_json["ok"], true);
    assert_eq!(remove_json["kind"], "auth_binding_remove_result");
    assert_eq!(remove_json["data"]["binding_id"], "deepwiki-mcp");
}

#[test]
fn auth_binding_add_fails_for_unknown_credential() {
    let files = AuthFiles::new();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("add")
        .arg("--id")
        .arg("no-cred")
        .arg("--host")
        .arg("api.example.com")
        .arg("--credential")
        .arg("missing-credential")
        .output()
        .expect("binding add should run");

    assert!(!output.status.success(), "binding add should fail");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn auth_binding_add_fails_for_duplicate_binding_id() {
    let files = AuthFiles::new();
    create_test_credential(&files, "dup");

    let first = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("add")
        .arg("--id")
        .arg("dup-binding")
        .arg("--host")
        .arg("api.example.com")
        .arg("--credential")
        .arg("dup")
        .output()
        .expect("first add should run");
    assert!(first.status.success(), "first add should succeed");

    let second = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("add")
        .arg("--id")
        .arg("dup-binding")
        .arg("--host")
        .arg("api.example.com")
        .arg("--credential")
        .arg("dup")
        .output()
        .expect("second add should run");

    assert!(!second.status.success(), "second add should fail");
    let json = parse_stdout_json(&second);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn auth_binding_match_fails_for_invalid_endpoint_url() {
    let files = AuthFiles::new();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("match")
        .arg("not-a-valid-url")
        .output()
        .expect("binding match should run");

    assert!(!output.status.success(), "binding match should fail");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn auth_binding_set_and_remove_have_text_output() {
    let files = AuthFiles::new();
    create_test_credential(&files, "txt");

    let add_output = uxc_command(&files)
        .arg("--text")
        .arg("auth")
        .arg("binding")
        .arg("add")
        .arg("--id")
        .arg("txt-binding")
        .arg("--host")
        .arg("api.example.com")
        .arg("--credential")
        .arg("txt")
        .output()
        .expect("binding add should run");
    assert!(add_output.status.success(), "binding add should succeed");
    let add_stdout = String::from_utf8_lossy(&add_output.stdout);
    assert!(add_stdout.contains("Created binding 'txt-binding'"));

    let remove_output = uxc_command(&files)
        .arg("--text")
        .arg("auth")
        .arg("binding")
        .arg("remove")
        .arg("txt-binding")
        .output()
        .expect("binding remove should run");
    assert!(
        remove_output.status.success(),
        "binding remove should succeed"
    );
    let remove_stdout = String::from_utf8_lossy(&remove_output.stdout);
    assert!(remove_stdout.contains("Removed binding 'txt-binding'."));
}

#[cfg(unix)]
#[test]
fn auth_bindings_file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let files = AuthFiles::new();
    create_test_credential(&files, "perm");

    let add_output = uxc_command(&files)
        .arg("auth")
        .arg("binding")
        .arg("add")
        .arg("--id")
        .arg("perm-binding")
        .arg("--host")
        .arg("api.example.com")
        .arg("--credential")
        .arg("perm")
        .output()
        .expect("binding add should run");
    assert!(add_output.status.success(), "binding add should succeed");

    let mode = fs::metadata(&files.bindings_file)
        .expect("bindings file metadata should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "bindings file should be mode 0600");
}

#[test]
fn auth_credential_set_preserves_existing_type_and_description() {
    let files = AuthFiles::new();

    let first = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("preserve")
        .arg("--auth-type")
        .arg("api_key")
        .arg("--secret")
        .arg("first-secret")
        .arg("--description")
        .arg("keep-me")
        .output()
        .expect("first credential set should run");
    assert!(first.status.success(), "first set should succeed");

    let second = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("preserve")
        .arg("--secret-env")
        .arg("PRESERVE_TOKEN")
        .output()
        .expect("second credential set should run");
    assert!(second.status.success(), "second set should succeed");

    let info = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("info")
        .arg("preserve")
        .output()
        .expect("credential info should run");
    assert!(info.status.success(), "info should succeed");
    let json = parse_stdout_json(&info);
    assert_eq!(json["data"]["auth_type"], "api_key");
    assert_eq!(json["data"]["description"], "keep-me");
    assert_eq!(json["data"]["secret_source"]["kind"], "env");
}

#[test]
fn auth_credential_set_supports_secret_op_source() {
    let files = AuthFiles::new();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("op-source")
        .arg("--secret-op")
        .arg("op://Engineering/demo/token")
        .output()
        .expect("credential set should run");
    assert!(output.status.success(), "credential set should succeed");

    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["secret_source"]["kind"], "op");
}

#[test]
fn auth_credential_switch_from_oauth_requires_explicit_secret() {
    let files = AuthFiles::new();

    let create_oauth = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("oauth-switch")
        .arg("--auth-type")
        .arg("oauth")
        .output()
        .expect("oauth credential set should run");
    assert!(create_oauth.status.success(), "oauth set should succeed");

    let switch_without_secret = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("oauth-switch")
        .arg("--auth-type")
        .arg("bearer")
        .output()
        .expect("switch set should run");
    assert!(
        !switch_without_secret.status.success(),
        "switch without explicit secret should fail"
    );
    let json = parse_stdout_json(&switch_without_secret);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires an explicit secret source"),
        "error should explain oauth switch secret requirement"
    );
}
