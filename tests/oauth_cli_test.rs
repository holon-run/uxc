use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

struct AuthFiles {
    _temp_dir: TempDir,
    credentials_file: std::path::PathBuf,
    bindings_file: std::path::PathBuf,
    session_dir: std::path::PathBuf,
}

impl AuthFiles {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let session_dir = temp_dir.path().join("oauth_sessions");
        Self {
            credentials_file: temp_dir.path().join("credentials.json"),
            bindings_file: temp_dir.path().join("auth_bindings.json"),
            session_dir,
            _temp_dir: temp_dir,
        }
    }
}

fn uxc_command(files: &AuthFiles) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_uxc"));
    cmd.env("UXC_CREDENTIALS_FILE", &files.credentials_file);
    cmd.env("UXC_AUTH_BINDINGS_FILE", &files.bindings_file);
    cmd.env("UXC_OAUTH_SESSION_DIR", &files.session_dir);
    cmd
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn read_session_json(files: &AuthFiles, session_id: &str) -> Value {
    let path = files.session_dir.join(format!("{}.json", session_id));
    let contents = fs::read_to_string(path).expect("session file should exist");
    serde_json::from_str(&contents).expect("session file should be valid JSON")
}

#[test]
fn oauth_start_returns_authorization_url_and_session() {
    let files = AuthFiles::new();
    let server = mockito::Server::new();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .arg("--scope")
        .arg("read")
        .output()
        .expect("oauth start should run");

    assert!(output.status.success(), "oauth start should succeed");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "auth_oauth_start_result");
    let session_id = json["data"]["session_id"].as_str().expect("session id");
    let session_path = files.session_dir.join(format!("{}.json", session_id));
    assert!(session_path.exists(), "session file should exist");
    assert!(
        json["data"]["authorization_url"]
            .as_str()
            .unwrap_or_default()
            .contains("/authorize?"),
        "authorization URL should be returned"
    );
}

#[test]
fn oauth_complete_success_persists_profile_and_removes_session() {
    let files = AuthFiles::new();
    let mut server = mockito::Server::new();

    let start = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .output()
        .expect("oauth start should run");
    assert!(start.status.success(), "oauth start should succeed");
    let start_json = parse_stdout_json(&start);
    let session_id = start_json["data"]["session_id"].as_str().unwrap();
    let session_json = read_session_json(&files, session_id);
    let state = session_json["state"].as_str().unwrap();

    let _token_mock = server
        .mock("POST", "/token")
        .match_body(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("grant_type".into(), "authorization_code".into()),
            mockito::Matcher::UrlEncoded("code".into(), "abc123".into()),
            mockito::Matcher::UrlEncoded(
                "redirect_uri".into(),
                "http://127.0.0.1:11111/callback".into(),
            ),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"access_token":"access-1","token_type":"bearer","refresh_token":"refresh-1","expires_in":3600}"#)
        .create();

    let complete = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("complete")
        .arg("notion")
        .arg("--session-id")
        .arg(session_id)
        .arg("--authorization-response")
        .arg(format!(
            "http://127.0.0.1:11111/callback?code=abc123&state={}",
            state
        ))
        .output()
        .expect("oauth complete should run");

    assert!(complete.status.success(), "oauth complete should succeed");
    let json = parse_stdout_json(&complete);
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "auth_set_result");
    assert_eq!(json["data"]["oauth"]["flow"], "authorization_code");
    assert!(
        !files
            .session_dir
            .join(format!("{}.json", session_id))
            .exists(),
        "session should be deleted after successful completion"
    );
}

#[test]
fn oauth_complete_missing_session_returns_structured_error() {
    let files = AuthFiles::new();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("complete")
        .arg("notion")
        .arg("--session-id")
        .arg("missing-session")
        .arg("--authorization-response")
        .arg("abc123")
        .output()
        .expect("oauth complete should run");

    assert!(!output.status.success(), "oauth complete should fail");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "OAUTH_SESSION_NOT_FOUND");
}

#[test]
fn oauth_complete_expired_session_returns_structured_error() {
    let files = AuthFiles::new();
    let server = mockito::Server::new();

    let start = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .output()
        .expect("oauth start should run");
    assert!(start.status.success(), "oauth start should succeed");
    let start_json = parse_stdout_json(&start);
    let session_id = start_json["data"]["session_id"].as_str().unwrap();
    let path = files.session_dir.join(format!("{}.json", session_id));
    let mut session_json = read_session_json(&files, session_id);
    session_json["expires_at"] = serde_json::json!(1);
    fs::write(&path, serde_json::to_vec_pretty(&session_json).unwrap()).unwrap();

    let complete = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("complete")
        .arg("notion")
        .arg("--session-id")
        .arg(session_id)
        .arg("--authorization-response")
        .arg("abc123")
        .output()
        .expect("oauth complete should run");

    assert!(!complete.status.success(), "oauth complete should fail");
    let json = parse_stdout_json(&complete);
    assert_eq!(json["error"]["code"], "OAUTH_SESSION_EXPIRED");
    assert!(
        !path.exists(),
        "expired session should be deleted after failed completion"
    );
}

