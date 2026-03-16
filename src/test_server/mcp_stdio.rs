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
    let mut tools_list_calls: u64 = 0;
    let mut dynamic_tools_enabled = false;
    let mut resource_subscribed = false;

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

        if matches!(scenario, Scenario::Timeout)
            || (matches!(scenario, Scenario::ToolCallTimeout) && method == "tools/call")
        {
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
                let list_changed = matches!(scenario, Scenario::DynamicToolset);
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {
                                "tools": {"listChanged": list_changed},
                                "resources": {"subscribe": true}
                            },
                            "serverInfo": {"name": "uxc-test-mcp-stdio", "version": "1.0.0"},
                            "instructions": "MCP stdio test server for local e2e"
                        }
                    }),
                )?;
            }
            "tools/list" => {
                tools_list_calls = tools_list_calls.saturating_add(1);
                if matches!(scenario, Scenario::ToolsListFailAfterFirst) && tools_list_calls > 1 {
                    respond(
                        &mut out,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {"code": -32002, "message": "tools/list failed after first request"}
                        }),
                    )?;
                    continue;
                }
                if matches!(scenario, Scenario::DynamicToolset) {
                    let tools = if dynamic_tools_enabled {
                        json!([
                            {
                                "name": "navigate",
                                "description": "Navigate to another page",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": {"type": "string"}
                                    },
                                    "required": ["path"]
                                }
                            },
                            {
                                "name": "graph3d_render",
                                "description": "Render a 3D graph on the current page",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "expression": {"type": "string"}
                                    },
                                    "required": ["expression"]
                                }
                            }
                        ])
                    } else {
                        json!([
                            {
                                "name": "navigate",
                                "description": "Navigate to another page",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": {"type": "string"}
                                    },
                                    "required": ["path"]
                                }
                            },
                            {
                                "name": "home_status",
                                "description": "Inspect the home page state",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {}
                                }
                            }
                        ])
                    };
                    respond(
                        &mut out,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "tools": tools }
                        }),
                    )?;
                    continue;
                }
                if matches!(scenario, Scenario::EmptyObjectRequired) {
                    respond(
                        &mut out,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": [
                                    {
                                        "name": "empty_check",
                                        "description": "Require an explicit empty object input",
                                        "inputSchema": {
                                            "type": "object",
                                            "properties": {}
                                        }
                                    }
                                ]
                            }
                        }),
                    )?;
                    continue;
                }
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

                if matches!(scenario, Scenario::EmptyObjectRequired) {
                    let arguments = req.get("params").and_then(|v| v.get("arguments"));
                    let is_object = arguments.map(Value::is_object).unwrap_or(false);
                    if !is_object {
                        respond(
                            &mut out,
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {"code": -32602, "message": "arguments object required"}
                            }),
                        )?;
                        continue;
                    }

                    respond(
                        &mut out,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [
                                    {"type": "text", "text": "received-empty-object"}
                                ],
                                "structuredContent": {
                                    "hasArgumentsObject": true,
                                    "argumentCount": arguments
                                        .and_then(Value::as_object)
                                        .map(|obj| obj.len())
                                        .unwrap_or_default()
                                }
                            }
                        }),
                    )?;
                    continue;
                }

                let message = req
                    .get("params")
                    .and_then(|v| v.get("arguments"))
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("hello");

                if matches!(scenario, Scenario::DynamicToolset) {
                    let name = req
                        .get("params")
                        .and_then(|v| v.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();

                    match name {
                        "navigate" => {
                            dynamic_tools_enabled = true;
                            respond(
                                &mut out,
                                json!({
                                    "jsonrpc": "2.0",
                                    "method": "notifications/tools/list_changed"
                                }),
                            )?;
                            respond(
                                &mut out,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [
                                            {"type": "text", "text": "navigated"}
                                        ]
                                    }
                                }),
                            )?;
                            continue;
                        }
                        "graph3d_render" => {
                            respond(
                                &mut out,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [
                                            {"type": "text", "text": "rendered"}
                                        ]
                                    }
                                }),
                            )?;
                            continue;
                        }
                        "home_status" => {
                            respond(
                                &mut out,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": {"code": -32601, "message": "Tool not found"}
                                }),
                            )?;
                            continue;
                        }
                        _ => {}
                    }
                }

                let mut result = json!({
                    "content": [
                        {"type": "text", "text": message}
                    ]
                });
                if matches!(scenario, Scenario::StructuredContent) {
                    result["structuredContent"] = json!({
                        "message": message,
                        "source": "mcp-stdio",
                        "length": message.len()
                    });
                }

                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }),
                )?;
            }
            "resources/subscribe" => {
                resource_subscribed = true;
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }),
                )?;
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/resources/updated",
                        "params": {
                            "uri": req.get("params").and_then(|v| v.get("uri")).cloned().unwrap_or(json!("unknown")),
                            "value": 1
                        }
                    }),
                )?;
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/resources/updated",
                        "params": {
                            "uri": req.get("params").and_then(|v| v.get("uri")).cloned().unwrap_or(json!("unknown")),
                            "value": 2
                        }
                    }),
                )?;
            }
            "resources/unsubscribe" => {
                resource_subscribed = false;
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }),
                )?;
            }
            "resources/list" => {
                let resources = if resource_subscribed {
                    json!([
                        {
                            "uri": "test://resource",
                            "name": "test-resource",
                            "description": "A subscribed test resource"
                        }
                    ])
                } else {
                    json!([
                        {
                            "uri": "test://resource",
                            "name": "test-resource",
                            "description": "A test resource"
                        }
                    ])
                };
                respond(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "resources": resources
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
