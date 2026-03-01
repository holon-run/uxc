use serial_test::serial;
use std::process::Command;
use tempfile::TempDir;

fn uxc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uxc"))
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

fn uxc_command_with_home(home: &std::path::Path) -> Command {
    let mut cmd = uxc_command();
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd
}

struct TestAuthFiles {
    _temp_dir: TempDir,
    credentials_file: std::path::PathBuf,
    bindings_file: std::path::PathBuf,
}

impl TestAuthFiles {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        Self {
            credentials_file: temp_dir.path().join("credentials.json"),
            bindings_file: temp_dir.path().join("auth_bindings.json"),
            _temp_dir: temp_dir,
        }
    }
}

fn uxc_command_with_auth_files(files: &TestAuthFiles) -> Command {
    let mut cmd = uxc_command();
    cmd.env("UXC_CREDENTIALS_FILE", &files.credentials_file);
    cmd.env("UXC_AUTH_BINDINGS_FILE", &files.bindings_file);
    cmd
}

fn without_http_scheme(url: &str) -> String {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string()
}

#[test]
#[serial]
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
#[serial]
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
#[serial]
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
#[serial]
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
#[serial]
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
        .arg("-h")
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
#[serial]
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
        .arg("-h")
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
    assert_eq!(json["data"]["examples"][0], "uxc <host> -h");
    assert_eq!(json["data"]["examples"][1], "uxc <host> <operation_id> -h");
    assert_eq!(
        json["data"]["examples"][2],
        "uxc <host> <operation_id> id=42"
    );
    assert_eq!(
        json["data"]["examples"][3],
        "uxc <host> <operation_id> '{...}'"
    );
}

#[test]
#[serial]
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
        .arg("-h")
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
    assert_eq!(json["data"]["examples"][0], "petcli -h");
    assert_eq!(json["data"]["examples"][1], "petcli <operation_id> -h");
    assert_eq!(json["data"]["examples"][2], "petcli <operation_id> id=42");
    assert_eq!(json["data"]["examples"][3], "petcli <operation_id> '{...}'");
}

