use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

struct TestEnv {
    temp_dir: TempDir,
    credentials_file: PathBuf,
    bindings_file: PathBuf,
    link_dir: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let link_dir = temp_dir.path().join("bin");
        fs::create_dir_all(&link_dir).expect("bin dir should be created");
        Self {
            credentials_file: temp_dir.path().join("credentials.json"),
            bindings_file: temp_dir.path().join("auth_bindings.json"),
            link_dir,
            temp_dir,
        }
    }
}

fn uxc_command(env: &TestEnv) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_uxc"));
    cmd.env("UXC_CREDENTIALS_FILE", &env.credentials_file);
    cmd.env("UXC_AUTH_BINDINGS_FILE", &env.bindings_file);
    cmd.env("HOME", env.temp_dir.path());
    cmd.env("USERPROFILE", env.temp_dir.path());
    cmd
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

#[test]
fn config_import_mcp_dry_run_from_json_file() {
    let env = TestEnv::new();
    let source_path = env.temp_dir.path().join("mcp.json");
    let source = r#"{
  "mcpServers": {
    "deepwiki": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "https://mcp.deepwiki.com/mcp"],
      "env": {"DEEPWIKI_API_KEY": "${DEEPWIKI_API_KEY}"}
    },
    "thegraph": {
      "url": "https://subgraphs.mcp.thegraph.com/sse",
      "headers": {"Authorization": "Bearer test-token"}
    }
  }
}"#;
    fs::write(&source_path, source).expect("source file should be written");

    let output = uxc_command(&env)
        .arg("config")
        .arg("import")
        .arg("mcp")
        .arg("--from")
        .arg(&source_path)
        .arg("--dry-run")
        .output()
        .expect("config import should run");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "config_import_mcp_result");
    assert_eq!(json["data"]["dry_run"], true);
    assert_eq!(json["data"]["create_links"], false);
    assert_eq!(json["data"]["skip_create_links"], true);
    assert_eq!(json["data"]["discovered_count"], 2);
    assert_eq!(json["data"]["processed_count"], 2);
    assert_eq!(json["data"]["servers"][0]["original_name"], "deepwiki");
    assert_eq!(json["data"]["servers"][0]["credential"]["planned"], true);
    assert_eq!(json["data"]["servers"][1]["original_name"], "thegraph");
    assert_eq!(json["data"]["servers"][1]["binding"]["planned"], true);
}

#[test]
fn config_import_mcp_default_apply_is_idempotent() {
    let env = TestEnv::new();
    let source_path = env.temp_dir.path().join("mcp.json");
    let source = r#"{
  "mcpServers": {
    "deepwiki": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "https://mcp.deepwiki.com/mcp"],
      "env": {"DEEPWIKI_API_KEY": "${DEEPWIKI_API_KEY}"}
    },
    "thegraph": {
      "url": "https://subgraphs.mcp.thegraph.com/sse",
      "headers": {"Authorization": "Bearer test-token"}
    }
  }
}"#;
    fs::write(&source_path, source).expect("source file should be written");

    let first = uxc_command(&env)
        .arg("config")
        .arg("import")
        .arg("mcp")
        .arg("--from")
        .arg(&source_path)
        .arg("--link-dir")
        .arg(&env.link_dir)
        .output()
        .expect("first import should run");
    assert!(
        first.status.success(),
        "first run should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json = parse_stdout_json(&first);
    assert_eq!(first_json["data"]["dry_run"], false);
    assert_eq!(first_json["data"]["create_links"], true);
    assert_eq!(first_json["data"]["created_links"], 2);
    assert_eq!(first_json["data"]["created_credentials"], 2);
    assert_eq!(first_json["data"]["created_bindings"], 1);
    assert_eq!(first_json["data"]["failed_count"], 0);

    let match_output = uxc_command(&env)
        .arg("auth")
        .arg("binding")
        .arg("match")
        .arg("https://subgraphs.mcp.thegraph.com/sse")
        .output()
        .expect("binding match should run");
    assert!(
        match_output.status.success(),
        "binding match should succeed"
    );
    let match_json = parse_stdout_json(&match_output);
    assert_eq!(match_json["data"]["matched"], true);

    let second = uxc_command(&env)
        .arg("config")
        .arg("import")
        .arg("mcp")
        .arg("--from")
        .arg(&source_path)
        .arg("--link-dir")
        .arg(&env.link_dir)
        .output()
        .expect("second import should run");
    assert!(
        second.status.success(),
        "second run should return summarized result\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_json = parse_stdout_json(&second);
    assert_eq!(second_json["data"]["created_links"], 0);
    assert_eq!(second_json["data"]["created_credentials"], 0);
    assert_eq!(second_json["data"]["created_bindings"], 0);
    assert_eq!(second_json["data"]["failed_count"], 0);
}

#[test]
fn config_import_mcp_skip_create_links_still_imports_auth_assets() {
    let env = TestEnv::new();
    let source_path = env.temp_dir.path().join("mcp.json");
    let source = r#"{
  "mcpServers": {
    "thegraph": {
      "url": "https://subgraphs.mcp.thegraph.com/sse",
      "headers": {"Authorization": "Bearer test-token"}
    }
  }
}"#;
    fs::write(&source_path, source).expect("source file should be written");

    let output = uxc_command(&env)
        .arg("config")
        .arg("import")
        .arg("mcp")
        .arg("--from")
        .arg(&source_path)
        .arg("--skip-create-links")
        .arg("--link-dir")
        .arg(&env.link_dir)
        .output()
        .expect("config import should run");
    assert!(output.status.success(), "command should succeed");
    let json = parse_stdout_json(&output);
    assert_eq!(json["data"]["create_links"], false);
    assert_eq!(json["data"]["skip_create_links"], true);
    assert_eq!(json["data"]["created_links"], 0);
    assert_eq!(json["data"]["created_credentials"], 1);
    assert_eq!(json["data"]["created_bindings"], 1);
}

#[test]
fn config_import_mcp_rejects_removed_create_links_flag() {
    let env = TestEnv::new();
    let output = uxc_command(&env)
        .arg("config")
        .arg("import")
        .arg("mcp")
        .arg("--create-links")
        .output()
        .expect("command should run");
    assert!(!output.status.success(), "command should fail");
}

#[test]
fn config_import_mcp_supports_codex_toml_shape() {
    let env = TestEnv::new();
    let source_path = env.temp_dir.path().join("config.toml");
    let source = r#"
[mcp_servers.deepwiki]
url = "https://mcp.deepwiki.com/mcp"
bearer_token_env_var = "DEEPWIKI_API_KEY"

[mcp_servers.deepwiki.env_http_headers]
Authorization = "Bearer ${DEEPWIKI_API_KEY}"
"#;
    fs::write(&source_path, source).expect("source file should be written");

    let output = uxc_command(&env)
        .arg("config")
        .arg("import")
        .arg("mcp")
        .arg("--from")
        .arg(&source_path)
        .arg("--dry-run")
        .output()
        .expect("config import should run");

    assert!(output.status.success(), "command should succeed");
    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "config_import_mcp_result");
    assert_eq!(json["data"]["discovered_count"], 1);
    assert_eq!(json["data"]["servers"][0]["original_name"], "deepwiki");
    assert_eq!(json["data"]["servers"][0]["credential"]["planned"], true);
    assert_eq!(json["data"]["servers"][0]["binding"]["planned"], true);
}
