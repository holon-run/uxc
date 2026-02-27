use std::process::Command;

fn uxc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uxc"))
}

fn without_http_scheme(url: &str) -> String {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string()
}

#[test]
fn bare_invocation_outputs_json_global_help() {
    let output = uxc_command()
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "global_help");
    assert_eq!(json["protocol"], "cli");
}

#[test]
fn global_help_flag_works() {
    let output = uxc_command().arg("-h").output().expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "global_help");
    assert_eq!(json["data"]["path"], "uxc");
}

#[test]
fn help_subcommand_defaults_to_json() {
    let output = uxc_command()
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "global_help");
}

#[test]
fn cache_and_auth_commands_default_to_json() {
    let cache_output = uxc_command()
        .arg("cache")
        .arg("stats")
        .output()
        .expect("failed to run uxc cache stats");
    assert!(cache_output.status.success(), "cache stats should succeed");
    let cache_json: serde_json::Value =
        serde_json::from_slice(&cache_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(cache_json["ok"], true);
    assert_eq!(cache_json["kind"], "cache_stats");

    let auth_output = uxc_command()
        .arg("auth")
        .arg("credential")
        .arg("list")
        .output()
        .expect("failed to run uxc auth credential list");
    assert!(
        auth_output.status.success(),
        "auth credential list should succeed"
    );
    let auth_json: serde_json::Value =
        serde_json::from_slice(&auth_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(auth_json["ok"], true);
    assert_eq!(auth_json["kind"], "auth_list");
}

#[test]
fn operation_help_works_with_dynamic_syntax() {
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

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("get:/pets")
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "operation_detail");
    assert_eq!(json["operation"], "get:/pets");
    assert_eq!(json["data"]["operation_id"], "get:/pets");
    assert_eq!(json["data"]["display_name"], "GET /pets");
}

#[test]
fn host_help_supports_url_without_scheme() {
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

    let output = uxc_command()
        .arg(without_http_scheme(&server.url()))
        .arg("--no-cache")
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "host_help");
    assert_eq!(json["data"]["examples"][0], "uxc <host> list");
    assert_eq!(
        json["data"]["examples"][1],
        "uxc <host> describe <operation_id>"
    );
    assert_eq!(
        json["data"]["examples"][2],
        "uxc <host> call <operation_id> --input-json '{...}'"
    );
}

#[test]
fn host_help_uses_link_name_for_next_commands_when_env_set() {
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

    let output = uxc_command()
        .env("UXC_LINK_NAME", "petcli")
        .arg(server.url())
        .arg("--no-cache")
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "host_help");
    assert_eq!(json["data"]["examples"][0], "petcli list");
    assert_eq!(
        json["data"]["examples"][1],
        "petcli describe <operation_id>"
    );
    assert_eq!(
        json["data"]["examples"][2],
        "petcli call <operation_id> --input-json '{...}'"
    );
}

#[test]
fn cache_without_subcommand_outputs_subcommand_help_json() {
    let output = uxc_command()
        .arg("cache")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "subcommand_help");
    assert_eq!(json["data"]["path"], "uxc cache");
}

#[test]
fn cache_stats_help_outputs_specific_subcommand_path() {
    let output = uxc_command()
        .arg("cache")
        .arg("stats")
        .arg("-h")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "subcommand_help");
    assert_eq!(json["data"]["path"], "uxc cache stats");
}

#[test]
fn auth_credential_without_subcommand_outputs_subcommand_help_json() {
    let output = uxc_command()
        .arg("auth")
        .arg("credential")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "subcommand_help");
    assert_eq!(json["data"]["path"], "uxc auth credential");
}

#[test]
fn call_and_link_help_flags_output_subcommand_help_json() {
    let call = uxc_command()
        .arg("call")
        .arg("-h")
        .output()
        .expect("failed to run uxc call -h");
    assert!(call.status.success(), "command should succeed");
    let call_json: serde_json::Value =
        serde_json::from_slice(&call.stdout).expect("stdout should be valid JSON");
    assert_eq!(call_json["ok"], true);
    assert_eq!(call_json["kind"], "subcommand_help");
    assert_eq!(call_json["data"]["path"], "uxc call");

    let link = uxc_command()
        .arg("link")
        .arg("--help")
        .output()
        .expect("failed to run uxc link --help");
    assert!(link.status.success(), "command should succeed");
    let link_json: serde_json::Value =
        serde_json::from_slice(&link.stdout).expect("stdout should be valid JSON");
    assert_eq!(link_json["ok"], true);
    assert_eq!(link_json["kind"], "subcommand_help");
    assert_eq!(link_json["data"]["path"], "uxc link");
}

