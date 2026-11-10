//! A stdio MCP server exposing the Computer Use tools.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};

use crate::runtime::Runtime;
use crate::tools;

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Every revision this server can answer. A client's requested revision is
/// echoed when it is one of these, because clients reject a version they did
/// not ask for.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The one resource this server publishes: a fresh capture of the screen. A
/// subscribing client is told when a call may have changed it, which is the
/// stdio equivalent of a preview that keeps up.
pub const SCREEN_URI: &str = "computer-use://screen";

pub const INSTRUCTIONS: &str = "Use this server for GUI control of macOS desktop apps and signed-in browser tasks, including filling forms and reading a screen. Tools: permissions, list_apps, get_app_state, query, click, perform_secondary_action, set_value, select_text, type_text, press_key, scroll, drag, screenshot. Call permissions for setup diagnostics in the actual MCP host, list_apps to find the target, then get_app_state before acting on indexes from its newest snapshot. Re-observe after actions and use screenshot or include_screenshot for visual verification and client previews; read the computer-use://screen resource for a fresh capture at any time, and subscribe to it to be told when a call may have changed the screen. press_key takes Return, Tab, Escape, the arrows and F1-F12; a newline inside type_text is pressed as Return, and a literal backslash-n is not a navigation key. type_text and press_key with background true deliver to one app without activating it. The app policy decides which apps are reachable: an unapproved app is refused with the exact `oxide-reins apps grant-once or apps allow` command, `apps revoke` withdraws a decision while this session keeps running, and the app hosting this server is refused unless `apps self-automation allow` says otherwise. A person moving the pointer or typing during a drag or a long type stops it: that is user takeover, so leave the desktop to them. Respect requests to leave submission to the user. Never approve OS permission dialogs yourself. If a tool search does not list these tools, the MCP connection is missing rather than the capability.";

fn negotiated_version(requested: Option<&str>) -> String {
    match requested {
        Some(version) if SUPPORTED_VERSIONS.contains(&version) => version.to_string(),
        _ => PROTOCOL_VERSION.to_string(),
    }
}

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut server = Server::new();
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
        if let Some(response) = server.handle(&request) {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        }
        for notification in server.drain() {
            writeln!(stdout, "{}", serde_json::to_string(&notification)?)?;
        }
        stdout.flush()?;
    }
    Ok(())
}

pub struct Server {
    runtime: Runtime,
    screen_subscribed: bool,
    pending: Vec<Value>,
}

impl Default for Server {
    fn default() -> Self {
        Server::new()
    }
}

impl Server {
    pub fn new() -> Server {
        Server {
            runtime: Runtime::new(),
            screen_subscribed: false,
            pending: Vec::new(),
        }
    }

    /// Notifications produced while answering the last request. The caller writes
    /// them after the response, which is the order a client expects.
    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending)
    }

    pub fn handle(&mut self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned()?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

        let outcome = match method {
            "initialize" => Ok(json!({
                "protocolVersion": negotiated_version(
                    params.get("protocolVersion").and_then(Value::as_str)
                ),
                "capabilities": {
                    "tools": { "listChanged": false },
                    "resources": { "subscribe": true, "listChanged": false }
                },
                "serverInfo": {
                    "name": "oxide-reins",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": INSTRUCTIONS
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::specs() })),
            "tools/call" => Ok(self.call_tool(&params)),
            "resources/list" => Ok(json!({
                "resources": [{
                    "uri": SCREEN_URI,
                    "name": "Screen",
                    "description": "A fresh capture of the screen as a PNG. Read it for a current view; subscribe to be told when a call may have changed what is on screen.",
                    "mimeType": "image/png"
                }]
            })),
            "resources/subscribe" => self.subscribe(&params, true),
            "resources/unsubscribe" => self.subscribe(&params, false),
            "resources/read" => self.read_resource(&params),
            other => {
                Err(json!({ "code": -32601, "message": format!("method not found: {other}") }))
            }
        };

        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        })
    }

    fn call_tool(&mut self, params: &Value) -> Value {
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
        let result = contained(&format!("tool `{name}`"), || {
            tools::execute(&mut self.runtime, name, &args)
        });
        // A failed action may still have moved something, so the subscription
        // follows the attempt, not the outcome.
        let touched = match &result {
            Ok(outcome) => !outcome.images.is_empty() || !tools::is_read_only(name),
            Err(_) => !tools::is_read_only(name),
        };
        if touched {
            self.notify_screen();
        }
        match result {
            Ok(outcome) => tools::content_json(&outcome),
            Err(error) => json!({
                "content": [{ "type": "text", "text": format!("{error:#}") }],
                "isError": true
            }),
        }
    }

    fn notify_screen(&mut self) {
        if !self.screen_subscribed {
            return;
        }
        self.pending.push(json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/updated",
            "params": { "uri": SCREEN_URI }
        }));
    }

    fn subscribe(&mut self, params: &Value, subscribed: bool) -> std::result::Result<Value, Value> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if uri != SCREEN_URI {
            return Err(unknown_resource(uri));
        }
        self.screen_subscribed = subscribed;
        if subscribed {
            self.notify_screen();
        }
        Ok(json!({}))
    }

    fn read_resource(&mut self, params: &Value) -> std::result::Result<Value, Value> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if uri != SCREEN_URI {
            return Err(unknown_resource(uri));
        }
        match contained("reading the screen resource", || {
            tools::execute(&mut self.runtime, "screenshot", &json!({}))
        }) {
            Ok(outcome) => Ok(json!({ "contents": contents(outcome) })),
            Err(error) => Err(json!({
                "code": -32603,
                "message": format!("{error:#}")
            })),
        }
    }
}

