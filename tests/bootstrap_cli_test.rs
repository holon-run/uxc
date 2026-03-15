use serde_json::Value;
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

#[test]
fn bootstrap_set_info_refresh_and_remove_work_for_bearer_fields_credential() {
    let files = AuthFiles::new();
    let mut server = mockito::Server::new();

    let set_credential = uxc_command(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("feishu-tenant")
        .arg("--auth-type")
        .arg("bearer")
        .arg("--field")
        .arg("app_id=literal:app-1")
        .arg("--field")
        .arg("app_secret=literal:secret-1")
        .output()
        .expect("credential set should run");
    assert!(set_credential.status.success());

    let set_bootstrap = uxc_command(&files)
        .arg("auth")
        .arg("bootstrap")
        .arg("set")
        .arg("feishu-tenant")
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .arg("--header")
        .arg("Content-Type=application/json")
        .arg("--request-json")
        .arg(r#"{"app_id":"{{field:app_id}}","app_secret":"{{field:app_secret}}"}"#)
        .arg("--access-token-pointer")
        .arg("/tenant_access_token")
        .arg("--expires-in-pointer")
        .arg("/expire")
        .arg("--success-code-pointer")
        .arg("/code")
        .arg("--success-code-value")
        .arg("0")
        .output()
        .expect("bootstrap set should run");
    assert!(set_bootstrap.status.success());
    let set_json = parse_stdout_json(&set_bootstrap);
    assert_eq!(set_json["ok"], true);
    assert_eq!(set_json["kind"], "auth_bootstrap_set_result");
    assert_eq!(set_json["data"]["bootstrap"]["token_present"], false);

    let info = uxc_command(&files)
        .arg("auth")
        .arg("bootstrap")
        .arg("info")
        .arg("feishu-tenant")
        .output()
        .expect("bootstrap info should run");
    assert!(info.status.success());
    let info_json = parse_stdout_json(&info);
    assert_eq!(
        info_json["data"]["bootstrap"]["token_endpoint"],
        format!("{}/token", server.url())
    );

    let _token_mock = server
        .mock("POST", "/token")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"code":0,"expire":7200,"tenant_access_token":"tenant-token-1"}"#)
        .create();

    let refresh = uxc_command(&files)
        .arg("auth")
        .arg("bootstrap")
        .arg("refresh")
        .arg("feishu-tenant")
        .output()
        .expect("bootstrap refresh should run");
    assert!(refresh.status.success());
    let refresh_json = parse_stdout_json(&refresh);
    assert_eq!(refresh_json["ok"], true);
    assert_eq!(refresh_json["kind"], "auth_bootstrap_refresh_result");
    assert_eq!(refresh_json["data"]["refreshed"], true);
    assert_eq!(refresh_json["data"]["bootstrap"]["token_present"], true);

    let remove = uxc_command(&files)
        .arg("auth")
        .arg("bootstrap")
        .arg("remove")
        .arg("feishu-tenant")
        .output()
        .expect("bootstrap remove should run");
    assert!(remove.status.success());
    let remove_json = parse_stdout_json(&remove);
    assert_eq!(remove_json["ok"], true);
    assert_eq!(remove_json["kind"], "auth_bootstrap_remove_result");
    assert_eq!(remove_json["data"]["removed"], true);
}