#[test]
fn operation_help_supports_url_without_scheme() {
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

    let output = uxc_command()
        .arg(without_http_scheme(&server.url()))
        .arg("--no-cache")
        .arg("get:/pets")
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "operation_detail");
    assert_eq!(json["operation"], "get:/pets");
}

#[test]
fn dynamic_operation_help_accepts_text_flag_after_help() {
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

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("get:/pets")
        .arg("help")
        .arg("--text")
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Protocol: openapi"));
    assert!(stdout.contains("Operation ID: get:/pets"));
}

#[test]
fn operation_help_includes_openapi_request_body_schema() {
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
    "/pet": {
      "post": {
        "summary": "add pet",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": { "$ref": "#/components/schemas/Pet" }
            }
          }
        },
        "responses": { "200": { "description": "ok" } }
      }
    }
  },
  "components": {
    "schemas": {
      "Pet": {
        "type": "object",
        "required": ["name"],
        "properties": {
          "name": { "type": "string" }
        }
      }
    }
  }
}"##,
        )
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("post:/pet")
        .arg("help")
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["kind"], "operation_detail");
    assert_eq!(json["data"]["input_schema"]["kind"], "openapi_request_body");
    assert_eq!(
        json["data"]["input_schema"]["content"]["application/json"]["schema"]["properties"]["name"]
            ["type"],
        "string"
    );
}

#[test]
fn text_and_format_flags_are_mutually_exclusive() {
    let output = uxc_command()
        .arg("--format")
        .arg("json")
        .arg("--text")
        .output()
        .expect("failed to run uxc");

    assert!(!output.status.success(), "command should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"), "stderr: {}", stderr);
}

#[test]
fn call_subcommand_executes_operation() {
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
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
}"#,
        )
        .create();
    let _call = server
        .mock("GET", "/pets")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"items":[]}"#)
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("call")
        .arg("get:/pets")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "call_result");
    assert_eq!(json["operation"], "get:/pets");
}

#[test]
fn dynamic_operation_accepts_bare_json_payload() {
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
  "openapi": "3.0.0",
  "info": { "title": "test", "version": "1.0.0" },
  "paths": {
    "/echo": {
      "post": {
        "summary": "echo payload",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["message"],
                "properties": {
                  "message": { "type": "string" }
                }
              }
            }
          }
        },
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"#,
        )
        .create();
    let _call = server
        .mock("POST", "/echo")
        .match_body(r#"{"message":"hello"}"#)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"hello"}"#)
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("post:/echo")
        .arg(r#"{"message":"hello"}"#)
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "call_result");
    assert_eq!(json["operation"], "post:/echo");
    assert_eq!(json["data"]["message"], "hello");
}

