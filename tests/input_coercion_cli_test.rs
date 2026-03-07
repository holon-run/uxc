use assert_cmd::Command;
use mockito::{Matcher, Server};

fn uxc() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("uxc"))
}

fn openapi_schema() -> String {
    serde_json::json!({
        "openapi": "3.0.0",
        "info": { "title": "test", "version": "1.0.0" },
        "paths": {
            "/pets": {
                "post": {
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "count": { "type": "integer" },
                                        "enabled": { "type": "boolean" }
                                    },
                                    "required": ["count", "enabled"],
                                    "additionalProperties": false
                                }
                            }
                        }
                    },
                    "responses": { "200": { "description": "ok" } }
                }
            }
        }
    })
    .to_string()
}

#[test]
fn key_value_arguments_are_coerced_before_openapi_execution() {
    let mut server = Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(openapi_schema())
        .create();

    let _execute = server
        .mock("POST", "/pets")
        .match_header(
            "content-type",
            Matcher::Regex("application/json".to_string()),
        )
        .match_body(Matcher::PartialJson(serde_json::json!({
            "count": 42,
            "enabled": true
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    uxc()
        .arg(server.url())
        .arg("post:/pets")
        .arg("count=42")
        .arg("enabled=true")
        .assert()
        .success();
}

#[test]
fn positional_json_arguments_are_coerced_before_openapi_execution() {
    let mut server = Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(openapi_schema())
        .create();

    let _execute = server
        .mock("POST", "/pets")
        .match_header(
            "content-type",
            Matcher::Regex("application/json".to_string()),
        )
        .match_body(Matcher::PartialJson(serde_json::json!({
            "count": 42,
            "enabled": true
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    uxc()
        .arg(server.url())
        .arg("post:/pets")
        .arg(r#"{"count":"42","enabled":"true"}"#)
        .assert()
        .success();
}
