use assert_cmd::Command;
use mockito::{Matcher, Server};
use std::fs;

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

fn multipart_openapi_schema() -> String {
    serde_json::json!({
        "openapi": "3.0.0",
        "info": { "title": "upload", "version": "1.0.0" },
        "paths": {
            "/upload": {
                "post": {
                    "requestBody": {
                        "required": true,
                        "content": {
                            "multipart/form-data": {
                                "schema": {
                                    "type": "object",
                                    "required": ["caption", "file"],
                                    "properties": {
                                        "caption": { "type": "string" },
                                        "count": { "type": "integer" },
                                        "file": { "type": "string", "format": "binary" }
                                    },
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
        .arg("--no-cache")
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
        .arg("--no-cache")
        .arg("post:/pets")
        .arg(r#"{"count":"42","enabled":"true"}"#)
        .assert()
        .success();
}

#[test]
fn key_value_arguments_are_coerced_before_multipart_openapi_execution() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("upload.txt");
    fs::write(&file_path, "multipart key value").unwrap();

    let mut server = Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(multipart_openapi_schema())
        .create();

    let _execute = server
        .mock("POST", "/upload")
        .match_header(
            "content-type",
            Matcher::Regex(r"multipart/form-data; boundary=".to_string()),
        )
        .match_body(Matcher::Regex(
            r#"(?s)name="caption".*hello.*name="count".*42.*name="file"; filename="upload\.txt".*multipart key value"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    uxc()
        .arg(server.url())
        .arg("--no-cache")
        .arg("post:/upload")
        .arg("caption=hello")
        .arg("count=42")
        .arg(format!("file={}", file_path.display()))
        .assert()
        .success();
}

#[test]
fn positional_json_arguments_are_coerced_before_multipart_openapi_execution() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("payload.txt");
    fs::write(&file_path, "multipart positional json").unwrap();

    let mut server = Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(multipart_openapi_schema())
        .create();

    let _execute = server
        .mock("POST", "/upload")
        .match_header(
            "content-type",
            Matcher::Regex(r"multipart/form-data; boundary=".to_string()),
        )
        .match_body(Matcher::Regex(
            r#"(?s)name="caption".*hello json.*name="count".*42.*name="file"; filename="payload\.txt".*multipart positional json"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    uxc()
        .arg(server.url())
        .arg("--no-cache")
        .arg("post:/upload")
        .arg(format!(
            r#"{{"caption":"hello json","count":"42","file":"{}"}}"#,
            file_path.display()
        ))
        .assert()
        .success();
}
