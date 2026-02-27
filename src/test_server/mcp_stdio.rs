//! MCP stdio test server for E2E testing

use super::common::Scenario;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn respond(out: &mut dyn Write, value: Value) -> Result<()> {
    writeln!(out, "{}", serde_json::to_string(&value)?)?;
    out.flush()?;
    Ok(())
}

pub fn run(scenario: Scenario) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if req.get("id").is_none() {
            // Notification
            continue;
        }

        let id = req.get("id").cloned().unwrap_or(json!(null));

        if matches!(scenario, Scenario::Timeout) {
            std::thread::sleep(super::common::timeout_duration());
            respond(
                &mut out,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32000, "message": "timeout"}
                }),
            )?;
            continue;
        }

        if matches!(scenario, Scenario::AuthRequired)
            && method != "initialize"
            && method != "notifications/initialized"
        {
            respond(
                &mut out,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32001, "message": "Unauthorized"}
                }),
            )?;
            continue;
        }

        match method {
            "initialize" => {
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {"listChanged": false}},
                            "serverInfo": {"name": "uxc-test-mcp-stdio", "version": "1.0.0"}
                        }
                    }),
                )?;
            }
            "tools/list" => {
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tools": [
                                {
                                    "name": "echo",
                                    "description": "Echo text back",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "message": {"type": "string"}
                                        },
                                        "required": ["message"]
                                    }
                                }
                            ]
                        }
                    }),
                )?;
            }
            "tools/call" => {
                if matches!(scenario, Scenario::Malformed) {
                    writeln!(out, "{{bad-json")?;
                    out.flush()?;
                    return Ok(());
                }

                let message = req
                    .get("params")
                    .and_then(|v| v.get("arguments"))
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("hello");

                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [
                                {"type": "text", "text": message}
                            ]
                        }
                    }),
                )?;
            }
            _ => {
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "Method not found"}
                    }),
                )?;
            }
        }
    }

    Ok(())
}

pub fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = if args.len() > 1 {
        Scenario::from_str(&args[1])?
    } else {
        Scenario::Ok
    };

    run(scenario)
}