#[test]
fn oauth_complete_state_mismatch_deletes_session() {
    let files = AuthFiles::new();
    let server = mockito::Server::new();

    let start = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .output()
        .expect("oauth start should run");
    let start_json = parse_stdout_json(&start);
    let session_id = start_json["data"]["session_id"].as_str().unwrap();

    let complete = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("complete")
        .arg("notion")
        .arg("--session-id")
        .arg(session_id)
        .arg("--authorization-response")
        .arg("http://127.0.0.1:11111/callback?code=abc123&state=wrong")
        .output()
        .expect("oauth complete should run");

    assert!(!complete.status.success(), "oauth complete should fail");
    let json = parse_stdout_json(&complete);
    assert_eq!(json["error"]["code"], "OAUTH_TOKEN_EXCHANGE_FAILED");
    assert!(
        !files
            .session_dir
            .join(format!("{}.json", session_id))
            .exists(),
        "state mismatch should delete session"
    );
}

#[test]
fn oauth_complete_retryable_error_keeps_session() {
    let files = AuthFiles::new();
    let mut server = mockito::Server::new();

    let start = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .output()
        .expect("oauth start should run");
    let start_json = parse_stdout_json(&start);
    let session_id = start_json["data"]["session_id"].as_str().unwrap();

    let _token_mock = server.mock("POST", "/token").with_status(500).create();

    let complete = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("complete")
        .arg("notion")
        .arg("--session-id")
        .arg(session_id)
        .arg("--authorization-response")
        .arg("abc123")
        .output()
        .expect("oauth complete should run");

    assert!(!complete.status.success(), "oauth complete should fail");
    assert!(
        files
            .session_dir
            .join(format!("{}.json", session_id))
            .exists(),
        "retryable token exchange failures should keep session"
    );
}

#[test]
fn oauth_login_authorization_code_remains_supported() {
    let files = AuthFiles::new();
    let mut server = mockito::Server::new();

    let _token_mock = server
        .mock("POST", "/token")
        .match_body(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("grant_type".into(), "authorization_code".into()),
            mockito::Matcher::UrlEncoded("code".into(), "abc123".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"access_token":"access-1","token_type":"bearer","refresh_token":"refresh-1","expires_in":3600}"#)
        .create();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("login")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--flow")
        .arg("authorization_code")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .arg("--authorization-code")
        .arg("abc123")
        .output()
        .expect("oauth login should run");

    assert!(output.status.success(), "oauth login should succeed");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["oauth"]["flow"], "authorization_code");
}

#[test]
fn oauth_start_supports_dynamic_client_registration() {
    let files = AuthFiles::new();
    let mut server = mockito::Server::new();

    let _registration_mock = server
        .mock("POST", "/register")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(r#"{"client_id":"registered-client","client_secret":"registered-secret"}"#)
        .create();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .arg("--registration-endpoint")
        .arg(format!("{}/register", server.url()))
        .output()
        .expect("oauth start should run");

    assert!(output.status.success(), "oauth start should succeed");
    let json = parse_stdout_json(&output);
    assert!(
        json["data"]["authorization_url"]
            .as_str()
            .unwrap_or_default()
            .contains("registered-client"),
        "registered client id should be used in authorization URL"
    );
}

#[test]
fn oauth_start_text_output_includes_next_step() {
    let files = AuthFiles::new();
    let server = mockito::Server::new();

    let output = uxc_command(&files)
        .arg("--text")
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .output()
        .expect("oauth start should run");

    assert!(output.status.success(), "oauth start should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Session ID:"),
        "text output should include session id"
    );
    assert!(
        stdout.contains("uxc auth oauth complete notion"),
        "text output should include next step command"
    );
}

#[cfg(unix)]
#[test]
fn oauth_session_file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let files = AuthFiles::new();
    let server = mockito::Server::new();

    let output = uxc_command(&files)
        .arg("auth")
        .arg("oauth")
        .arg("start")
        .arg("notion")
        .arg("--endpoint")
        .arg("https://api.example.com/mcp")
        .arg("--redirect-uri")
        .arg("http://127.0.0.1:11111/callback")
        .arg("--client-id")
        .arg("client-1")
        .arg("--authorization-endpoint")
        .arg(format!("{}/authorize", server.url()))
        .arg("--token-endpoint")
        .arg(format!("{}/token", server.url()))
        .output()
        .expect("oauth start should run");
    assert!(output.status.success(), "oauth start should succeed");
    let json = parse_stdout_json(&output);
    let session_id = json["data"]["session_id"].as_str().unwrap();
    let session_path = files.session_dir.join(format!("{}.json", session_id));

    let mode = fs::metadata(session_path)
        .expect("session metadata should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "session file should be mode 0600");
}