#[test]
#[serial]
fn host_help_uses_stale_cache_fallback_with_meta() {
    daemon_stop_best_effort();
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    let endpoint = {
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

        let prime = uxc_command_with_home(temp_home.path())
            .arg(server.url())
            .arg("--cache-ttl")
            .arg("1")
            .arg("-h")
            .output()
            .expect("prime cache should succeed");
        assert!(prime.status.success(), "prime run should succeed");

        server.url()
    };

    std::thread::sleep(std::time::Duration::from_secs(2));

    let output = uxc_command_with_home(temp_home.path())
        .arg(&endpoint)
        .arg("--cache-ttl")
        .arg("1")
        .arg("-h")
        .output()
        .expect("offline fallback should run");

    assert!(
        output.status.success(),
        "offline fallback should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "host_help");
    assert_eq!(json["meta"]["cache_fallback"], true);
    assert_eq!(json["meta"]["cache_stale"], true);
    assert_eq!(json["meta"]["cache_source"], "schema_cache");
    assert!(
        json["meta"]["cache_age_ms"].as_u64().is_some(),
        "cache_age_ms should be present"
    );

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn refresh_schema_forces_online_fetch_in_help_flow() {
    let temp_home = tempfile::tempdir().expect("temp home should be created");
    let mut server = mockito::Server::new();
    let endpoint = server.url();

    {
        let _schema_v1 = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r##"{
  "openapi": "3.0.0",
  "info": { "title": "v1", "version": "1.0.0" },
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

        let output = uxc_command_with_home(temp_home.path())
            .arg(&endpoint)
            .arg("-h")
            .output()
            .expect("initial host help should run");
        assert!(output.status.success());
    }

    {
        let _schema_v2 = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r##"{
  "openapi": "3.0.0",
  "info": { "title": "v2", "version": "1.0.0" },
  "paths": {
    "/users": {
      "get": {
        "summary": "list users",
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"##,
            )
            .create();

        let output = uxc_command_with_home(temp_home.path())
            .arg(&endpoint)
            .arg("--refresh-schema")
            .arg("-h")
            .output()
            .expect("refresh host help should run");
        assert!(
            output.status.success(),
            "refresh run should succeed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
        let operations = json["data"]["operations"]
            .as_array()
            .expect("operations should be an array");
        assert!(
            operations
                .iter()
                .any(|op| op["operation_id"] == "get:/users"),
            "expected refreshed operations to include get:/users"
        );
    }
}

#[test]
#[serial]
fn auth_info_alias_outputs_auth_info_json() {
    let files = TestAuthFiles::new();

    let _ = uxc_command_with_auth_files(&files)
        .arg("auth")
        .arg("credential")
        .arg("set")
        .arg("alias-test")
        .arg("--secret")
        .arg("dummy")
        .output()
        .expect("failed to set auth credential");

    let output = uxc_command_with_auth_files(&files)
        .arg("auth")
        .arg("info")
        .arg("alias-test")
        .output()
        .expect("failed to run uxc auth info");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "auth_info");
    assert_eq!(json["operation"], "alias-test");

    let _ = uxc_command_with_auth_files(&files)
        .arg("auth")
        .arg("credential")
        .arg("remove")
        .arg("alias-test")
        .output();
}

#[test]
#[serial]
fn auth_oauth_list_outputs_auth_list_json() {
    let output = uxc_command()
        .arg("auth")
        .arg("oauth")
        .arg("list")
        .output()
        .expect("failed to run uxc auth oauth list");

    assert!(output.status.success(), "command should succeed");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], true);
    assert_eq!(json["kind"], "auth_list");
}

#[test]
#[serial]
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
#[serial]
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
#[serial]
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
#[serial]
fn host_help_keyword_is_treated_as_operation_literal() {
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
        .arg("help")
        .output()
        .expect("failed to run uxc");
    assert!(!output.status.success(), "command should fail");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");

    let message = json["error"]["message"]
        .as_str()
        .expect("error.message should be a string");
    assert!(
        message.contains("Invalid operation ID format")
            || message.contains("Unknown argument 'help'"),
        "unexpected message: {}",
        message
    );
}

#[test]
#[serial]
fn operation_help_keyword_is_not_a_help_alias() {
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

    assert!(!output.status.success(), "command should fail");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Unknown argument 'help'")),
        "unexpected message: {}",
        json["error"]["message"]
    );
}

#[test]
#[serial]
fn link_help_flag_outputs_subcommand_help_json() {
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
#[serial]
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
        .arg("-h")
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
#[serial]
fn dynamic_operation_help_supports_text_output() {
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
        .arg("--text")
        .arg(server.url())
        .arg("--no-cache")
        .arg("get:/pets")
        .arg("-h")
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
#[serial]
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
        .arg("-h")
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
#[serial]
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
#[serial]
fn dynamic_operation_executes_operation() {
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
#[serial]
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
#[serial]
fn dynamic_operation_rejects_conflicting_json_inputs() {
    let output = uxc_command()
        .arg("https://example.com")
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
#[serial]
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
#[serial]
fn dynamic_operation_rejects_json_passed_to_args_flag() {
    let output = uxc_command()
        .arg("https://example.com")
        .arg("op")
        .arg("--args")
        .arg(r#"{"query":"ai"}"#)
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
        .is_some_and(|m| m.contains("Use key=value for --args")));
}

#[test]
#[serial]
fn host_help_outputs_operation_id_and_display_name() {
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
        .arg("-h")
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
#[serial]
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

    let host_help_output = uxc_command()
        .arg(target_server.url())
        .arg("--no-cache")
        .arg("--schema-url")
        .arg(&schema_url)
        .arg("-h")
        .output()
        .expect("failed to run uxc host help");
    assert!(
        host_help_output.status.success(),
        "host help should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&host_help_output.stdout),
        String::from_utf8_lossy(&host_help_output.stderr)
    );
    let host_help_json: serde_json::Value =
        serde_json::from_slice(&host_help_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(host_help_json["protocol"], "openapi");
    assert!(host_help_json["data"]["operations"]
        .as_array()
        .is_some_and(|ops| { ops.iter().any(|op| op["operation_id"] == "get:/pets") }));

    let call_output = uxc_command()
        .arg(target_server.url())
        .arg("--no-cache")
        .arg("--schema-url")
        .arg(&schema_url)
        .arg("get:/pets")
        .output()
        .expect("failed to run uxc operation call");
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
#[serial]
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

    let host_help_output = uxc_command()
        .arg(target_server.url())
        .arg("--no-cache")
        .arg("-h")
        .env("UXC_SCHEMA_MAPPINGS_FILE", &mapping_file_path)
        .output()
        .expect("failed to run uxc host help");
    assert!(
        host_help_output.status.success(),
        "host help should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&host_help_output.stdout),
        String::from_utf8_lossy(&host_help_output.stderr)
    );

    let host_help_json: serde_json::Value =
        serde_json::from_slice(&host_help_output.stdout).expect("stdout should be valid JSON");
    assert_eq!(host_help_json["protocol"], "openapi");
    assert!(host_help_json["data"]["operations"]
        .as_array()
        .is_some_and(|ops| { ops.iter().any(|op| op["operation_id"] == "get:/pets") }));
}
