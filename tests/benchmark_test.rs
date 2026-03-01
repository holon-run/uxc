mod common;

use std::time::{Duration, Instant};

use assert_cmd::Command;
use common::{start_test_server, test_server_binary};
use serial_test::serial;

fn uxc_command() -> Command {
    Command::cargo_bin("uxc").expect("uxc binary should build")
}

fn daemon_stop_best_effort() {
    let _ = uxc_command().arg("daemon").arg("stop").output();
}

fn warm_latency_bound(cold: Duration) -> Duration {
    cold.saturating_mul(25)
        .saturating_add(Duration::from_millis(50))
}

fn benchmark_sample_count() -> usize {
    std::env::var("UXC_BENCH_P95_SAMPLES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 5)
        .unwrap_or(20)
}

#[test]
#[serial]
fn benchmark_mcp_stdio_cold_vs_warm_latency() {
    daemon_stop_best_effort();

    let bin = test_server_binary("mcp-stdio");
    let endpoint = format!("{} ok", bin.display());

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let cold_t0 = Instant::now();
    let cold_output = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"benchmark-cold"}"#)
        .output()
        .expect("cold call should run");
    let cold = cold_t0.elapsed();
    assert!(
        cold_output.status.success(),
        "cold call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cold_output.stdout),
        String::from_utf8_lossy(&cold_output.stderr)
    );

    let warm_t0 = Instant::now();
    let warm_output = uxc_command()
        .arg(&endpoint)
        .arg("echo")
        .arg("--input-json")
        .arg(r#"{"message":"benchmark-warm"}"#)
        .output()
        .expect("warm call should run");
    let warm = warm_t0.elapsed();
    assert!(
        warm_output.status.success(),
        "warm call should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&warm_output.stdout),
        String::from_utf8_lossy(&warm_output.stderr)
    );

    eprintln!(
        "mcp-stdio latency: cold={}ms warm={}ms",
        cold.as_millis(),
        warm.as_millis()
    );
    assert!(
        warm <= warm_latency_bound(cold),
        "warm call unexpectedly slower: cold={}ms warm={}ms",
        cold.as_millis(),
        warm.as_millis()
    );

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn benchmark_openapi_http_cold_vs_warm_latency() {
    daemon_stop_best_effort();

    let server = start_test_server("openapi", "ok");
    let endpoint = format!("http://{}/", server.addr);

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let cold_t0 = Instant::now();
    let cold_output = uxc_command()
        .arg(&endpoint)
        .arg("get:/health")
        .output()
        .expect("cold call should run");
    let cold = cold_t0.elapsed();
    assert!(cold_output.status.success());

    let warm_t0 = Instant::now();
    let warm_output = uxc_command()
        .arg(&endpoint)
        .arg("get:/health")
        .output()
        .expect("warm call should run");
    let warm = warm_t0.elapsed();
    assert!(warm_output.status.success());

    eprintln!(
        "openapi-http latency: cold={}ms warm={}ms",
        cold.as_millis(),
        warm.as_millis()
    );

    assert!(
        warm <= warm_latency_bound(cold),
        "warm call unexpectedly slower: cold={}ms warm={}ms",
        cold.as_millis(),
        warm.as_millis()
    );

    daemon_stop_best_effort();
}

#[test]
#[serial]
fn benchmark_repeated_call_latency_p95() {
    daemon_stop_best_effort();

    let server = start_test_server("openapi", "ok");
    let endpoint = format!("http://{}/", server.addr);

    let start = uxc_command()
        .arg("daemon")
        .arg("start")
        .output()
        .expect("daemon start should run");
    assert!(start.status.success());

    let sample_count = benchmark_sample_count();
    let mut latencies_ms = Vec::with_capacity(sample_count);

    for i in 0..sample_count {
        let t0 = Instant::now();
        let output = uxc_command()
            .arg(&endpoint)
            .arg("get:/health")
            .output()
            .expect("repeated call should run");
        let elapsed = t0.elapsed().as_millis() as u64;
        assert!(
            output.status.success(),
            "repeated call #{i} should succeed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        latencies_ms.push(elapsed);
    }

    latencies_ms.sort_unstable();

    let p50 = latencies_ms[latencies_ms.len() / 2];
    let p95_index = ((latencies_ms.len() as f64) * 0.95).ceil() as usize - 1;
    let p95 = latencies_ms[p95_index.min(latencies_ms.len() - 1)];

    eprintln!(
        "openapi repeated latency: p50={}ms p95={}ms samples={}",
        p50,
        p95,
        latencies_ms.len()
    );

    assert!(p95 < 5_000, "p95 latency should stay within sane bound");

    daemon_stop_best_effort();
}
