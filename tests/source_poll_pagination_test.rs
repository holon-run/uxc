mod common;

use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

fn fixture() -> (ServerGuard, Mock) {
    let mut server = Server::new();
    let schema = json!({
        "openapi": "3.0.3",
        "info": {"title":"Poll pagination fixture","version":"1"},
        "servers": [{"url":server.url()}],
        "paths": {"/events":{"get":{
            "parameters":[
                {"name":"page","in":"query","schema":{"type":"integer"}},
                {"name":"per_page","in":"query","required":true,"schema":{"type":"integer"}}
            ],
            "responses":{"200":{"description":"feed"}}
        }}}
    });
    let mock = server
        .mock("GET", "/openapi.json")
        .with_header("content-type", "application/json")
        .with_body(schema.to_string())
        .expect_at_least(1)
        .create();
    (server, mock)
}

fn page(server: &mut Server, n: u32, etag: Option<&str>) -> Mock {
    let validator = etag.map_or(Matcher::Missing, |etag| Matcher::Exact(etag.to_string()));
    server
        .mock("GET", "/events")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("page".to_string(), n.to_string()),
            Matcher::UrlEncoded("per_page".to_string(), "10".to_string()),
        ]))
        .match_header("if-none-match", validator)
        .match_header("if-modified-since", Matcher::Missing)
        .with_header("content-type", "application/json")
}

fn items(ids: impl IntoIterator<Item = i64>) -> String {
    json!({"items": ids.into_iter().map(|id| json!({"id":id})).collect::<Vec<_>>()}).to_string()
}

fn cli(home: &Path, args: &[&str]) -> Value {
    let output = common::uxc_command_with_home(home)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    envelope["data"].clone()
}

fn ensure(home: &Path, server: &Server, max_pages: u32) -> Value {
    let config = json!({
        "interval_secs":1,
        "initial_items_limit":0,
        "extract_items_pointer":"/items",
        "checkpoint_strategy":{"type":"item_key","item_key_pointer":"/id","seen_window":30},
        "index_pagination":{"page_arg":"page","page_size_arg":"per_page","max_pages":max_pages}
    });
    cli(
        home,
        &[
            "source",
            "ensure",
            "test",
            "indexed",
            &server.url(),
            "get:/events",
            r#"{"per_page":10}"#,
            "--mode",
            "poll",
            "--poll-config",
            &config.to_string(),
            "--timeout-ms",
            "750",
        ],
    )
}

fn wait_status(home: &Path, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(16);
    loop {
        let state = cli(home, &["source", "status", "test", "indexed"]);
        if predicate(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "{state:#}");
        thread::sleep(Duration::from_millis(40));
    }
}

fn assert_events(home: &Path, stream: &str, expected: impl IntoIterator<Item = i64>) {
    let data = cli(home, &["stream", "read", stream]);
    let events = data["events"].as_array().unwrap();
    let expected: Vec<_> = expected.into_iter().collect();
    assert_eq!(events.len(), expected.len(), "{events:?}");
    for (i, (event, id)) in events.iter().zip(expected).enumerate() {
        assert_eq!(event["offset"], i + 1);
        assert_eq!(event["raw_payload"]["id"], id);
    }
}

