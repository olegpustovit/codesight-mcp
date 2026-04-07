use std::io::{self, BufRead, Write};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::service::CodeService;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse<T> {
    jsonrpc: &'static str,
    id: Value,
    result: T,
}

pub fn run_stdio(service: &CodeService) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: RpcRequest = serde_json::from_str(&line)
            .with_context(|| format!("invalid JSON-RPC line: {line}"))?;
        match request.method.as_str() {
            "initialize" => {
                write_json(
                    &mut out,
                    RpcResponse {
                        jsonrpc: "2.0",
                        id: request.id.unwrap_or_else(|| json!(0)),
                        result: json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {
                                "tools": { "listChanged": false }
                            },
                            "serverInfo": {
                                "name": env!("CARGO_PKG_NAME"),
                                "version": env!("CARGO_PKG_VERSION")
                            }
                        }),
                    },
                )?;
            }
            "tools/list" => {
                write_json(
                    &mut out,
                    RpcResponse {
                        jsonrpc: "2.0",
                        id: request.id.unwrap_or_else(|| json!(0)),
                        result: json!({ "tools": tool_definitions() }),
                    },
                )?;
            }
            "tools/call" => {
                let params: ToolCallParams = serde_json::from_value(request.params)?;
                let payload = service.dispatch(&params.name, params.arguments)?;
                write_json(
                    &mut out,
                    RpcResponse {
                        jsonrpc: "2.0",
                        id: request.id.unwrap_or_else(|| json!(0)),
                        result: json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": serde_json::to_string(&payload)?
                                }
                            ],
                            "isError": payload.get("error").is_some()
                        }),
                    },
                )?;
            }
            "notifications/initialized" => {}
            _ => {
                eprintln!(
                    "codesight-mcp: ignoring unsupported JSON-RPC method '{}'",
                    request.method
                );
            }
        }
    }

    Ok(())
}

fn write_json<T: Serialize>(out: &mut impl Write, value: T) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, &value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn tool_definitions() -> Vec<Value> {
    serde_json::from_str(include_str!("tools.json")).expect("tools.json must be valid JSON")
}
