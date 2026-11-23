use assert_cmd::Command;
use serde_json::{json, Value};
use std::process::Output;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

// SEP-3415 Events Beta fixture pinned to modelcontextprotocol/ext-events
// commit c77b84395eff0ae32f53b0516c2214fe276ed050.
const PROTOCOL_VERSION: &str = "2026-07-28";
const EXTENSION: &str = "io.modelcontextprotocol/events";
const EVENT: &str = "weather.reading";
const WEBHOOK_ONLY: &str = "weather.alert";
const PAGE_TWO: &str = "catalog:2+/=";
const INITIAL: &str = "reading:0+/=";
const NEXT: &str = "reading:1+/=";
const EARLIEST: &str = "reading:100+/=";

fn list_page(cursor: &Value) -> Value {
    if cursor == PAGE_TWO {
        json!({
            "resultType": "complete",
            "events": [{
                "name": EVENT,
                "description": "A station reading",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "station": {"type": "string", "x-mcp-header": "Station"},
                        "units": {"type": "string"}
                    },
                    "required": ["station"]
                },
                "payloadSchema": {
                    "type": "object",
                    "properties": {"temperature": {"type": "number"}}
                },
                "delivery": ["poll"],
                "_meta": {"fixture.example/category": "weather"},
                "fixtureField": {"keep": true}
            }]
        })
    } else {
        json!({
            "resultType": "complete",
            "events": [{
                "name": WEBHOOK_ONLY,
                "inputSchema": {"type": "object"},
                "payloadSchema": {"type": "object"},
                "delivery": ["webhook"]
            }],
            "nextCursor": PAGE_TWO,
            "_meta": {"fixture.example/page": 1}
        })
    }
}

fn poll_result(cursor: &str) -> Value {
    let events = if cursor == NEXT {
        json!([{
            "eventId": "reading-1",
            "name": EVENT,
            "timestamp": "2026-11-23T01:00:00Z",
            "data": {"temperature": 5.5},
            "cursor": NEXT,
            "_meta": {"fixture.example/station": "sfo"}
        }])
    } else {
        json!([])
    };
    json!({
        "resultType": "complete",
        "events": events,
        "cursor": cursor,
        "truncated": cursor == EARLIEST,
        "hasMore": cursor == NEXT,
        "nextPollMs": 2500,
        "_meta": {"fixture.example/trace": "poll"},
        "fixtureField": {"keep": true}
    })
}

#[derive(Debug)]
struct EventRequest {
    body: Value,
    headers: Value,
}

fn rpc_reply(
    request: &mockito::Request,
    body: &Value,
    events_enabled: bool,
) -> (&'static str, Value) {
    let params = &body["params"];
    let error = |code, message, data: Value| {
        (
            "error",
            json!({"code": code, "message": message, "data": data}),
        )
    };
    match body["method"].as_str() {
        Some("server/discover") => {
            let mut capabilities = json!({"tools": {}});
            if events_enabled {
                capabilities["extensions"] = json!({(EXTENSION): {}});
            }
            (
                "result",
                json!({
                    "resultType": "complete",
                    "supportedVersions": [PROTOCOL_VERSION],
                    "capabilities": capabilities
                }),
            )
        }
        Some("tools/list") => (
            "result",
            json!({
                "resultType": "complete",
                "tools": [{"name": "ping", "inputSchema": {"type": "object"}}]
            }),
        ),
        Some("events/list") if events_enabled => ("result", list_page(&params["cursor"])),
        Some("events/poll") if events_enabled => match params["name"].as_str() {
            Some(WEBHOOK_ONLY) => error(
                -32026,
                "Unsupported",
                json!({"feature": "deliveryMode", "value": "poll"}),
            ),
            Some(EVENT) => match params["cursor"].as_str() {
                None if params.get("cursor").is_some_and(Value::is_null) => {
                    ("result", poll_result(INITIAL))
                }
                Some(INITIAL) => ("result", poll_result(NEXT)),
                Some("expired-cursor") => ("result", poll_result(EARLIEST)),
                _ => error(
                    -32602,
                    "Invalid cursor",
                    json!({"field": "cursor", "reason": "malformed"}),
                ),
            },
            _ => error(-32023, "Not found", json!({"kind": "event"})),
        },
        _ => error(-32601, "Method not found", json!({"path": request.path()})),
    }
}

struct Fixture {
    endpoint: String,
    requests: Arc<Mutex<Vec<EventRequest>>>,
    _server: mockito::ServerGuard,
}

