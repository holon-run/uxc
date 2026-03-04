use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn uxc_with_home(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_uxc"));
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd
}

#[test]
fn cache_clear_normalizes_shorthand_url() {
    let temp_home = TempDir::new().expect("temp home should be created");
    let output = uxc_with_home(temp_home.path())
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

#[test]
fn cache_list_and_clear_by_key_flow() {
    let temp_home = TempDir::new().expect("temp home should be created");
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r##"{
  "openapi": "3.0.0",
  "info": { "title": "test", "version": "1.0.0" },
  "paths": {
    "/pets": {
      "get": {
        "summary": "list pets",
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"##,
        )
        .create();

    let prime = uxc_with_home(temp_home.path())
        .arg(server.url())
        .arg("get:/pets")
        .arg("-h")
        .output()
        .expect("prime cache should run");
    assert!(prime.status.success(), "prime cache should succeed");

    let list = uxc_with_home(temp_home.path())
        .arg("cache")
        .arg("list")
        .output()
        .expect("cache list should run");
    assert!(list.status.success(), "cache list should succeed");
    let list_json = parse_stdout_json(&list);
    assert_eq!(list_json["ok"], true);
    assert_eq!(list_json["kind"], "cache_list");
    assert!(
        list_json["data"]["count"].as_u64().unwrap_or(0) >= 1,
        "cache list should include at least one entry"
    );
    let key = list_json["data"]["entries"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["url"] == server.url())
                .and_then(|entry| entry["key"].as_str())
        })
        .expect("cache key for target endpoint should exist")
        .to_string();

    let clear = uxc_with_home(temp_home.path())
        .arg("cache")
        .arg("clear")
        .arg("--key")
        .arg(&key)
        .output()
        .expect("cache clear --key should run");
    assert!(clear.status.success(), "cache clear --key should succeed");
    let clear_json = parse_stdout_json(&clear);
    assert_eq!(clear_json["ok"], true);
    assert_eq!(clear_json["kind"], "cache_clear_result");
    assert_eq!(clear_json["data"]["scope"], "key");
    assert_eq!(clear_json["data"]["key"], key);

    let after = uxc_with_home(temp_home.path())
        .arg("cache")
        .arg("list")
        .output()
        .expect("cache list should run after clear");
    assert!(
        after.status.success(),
        "cache list after clear should succeed"
    );
    let after_json = parse_stdout_json(&after);
    let still_present = after_json["data"]["entries"]
        .as_array()
        .is_some_and(|entries| entries.iter().any(|entry| entry["url"] == server.url()));
    assert!(!still_present, "target endpoint cache should be removed");
}

#[test]
fn cache_clear_by_key_accepts_json_suffix() {
    let temp_home = TempDir::new().expect("temp home should be created");
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r##"{
  "openapi": "3.0.0",
  "info": { "title": "test", "version": "1.0.0" },
  "paths": {
    "/pets": {
      "get": {
        "summary": "list pets",
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"##,
        )
        .create();

    let prime = uxc_with_home(temp_home.path())
        .arg(server.url())
        .arg("get:/pets")
        .arg("-h")
        .output()
        .expect("prime cache should run");
    assert!(prime.status.success(), "prime cache should succeed");

    let list = uxc_with_home(temp_home.path())
        .arg("cache")
        .arg("list")
        .output()
        .expect("cache list should run");
    let list_json = parse_stdout_json(&list);
    let key = list_json["data"]["entries"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["url"] == server.url())
                .and_then(|entry| entry["key"].as_str())
        })
        .expect("cache key for target endpoint should exist")
        .to_string();

    let clear = uxc_with_home(temp_home.path())
        .arg("cache")
        .arg("clear")
        .arg("--key")
        .arg(format!("{key}.json"))
        .output()
        .expect("cache clear --key <key>.json should run");
    assert!(
        clear.status.success(),
        "cache clear by key file should succeed"
    );
    let clear_json = parse_stdout_json(&clear);
    assert_eq!(clear_json["data"]["scope"], "key");
    assert_eq!(clear_json["data"]["key"], key);
}
