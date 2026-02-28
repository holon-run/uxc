use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

#[test]
fn cache_clear_normalizes_shorthand_url() {
    let temp_home = TempDir::new().expect("temp home should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_uxc"))
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .arg("cache")
        .arg("clear")
        .arg("mcp.notion.com/mcp")
        .output()
        .expect("cache clear should run");

    assert!(
        output.status.success(),
        "cache clear should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json = parse_stdout_json(&output);
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "cache_clear_result");
    assert_eq!(json["data"]["scope"], "url");
    assert_eq!(json["data"]["url"], "https://mcp.notion.com/mcp");
}