impl Fixture {
    fn new(events_enabled: bool) -> Self {
        let mut server = mockito::Server::new();
        let endpoint = format!("{}/mcp", server.url());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        server
            .mock("POST", "/mcp")
            .with_header("content-type", "application/json")
            .with_body_from_request(move |request| {
                let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
                let (key, reply) = rpc_reply(request, &body, events_enabled);
                let response = json!({"jsonrpc": "2.0", "id": body["id"], (key): reply});
                let headers = |name| {
                    request
                        .header(name)
                        .first()
                        .map(|value| value.to_str().unwrap().to_owned())
                };
                recorded.lock().unwrap().push(EventRequest {
                    body,
                    headers: json!({
                        "method": headers("mcp-method"),
                        "version": headers("mcp-protocol-version"),
                        "name": headers("mcp-name"),
                        "station": headers("mcp-param-station"),
                        "units": headers("mcp-param-units"),
                        "session": headers("mcp-session-id")
                    }),
                });
                response.to_string().into_bytes()
            })
            .create();
        Self {
            endpoint,
            requests,
            _server: server,
        }
    }
}

struct Cli(TempDir);

impl Cli {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn command(&self) -> Command {
        let home = self.0.path();
        let mut command = Command::new(env!("CARGO_BIN_EXE_uxc"));
        command
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .env_remove("UXC_CREDENTIALS_FILE")
            .env_remove("UXC_AUTH_BINDINGS_FILE")
            .env("UXC_DAEMON_IDLE_TIMEOUT_SECS", "30")
            .timeout(Duration::from_secs(20));
        command
    }

    fn run(&self, fixture: &Fixture, args: &[&str]) -> Output {
        self.command()
            .arg(&fixture.endpoint)
            .args(args)
            .output()
            .unwrap()
    }

    fn call(&self, fixture: &Fixture, args: &[&str]) -> Value {
        let result = envelope(self.run(fixture, args), true);
        assert_eq!(result["kind"], "call_result", "{result}");
        assert_eq!(result["protocol"], "mcp", "{result}");
        assert_eq!(result["operation"], args[0], "{result}");
        result["data"].clone()
    }

    fn assert_no_delivery_state(&self) {
        let daemon = self.0.path().join(".uxc/daemon");
        for name in [
            "managed-source-checkpoints",
            "managed-source-cursors",
            "managed-source-sinks",
        ] {
            assert!(
                !daemon.join(name).exists(),
                "unexpected delivery state: {name}"
            );
        }
        let database = daemon.join("managed-source-streams.db");
        if database.exists() {
            let connection = rusqlite::Connection::open_with_flags(
                database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let count: i64 = connection
                .query_row("SELECT count(*) FROM managed_sources", [], |row| row.get(0))
                .unwrap();
            assert_eq!(
                count, 0,
                "a one-shot poll must not register a managed source"
            );
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = self.command().args(["daemon", "stop"]).output();
    }
}

fn envelope(output: Output, success: bool) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.success(),
        success,
        "stdout={stdout}\nstderr={stderr}"
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("invalid CLI JSON ({error}): stdout={stdout}\nstderr={stderr}")
    });
    assert_eq!(envelope["ok"], success, "{envelope}");
    envelope
}

#[test]
fn discovers_event_operations_and_lists_one_page_per_call() {
    let fixture = Fixture::new(true);
    let cli = Cli::new();
    let help = cli.run(&fixture, &["-h"]);
    assert!(help.status.success(), "{help:?}");
    let help = String::from_utf8_lossy(&help.stdout);
    for operation in ["events/list", "events/poll"] {
        assert!(help.contains(operation), "{help}");
    }
    let schema = envelope(cli.run(&fixture, &["--codegen-schema"]), true);
    assert_eq!(schema["kind"], "codegen_host_schema", "{schema}");
    for name in ["events/list", "events/poll"] {
        let operation = schema["data"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["id"] == name)
            .unwrap_or_else(|| panic!("{name} missing from {schema}"));
        assert_eq!(operation["kind"], "execute", "{operation}");
        assert_eq!(operation["subscribable"], false, "{operation}");
    }

    let first = cli.call(&fixture, &["events/list"]);
    assert_eq!(first, list_page(&Value::Null));
    let cursor = first["nextCursor"].as_str().unwrap();
    assert_eq!(
        cli.call(&fixture, &["events/list", &format!("cursor={cursor}")]),
        list_page(&json!(cursor))
    );
    let requests = fixture.requests.lock().unwrap();
    let lists: Vec<_> = requests
        .iter()
        .filter(|r| r.body["method"] == "events/list")
        .collect();
    assert_eq!(lists.len(), 2, "{requests:?}");
    assert!(lists[0].body["params"].get("cursor").is_none());
    assert_eq!(lists[1].body["params"]["cursor"], PAGE_TWO);
    for list in lists {
        assert_eq!(list.headers["method"], "events/list");
        assert!(list.headers["name"].is_null());
    }
}

