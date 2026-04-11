use assert_cmd::Command;
use serial_test::serial;

#[allow(deprecated)]
fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

#[test]
#[serial]
fn endpoint_codegen_schema_exports_runtime_contract_boundaries() {
    daemon_stop_best_effort();

    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r##"{
  "openapi": "3.0.0",
  "info": { "title": "pets", "version": "1.0.0" },
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

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("--codegen-schema")
        .output()
        .expect("codegen schema should run");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "codegen_host_schema");
    assert_eq!(json["protocol"], "openapi");
    assert_eq!(json["data"]["version"], "v1");
    assert_eq!(json["data"]["host"]["endpoint"], server.url());
    assert_eq!(
        json["data"]["runtime"]["invoke_options_schema"]["properties"]["daemon_idle_ttl"]["type"]
            [0],
        "integer"
    );
    assert_eq!(
        json["data"]["runtime"]["lifecycle_contract"]["idle_policy"]["latest_request_wins_ttl"],
        true
    );
    assert_eq!(
        json["data"]["runtime"]["lifecycle_contract"]["mcp_stdio_lifecycle"]["declaration_method"],
        "uxc/lifecycle_contract"
    );
    assert_eq!(
        json["data"]["runtime"]["artifact_contract"]["compaction_model"]["meta_fields"][4],
        "artifact_ref"
    );
    assert_eq!(json["data"]["operations"][0]["id"], "get:/pets");
    assert_eq!(json["data"]["operations"][0]["kind"], "execute");
    assert_eq!(json["data"]["operations"][0]["result_kind"], "call_result");
    assert_eq!(json["data"]["operations"][0]["subscribable"], false);

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn codegen_schema_rejects_operation_argument_combo() {
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r##"{
  "openapi": "3.0.0",
  "info": { "title": "pets", "version": "1.0.0" },
  "paths": { "/pets": { "get": { "responses": { "200": { "description": "ok" } } } } }
}"##,
        )
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--codegen-schema")
        .arg("get:/pets")
        .output()
        .expect("command should run");

    assert!(!output.status.success(), "command should fail");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|m| m.contains("--codegen-schema cannot be combined")));
}
