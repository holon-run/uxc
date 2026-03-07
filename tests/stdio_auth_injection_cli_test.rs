use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

struct AuthFiles {
    temp_dir: TempDir,
    credentials_file: PathBuf,
    bindings_file: PathBuf,
}

impl AuthFiles {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        Self {
            credentials_file: temp_dir.path().join("credentials.json"),
            bindings_file: temp_dir.path().join("auth_bindings.json"),
            temp_dir,
        }
    }
}

fn uxc_command(files: &AuthFiles) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_uxc"));
    cmd.env("UXC_CREDENTIALS_FILE", &files.credentials_file);
    cmd.env("UXC_AUTH_BINDINGS_FILE", &files.bindings_file);
    cmd.env("HOME", files.temp_dir.path());
    cmd.env("USERPROFILE", files.temp_dir.path());
    cmd
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn create_literal_credential(files: &AuthFiles, id: &str, secret: &str) {
    let output = uxc_command(files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg(id)
        .arg("--secret")
        .arg(secret)
        .output()
        .expect("credential set should run");
    assert!(output.status.success(), "credential set should succeed");
}

fn write_env_mcp_server(files: &AuthFiles, env_name: &str, expected: &str) -> String {
    let script_path = files.temp_dir.path().join("env_mcp_server.py");
    let script = format!(
        "import json, os, sys\nexpected = {expected:?}\nenv_name = {env_name:?}\nif os.environ.get(env_name) != expected:\n    sys.exit(7)\nfor line in sys.stdin:\n    msg = json.loads(line)\n    method = msg.get('method')\n    if method == 'initialize':\n        print(json.dumps({{'jsonrpc':'2.0','id':msg['id'],'result':{{'protocolVersion':'2024-11-05','capabilities':{{'tools':{{}}}},'serverInfo':{{'name':'env-test','version':'1.0.0'}}}}}}), flush=True)\n    elif method == 'notifications/initialized':\n        continue\n    elif method == 'tools/list':\n        print(json.dumps({{'jsonrpc':'2.0','id':msg['id'],'result':{{'tools':[{{'name':'ping','description':'Ping','inputSchema':{{'type':'object'}}}}]}}}}), flush=True)\n    elif method == 'tools/call':\n        print(json.dumps({{'jsonrpc':'2.0','id':msg['id'],'result':{{'content':[{{'type':'text','text':'ok'}}]}}}}), flush=True)\n"
    );
    fs::write(&script_path, script).expect("script should be written");
    format!("python3 {}", script_path.display())
}

#[test]
fn direct_stdio_host_help_supports_injected_env() {
    let files = AuthFiles::new();
    create_literal_credential(&files, "thegraph", "test-token");
    let command = write_env_mcp_server(&files, "THEGRAPH_API_KEY", "test-token");

    let output = uxc_command(&files)
        .arg("--auth")
        .arg("thegraph")
        .arg("--inject-env")
        .arg("THEGRAPH_API_KEY={{secret}}")
        .arg(command)
        .arg("-h")
        .output()
        .expect("uxc host help should run");

    assert!(
        output.status.success(),
        "host help should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["protocol"], "mcp");
    assert_eq!(json["data"]["operations"][0]["operation_id"], "ping");
}

#[test]
fn direct_stdio_inject_env_requires_auth() {
    let files = AuthFiles::new();
    let command = write_env_mcp_server(&files, "THEGRAPH_API_KEY", "test-token");

    let output = uxc_command(&files)
        .arg("--inject-env")
        .arg("THEGRAPH_API_KEY={{secret}}")
        .arg(command)
        .arg("-h")
        .output()
        .expect("uxc host help should run");

    assert!(!output.status.success(), "host help should fail");
    let json = parse_stdout_json(&output);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("--inject-env requires a credential"));
}

#[test]
fn direct_non_stdio_inject_env_is_rejected() {
    let files = AuthFiles::new();
    create_literal_credential(&files, "demo", "secret");

    let output = uxc_command(&files)
        .arg("--auth")
        .arg("demo")
        .arg("--inject-env")
        .arg("TOKEN={{secret}}")
        .arg("petstore3.swagger.io/api/v3")
        .arg("-h")
        .output()
        .expect("uxc help should run");

    assert!(!output.status.success(), "command should fail");
    let json = parse_stdout_json(&output);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("only supported for stdio endpoints"));
}
