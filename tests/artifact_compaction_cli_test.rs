use assert_cmd::Command;
use serial_test::serial;
use std::fs;
use tempfile::TempDir;

#[allow(deprecated)]
fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort(home: &TempDir) {
    let runtime_dir = home.path().join("runtime");
    let _ = fs::create_dir_all(&runtime_dir);
    let _ = uxc_command()
        .arg("daemon")
        .arg("stop")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .output();
}

#[test]
#[serial]
fn call_result_large_payload_compacts_with_artifact_metadata() {
    let home = TempDir::new().expect("temp home");
    daemon_stop_best_effort(&home);

    let big_blob = "x".repeat(80_000);
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r##"{
  "openapi": "3.0.0",
  "info": { "title": "blob", "version": "1.0.0" },
  "paths": {
    "/blob": {
      "get": {
        "responses": {
          "200": { "description": "ok" }
        }
      }
    }
  }
}"##,
        )
        .create();
    let _blob = server
        .mock("GET", "/blob")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "blob": big_blob,
                "ok": true
            })
            .to_string(),
        )
        .create();

    let runtime_dir = home.path().join("runtime");
    let _ = fs::create_dir_all(&runtime_dir);
    let output = uxc_command()
        .arg(server.url())
        .arg("get:/blob")
        .arg("--no-cache")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .output()
        .expect("call should run");
    assert!(output.status.success(), "call should succeed");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "call_result");
    assert_eq!(json["meta"]["artifact_truncated"], true);
    assert_eq!(json["meta"]["artifact_kind"], "call_result");
    assert!(json["meta"]["artifact_bytes"]
        .as_u64()
        .is_some_and(|n| n > 64 * 1024));
    let artifact_path = json["meta"]["artifact_path"]
        .as_str()
        .expect("artifact path must exist");
    assert!(json["meta"]["artifact_sha256"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
    assert!(
        fs::metadata(artifact_path).is_ok(),
        "artifact file should exist"
    );

    let preview_blob = json["data"]["blob"]
        .as_str()
        .expect("preview blob should be string");
    assert!(preview_blob.len() < 80_000, "preview should be truncated");

    let artifact_raw = fs::read_to_string(artifact_path).expect("read artifact file");
    let artifact_json: serde_json::Value = serde_json::from_str(&artifact_raw).expect("json file");
    assert_eq!(artifact_json["ok"], true);
    assert_eq!(artifact_json["blob"].as_str().unwrap_or("").len(), 80_000);

    daemon_stop_best_effort(&home);
}

#[test]
#[serial]
fn host_help_large_payload_compacts_but_codegen_schema_stays_inline() {
    let home = TempDir::new().expect("temp home");
    daemon_stop_best_effort(&home);

    let mut paths = serde_json::Map::new();
    for i in 0..1600 {
        let path = format!("/items/{i}");
        paths.insert(
            path,
            serde_json::json!({
                "get": {
                    "summary": format!("Get item {i}"),
                    "responses": {
                        "200": { "description": "ok" }
                    }
                }
            }),
        );
    }
    let schema = serde_json::json!({
        "openapi": "3.0.0",
        "info": { "title": "huge", "version": "1.0.0" },
        "paths": paths
    });

    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(schema.to_string())
        .create();

    let runtime_dir = home.path().join("runtime");
    let _ = fs::create_dir_all(&runtime_dir);
    let host_help = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("-h")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .output()
        .expect("host help should run");
    assert!(host_help.status.success(), "host help should succeed");
    let host_help_json: serde_json::Value =
        serde_json::from_slice(&host_help.stdout).expect("valid json");
    assert_eq!(host_help_json["kind"], "host_help");
    assert_eq!(host_help_json["meta"]["artifact_truncated"], true);
    assert_eq!(host_help_json["meta"]["artifact_kind"], "host_help");
    assert!(host_help_json["meta"]["artifact_path"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));

    let codegen = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("--codegen-schema")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .output()
        .expect("codegen should run");
    assert!(codegen.status.success(), "codegen should succeed");
    let codegen_json: serde_json::Value = serde_json::from_slice(&codegen.stdout).expect("json");
    assert_eq!(codegen_json["kind"], "codegen_host_schema");
    assert!(codegen_json["meta"]["artifact_truncated"].is_null());
    assert!(codegen_json["meta"]["artifact_path"].is_null());
    assert!(codegen_json["data"]["operations"]
        .as_array()
        .is_some_and(|ops| ops.len() > 1000));

    daemon_stop_best_effort(&home);
}