#[test]
fn polls_once_and_resumes_using_only_the_server_cursor() {
    let fixture = Fixture::new(true);
    let args = [
        "events/poll",
        "name=weather.reading",
        "arguments:={\"station\":\"sfo\",\"units\":\"celsius\"}",
    ];

    let first = Cli::new();
    let initial = first.call(&fixture, &[&args[..], &["cursor:=null"]].concat());
    assert_eq!(initial, poll_result(INITIAL));
    first.assert_no_delivery_state();
    drop(first); // Resume in another CLI process, with no daemon or cache carried over.

    let resumed = Cli::new();
    let cursor = format!("cursor={}", initial["cursor"].as_str().unwrap());
    assert_eq!(
        resumed.call(&fixture, &[&args[..], &[&cursor]].concat()),
        poll_result(NEXT)
    );
    assert_eq!(
        resumed.call(&fixture, &[&args[..], &["cursor=expired-cursor"]].concat()),
        poll_result(EARLIEST)
    );
    // Omission is equivalent to an explicit JSON null on the wire.
    assert_eq!(resumed.call(&fixture, &args), poll_result(INITIAL));
    resumed.assert_no_delivery_state();

    let requests = fixture.requests.lock().unwrap();
    let polls: Vec<_> = requests
        .iter()
        .filter(|r| r.body["method"] == "events/poll")
        .collect();
    assert_eq!(polls.len(), 4, "{requests:?}");
    for (request, cursor) in polls.into_iter().zip([
        Value::Null,
        json!(INITIAL),
        json!("expired-cursor"),
        Value::Null,
    ]) {
        let params = &request.body["params"];
        assert_eq!(params.get("cursor"), Some(&cursor), "{request:?}");
        assert_eq!(params["name"], EVENT);
        assert_eq!(
            params["arguments"],
            json!({"station": "sfo", "units": "celsius"})
        );
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/clientCapabilities"]["extensions"][EXTENSION],
            json!({})
        );
        assert_eq!(
            request.headers,
            json!({
                "method": "events/poll",
                "version": PROTOCOL_VERSION,
                "name": EVENT,
                "station": "sfo",
                "units": null,
                "session": null
            })
        );
    }
}

#[test]
fn missing_extension_reports_error_without_sending_an_events_request() {
    let fixture = Fixture::new(false);
    let cli = Cli::new();
    for args in [
        &["events/list"][..],
        &["events/poll", "name=weather.reading"][..],
    ] {
        let error = envelope(cli.run(&fixture, args), false);
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains(EXTENSION),
            "{error}"
        );
    }
    let requests = fixture.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r.body["method"] == "server/discover"),
        "{requests:?}"
    );
    assert!(
        requests.iter().all(|r| matches!(
            r.body["method"].as_str(),
            Some("server/discover" | "tools/list" | "subscriptions/listen")
        )),
        "{requests:?}"
    );
}

#[test]
fn preserves_webhook_only_and_invalid_cursor_jsonrpc_errors() {
    let fixture = Fixture::new(true);
    let cli = Cli::new();
    let unsupported = envelope(
        cli.run(&fixture, &["events/poll", "name=weather.alert"]),
        false,
    );
    let error = &unsupported["error"];
    assert_eq!(error["details"]["jsonrpc_code"], -32026, "{unsupported}");
    assert_eq!(error["details"]["feature"], "deliveryMode");
    assert_eq!(error["details"]["value"], "poll");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(WEBHOOK_ONLY) && message.contains("poll"),
        "{unsupported}"
    );

    let invalid = envelope(
        cli.run(
            &fixture,
            &[
                "events/poll",
                "name=weather.reading",
                "arguments.station=sfo",
                "cursor=malformed",
            ],
        ),
        false,
    );
    assert_eq!(
        invalid["error"]["details"]["jsonrpc_code"], -32602,
        "{invalid}"
    );
    assert_eq!(invalid["error"]["details"]["field"], "cursor");
    assert_eq!(invalid["error"]["details"]["reason"], "malformed");

    let requests = fixture.requests.lock().unwrap();
    let polls: Vec<_> = requests
        .iter()
        .filter(|r| r.body["method"] == "events/poll")
        .collect();
    assert_eq!(polls.len(), 2, "{requests:?}");
    assert_eq!(polls[0].headers["name"], WEBHOOK_ONLY);
    assert_eq!(polls[1].headers["name"], EVENT);
    cli.assert_no_delivery_state();
}