fn unknown_resource(uri: &str) -> Value {
    json!({
        "code": -32602,
        "message": format!("no resource `{uri}`; the only one this server publishes is {SCREEN_URI}")
    })
}

fn contents(outcome: tools::Outcome) -> Vec<Value> {
    outcome
        .images
        .into_iter()
        .map(|bytes| {
            json!({
                "uri": SCREEN_URI,
                "mimeType": "image/png",
                "blob": BASE64.encode(bytes)
            })
        })
        .collect()
}

/// One call cannot end the connection: a panic inside a tool is answered as an
/// error result and the next request is still served. The runtime's element cache
/// is rebuilt by the next observation, and every indexed action already refuses
/// an element that is no longer bound to its window, so a stale cache fails loudly
/// instead of acting on the wrong element.
fn contained<T>(what: &str, call: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(result) => result,
        Err(payload) => Err(anyhow::anyhow!(
            "{what} stopped with an internal error ({}). The server is still running; call get_app_state again before the next action",
            panic_text(payload.as_ref())
        )),
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "no message".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> Option<Value> {
        Server::new().handle(&value)
    }

    fn subscribe(server: &mut Server) {
        let response = server
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "resources/subscribe",
                "params": { "uri": SCREEN_URI }
            }))
            .expect("subscribe is answered");
        assert!(response.get("error").is_none(), "{response}");
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
        assert_eq!(
            response["result"]["capabilities"]["resources"]["subscribe"],
            json!(true)
        );
    }

    #[test]
    fn the_screen_resource_is_listed_and_readable_by_uri() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "resources/list",
            "params": {}
        }))
        .expect("resources/list is answered");
        let resources = response["result"]["resources"]
            .as_array()
            .expect("resources is an array");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0]["uri"], json!(SCREEN_URI));
        assert_eq!(resources[0]["mimeType"], json!("image/png"));

        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/read",
            "params": { "uri": "computer-use://no-such-screen" }
        }))
        .expect("an unknown resource is still answered");
        assert_eq!(response["error"]["code"], json!(-32602));
        let message = response["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains(SCREEN_URI), "{message}");
    }

    #[test]
    fn a_capture_becomes_a_png_block_a_client_can_render() {
        let blank = contents(tools::Outcome {
            text: "Captured a screenshot.".to_string(),
            images: vec![b"png".to_vec()],
        });
        assert_eq!(blank.len(), 1);
        assert_eq!(blank[0]["uri"], json!(SCREEN_URI));
        assert_eq!(blank[0]["mimeType"], json!("image/png"));
        assert_eq!(blank[0]["blob"], json!("cG5n"));
    }

    #[test]
    fn a_subscribed_client_is_told_when_a_call_may_have_changed_the_screen() {
        let mut server = Server::new();
        subscribe(&mut server);
        assert_eq!(
            server.drain().len(),
            1,
            "subscribing reports the first view"
        );

        server.handle(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "permissions", "arguments": {} }
        }));
        assert!(
            server.drain().is_empty(),
            "reading permissions changes nothing on screen"
        );

        server.handle(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "click", "arguments": { "app": "TextEdit" } }
        }));
        let notifications = server.drain();
        assert_eq!(notifications.len(), 1, "{notifications:?}");
        assert_eq!(
            notifications[0]["method"],
            json!("notifications/resources/updated")
        );
        assert_eq!(notifications[0]["params"]["uri"], json!(SCREEN_URI));
    }

    #[test]
    fn unsubscribing_stops_the_notifications() {
        let mut server = Server::new();
        subscribe(&mut server);
        server.drain();
        let response = server
            .handle(&json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "resources/unsubscribe",
                "params": { "uri": SCREEN_URI }
            }))
            .expect("unsubscribe is answered");
        assert!(response.get("error").is_none(), "{response}");
        server.handle(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": { "name": "click", "arguments": { "app": "TextEdit" } }
        }));
        assert!(server.drain().is_empty());
    }

    #[test]
    fn the_instructions_name_every_tool_and_the_policy_commands() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": PROTOCOL_VERSION }
        }))
        .expect("initialize is answered");
        let instructions = response["result"]["instructions"]
            .as_str()
            .expect("initialize carries instructions");
        for name in tools::names() {
            assert!(
                instructions.contains(&name),
                "the instructions do not mention {name}: {instructions}"
            );
        }
        for hint in ["grant-once", "apps allow", "apps revoke", "takeover"] {
            assert!(
                instructions.contains(hint),
                "the instructions do not explain {hint}: {instructions}"
            );
        }
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

    #[test]
    fn a_call_that_panics_is_answered_and_the_server_keeps_serving() {
        let error =
            contained::<()>("tool `demo`", || panic!("boom")).expect_err("a panic is an error");
        let message = format!("{error:#}");
        assert!(message.contains("tool `demo`"), "{message}");
        assert!(message.contains("boom"), "{message}");
        assert!(message.contains("get_app_state"), "{message}");

        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/list",
            "params": {}
        }))
        .expect("the server still answers");
        assert!(response["result"]["tools"].is_array());
    }

    #[test]
    fn a_contained_call_passes_its_value_through() {
        assert_eq!(contained("demo", || Ok(7)).unwrap(), 7);
    }
}
