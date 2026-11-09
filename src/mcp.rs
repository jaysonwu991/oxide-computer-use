//! A stdio MCP server exposing the Computer Use tools.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::runtime::Runtime;
use crate::tools;

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Every revision this server can answer. A client's requested revision is
/// echoed when it is one of these, because clients reject a version they did
/// not ask for.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

fn negotiated_version(requested: Option<&str>) -> String {
    match requested {
        Some(version) if SUPPORTED_VERSIONS.contains(&version) => version.to_string(),
        _ => PROTOCOL_VERSION.to_string(),
    }
}

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut runtime = Runtime::new();
    for line in stdin.lock().lines() {
        let line = line.context("reading a request from stdin")?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let failure = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {error}") }
                });
                writeln!(stdout, "{failure}")?;
                stdout.flush()?;
                continue;
            }
        };
        let Some(response) = handle(&mut runtime, &request) else {
            continue;
        };
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(runtime: &mut Runtime, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let id = id?;

    let outcome = match method {
        "initialize" => Ok(json!({
            "protocolVersion": negotiated_version(
                params.get("protocolVersion").and_then(Value::as_str)
            ),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "oxide-reins",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Use this server for native apps and signed-in browser tasks, including filling forms. Call permissions for setup diagnostics in the actual MCP host, list_apps to find the target, then get_app_state before acting on indexes from its newest snapshot. Re-observe after actions and use screenshot or include_screenshot for visual verification and client previews. Screenshots are snapshots, not a live stream. Respect requests to leave submission to the user. AXPress and set_value can work in the background; keyboard and pointer input activate the app and use the real desktop. Never approve OS permission dialogs yourself."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::specs() })),
        "tools/call" => Ok(call_tool(runtime, &params)),
        other => Err(json!({ "code": -32601, "message": format!("method not found: {other}") })),
    };

    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

fn call_tool(runtime: &mut Runtime, params: &Value) -> Value {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => {
            return json!({
                "content": [{ "type": "text", "text": "`name` is required for tools/call" }],
                "isError": true
            })
        }
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match tools::execute(runtime, name, &args) {
        Ok(outcome) => tools::content_json(&outcome),
        Err(error) => json!({
            "content": [{ "type": "text", "text": format!("{error:#}") }],
            "isError": true
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> Option<Value> {
        let mut runtime = Runtime::new();
        handle(&mut runtime, &value)
    }

    #[test]
    fn initialize_reports_protocol_and_capabilities() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {} }
        }))
        .expect("initialize is answered");
        assert_eq!(response["jsonrpc"], json!("2.0"));
        assert_eq!(response["id"], json!(1));
        assert_eq!(
            response["result"]["protocolVersion"],
            json!(PROTOCOL_VERSION)
        );
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            json!("oxide-reins")
        );
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn a_client_version_we_support_is_echoed() {
        for version in SUPPORTED_VERSIONS {
            let response = request(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": version }
            }))
            .expect("initialize is answered");
            assert_eq!(response["result"]["protocolVersion"], json!(version));
        }
    }

    #[test]
    fn an_unknown_version_gets_the_newest_we_speak() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "1999-01-01" }
        }))
        .expect("initialize is answered");
        assert_eq!(
            response["result"]["protocolVersion"],
            json!(PROTOCOL_VERSION)
        );
    }

    #[test]
    fn notifications_are_not_answered() {
        assert!(request(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .is_none());
    }

    #[test]
    fn tools_list_exposes_the_computer_use_surface() {
        let response = request(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .expect("tools/list is answered");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools is an array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(names, tools::names());
        assert!(names.contains(&"get_app_state"));
        assert!(names.contains(&"press_key"));
    }

    #[test]
    fn unknown_method_is_a_protocol_error() {
        let response = request(json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" }))
            .expect("unknown methods are answered");
        assert_eq!(response["error"]["code"], json!(-32601));
        assert!(response.get("result").is_none());
    }

    #[test]
    fn tool_failures_are_results_not_protocol_errors() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "no_such_tool", "arguments": {} }
        }))
        .expect("a failing tool is still answered");
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["isError"], json!(true));
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(text.contains("unknown tool"), "unexpected text: {text}");
    }

    #[test]
    fn a_missing_tool_name_is_reported_as_a_result() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "arguments": {} }
        }))
        .expect("the request is answered");
        assert_eq!(response["result"]["isError"], json!(true));
    }
}