#[test]
fn call_subcommand_accepts_bare_json_payload() {
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
  "openapi": "3.0.0",
  "info": { "title": "test", "version": "1.0.0" },
  "paths": {
    "/echo": {
      "post": {
        "summary": "echo payload",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["message"],
                "properties": {
                  "message": { "type": "string" }
                }
              }
            }
          }
        },
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"#,
        )
        .create();
    let _call = server
        .mock("POST", "/echo")
        .match_body(r#"{"message":"hello"}"#)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"hello"}"#)
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("call")
        .arg("post:/echo")
        .arg(r#"{"message":"hello"}"#)
        .output()
        .expect("failed to run uxc");

    assert!(
        output.status.success(),
        "command should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "call_result");
    assert_eq!(json["operation"], "post:/echo");
    assert_eq!(json["data"]["message"], "hello");
}

#[test]
fn call_subcommand_rejects_conflicting_json_inputs() {
    let output = uxc_command()
        .arg("https://example.com")
        .arg("call")
        .arg("post:/echo")
        .arg("--input-json")
        .arg(r#"{"message":"from-flag"}"#)
        .arg(r#"{"message":"from-positional"}"#)
        .output()
        .expect("failed to run uxc");

    assert!(
        !output.status.success(),
        "command should fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|m| m.contains("Cannot provide both --input-json and positional JSON")));
}

#[test]
fn dynamic_operation_rejects_non_object_positional_json_payload() {
    let output = uxc_command()
        .arg("https://example.com")
        .arg("op")
        .arg(r#"["not","object"]"#)
        .output()
        .expect("failed to run uxc");

    assert!(
        !output.status.success(),
        "command should fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|m| m.contains("must be an object")));
}

#[test]
fn list_outputs_operation_id_and_display_name() {
    let mut server = mockito::Server::new();
    let _schema = server
        .mock("GET", "/openapi.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
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
}"#,
        )
        .create();

    let output = uxc_command()
        .arg(server.url())
        .arg("--no-cache")
        .arg("list")
        .output()
        .expect("failed to run uxc");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let first = &json["data"]["operations"][0];
    assert_eq!(first["operation_id"], "get:/pets");
    assert_eq!(first["display_name"], "GET /pets");
}

#[test]
fn schema_url_override_supports_schema_separated_openapi_service() {
    let mut target_server = mockito::Server::new();
    let _call = target_server
        .mock("GET", "/pets")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"items":[]}"#)
        .create();

    let mut schema_server = mockito::Server::new();
    let _schema = schema_server
        .mock("GET", "/schema.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
  "openapi": "3.0.0",
  "info": { "title": "separated", "version": "1.0.0" },
  "paths": {
    "/pets": {
      "get": {
        "summary": "list pets",
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"#,
        )
        .create();
    let schema_url = format!("{}/schema.json", schema_server.url());

    let list_output = uxc_command()
        .arg(target_server.url())
        .arg("--no-cache")
        .arg("--schema-url")
        .arg(&schema_url)
        .arg("list")
        .output()
        .expect("failed to run uxc list");
    assert!(
        list_output.status.success(),
        "list should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );
    let list_json: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(list_json["protocol"], "openapi");
    assert!(list_json["data"]["operations"]
        .as_array()
        .is_some_and(|ops| { ops.iter().any(|op| op["operation_id"] == "get:/pets") }));

    let call_output = uxc_command()
        .arg(target_server.url())
        .arg("--no-cache")
        .arg("--schema-url")
        .arg(&schema_url)
        .arg("call")
        .arg("get:/pets")
        .output()
        .expect("failed to run uxc call");
    assert!(
        call_output.status.success(),
        "call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&call_output.stdout),
        String::from_utf8_lossy(&call_output.stderr)
    );
    let call_json: serde_json::Value =
        serde_json::from_slice(&call_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(call_json["ok"], true);
    assert_eq!(call_json["kind"], "call_result");
}

#[test]
fn user_schema_mapping_file_supports_schema_separated_openapi_service() {
    let mut target_server = mockito::Server::new();
    let _call = target_server
        .mock("GET", "/pets")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"items":[]}"#)
        .create();

    let mut schema_server = mockito::Server::new();
    let _schema = schema_server
        .mock("GET", "/schema.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
  "openapi": "3.0.0",
  "info": { "title": "mapped", "version": "1.0.0" },
  "paths": {
    "/pets": {
      "get": {
        "summary": "list pets",
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"#,
        )
        .create();

    let mapping_file_dir = tempfile::tempdir().expect("failed to create tempdir");
    let mapping_file_path = mapping_file_dir.path().join("schema_mappings.json");
    let schema_url = format!("{}/schema.json", schema_server.url());
    std::fs::write(
        &mapping_file_path,
        format!(
            r#"{{
  "version": 1,
  "openapi": [
    {{
      "host": "127.0.0.1",
      "path_prefix": "/",
      "schema_url": "{schema_url}",
      "priority": 100
    }}
  ]
}}"#,
            schema_url = schema_url
        ),
    )
    .expect("failed to write schema mapping file");

    let list_output = uxc_command()
        .arg(target_server.url())
        .arg("--no-cache")
        .arg("list")
        .env("UXC_SCHEMA_MAPPINGS_FILE", &mapping_file_path)
        .output()
        .expect("failed to run uxc list");
    assert!(
        list_output.status.success(),
        "list should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );

    let list_json: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(list_json["protocol"], "openapi");
    assert!(list_json["data"]["operations"]
        .as_array()
        .is_some_and(|ops| { ops.iter().any(|op| op["operation_id"] == "get:/pets") }));
}