#[test]
fn indexed_poll_http_walks_16_over_10_and_persists_gap_after_restart() {
    let home = common::fresh_test_home_dir();
    let (mut server, _schema) = fixture();
    let baseline = page(&mut server, 1, None)
        .with_header("etag", "\"base\"")
        .with_body(items((-9..=0).rev()))
        .create();
    let burst_home = page(&mut server, 1, Some("\"base\""))
        .with_header("etag", "\"burst\"")
        .with_body(items((7..=16).rev()))
        .create();
    // Six new events, then the first cross-round anchor. The homepage ETag must win.
    let burst_tail = page(&mut server, 2, None)
        .with_header("etag", "\"tail\"")
        .with_body(items((0..=6).rev()))
        .create();
    let cap_home = page(&mut server, 1, Some("\"burst\""))
        .with_header("etag", "\"capped\"")
        .with_body(items((31..=40).rev()))
        .create();
    let cap_tail = page(&mut server, 2, None)
        .with_header("etag", "\"cap-tail\"")
        .with_body(items((21..=30).rev()))
        .create();
    let unchanged = page(&mut server, 1, Some("\"capped\""))
        .with_status(304)
        .with_header("x-poll-interval", "10")
        .expect_at_least(1)
        .create();

    let ensured = ensure(home.path(), &server, 2);
    let stream = ensured["stream_id"].as_str().unwrap();
    let burst = wait_status(home.path(), |state| state["stream"]["event_count"] == 16);
    assert!(burst["checkpoint"]["possible_gap_count"].is_null());
    assert_eq!(burst["checkpoint"]["etag"], "\"burst\"");
    assert_events(home.path(), stream, (1..=16).rev());
    let state = wait_status(home.path(), |state| {
        state["checkpoint"]["consecutive_304_count"] == 1
    });
    assert_eq!(state["checkpoint"]["possible_gap_count"], 1);
    assert_eq!(
        state["checkpoint"]["last_possible_gap_reason"],
        "page_budget_exhausted"
    );
    assert!(state["checkpoint"]["last_possible_gap_at_unix"].is_u64());
    assert!(state["last_error"].is_null());
    assert_events(home.path(), stream, (1..=16).rev().chain((21..=40).rev()));
    common::stop_test_daemon(home.path());
    let log = fs::read_to_string(home.path().join(".uxc/daemon/daemon.log")).unwrap();
    let warnings = log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "managed_source_possible_gap")
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["meta"]["reason"], "page_budget_exhausted");
    assert_eq!(warnings[0]["meta"]["possible_gap_count"], 1);

    let restarted = ensure(home.path(), &server, 2);
    assert_eq!(restarted["run_id"], ensured["run_id"]);
    let state = wait_status(home.path(), |state| {
        state["checkpoint"]["consecutive_304_count"] == 2
    });
    assert_eq!(state["checkpoint"]["possible_gap_count"], 1);
    assert_eq!(
        state["checkpoint"]["last_possible_gap_reason"],
        "page_budget_exhausted"
    );
    assert_eq!(state["checkpoint"]["etag"], "\"capped\"");
    assert_events(home.path(), stream, (1..=16).rev().chain((21..=40).rev()));
    common::stop_test_daemon(home.path());
    for mock in [
        &baseline,
        &burst_home,
        &burst_tail,
        &cap_home,
        &cap_tail,
        &unchanged,
    ] {
        mock.assert();
    }
}

#[test]
fn indexed_poll_http_discards_failed_pages_and_retries_from_page_one() {
    let home = common::fresh_test_home_dir();
    let (mut server, _schema) = fixture();
    let baseline = page(&mut server, 1, None)
        .with_header("etag", "\"base\"")
        .with_body(items((-9..=0).rev()))
        .create();
    let homepage = page(&mut server, 1, Some("\"base\""))
        .with_header("etag", "\"candidate\"")
        .with_body(items((7..=16).rev()))
        .expect(5)
        .create();
    let throttle = page(&mut server, 2, None).with_status(429).create();
    let malformed = page(&mut server, 2, None)
        .with_body(r#"{"items":[{"id":6},{"malformed":5}]}"#)
        .create();
    let delayed = page(&mut server, 2, None)
        .with_chunked_body(|writer| {
            thread::sleep(Duration::from_millis(1400));
            writer.write_all(items((0..=6).rev()).as_bytes())
        })
        .create();
    let unsolicited_304 = page(&mut server, 2, None).with_status(304).create();
    let tail = page(&mut server, 2, None)
        .with_body(items((0..=6).rev()))
        .create();
    let unchanged = page(&mut server, 1, Some("\"candidate\""))
        .with_status(304)
        .with_header("x-poll-interval", "10")
        .create();

    let ensured = ensure(home.path(), &server, 3);
    let stream = ensured["stream_id"].as_str().unwrap();
    for failure in 1..=4 {
        let state = wait_status(home.path(), |state| state["reconnect_count"] == failure);
        assert_eq!(state["status"], "reconnecting", "{state:#}");
        let error = state["last_error"].as_str().unwrap();
        assert!(error.contains("poll page 2:"), "{error}");
        let expected = [
            "429",
            "item_key_pointer",
            "timed out",
            "unexpected HTTP 304",
        ];
        assert!(error.contains(expected[failure as usize - 1]), "{error}");
        assert_eq!(state["stream"]["event_count"], 0);
        assert_eq!(state["checkpoint"]["etag"], "\"base\"");
        assert_eq!(state["checkpoint"]["seen_window_len"], 10);
        assert!(state["checkpoint"]["possible_gap_count"].is_null());
    }
    let state = wait_status(home.path(), |state| {
        state["checkpoint"]["consecutive_304_count"] == 1
    });
    assert_eq!(state["checkpoint"]["etag"], "\"candidate\"");
    assert_eq!(state["checkpoint"]["seen_window_len"], 26);
    assert!(state["checkpoint"]["possible_gap_count"].is_null());
    assert_events(home.path(), stream, (1..=16).rev());
    common::stop_test_daemon(home.path());
    for mock in [
        &baseline,
        &homepage,
        &throttle,
        &malformed,
        &delayed,
        &unsolicited_304,
        &tail,
        &unchanged,
    ] {
        mock.assert();
    }
}
