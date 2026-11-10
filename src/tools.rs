//! The Computer Use tool surface shared by the MCP server and the CLI.

use anyhow::{bail, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};

use crate::runtime::{Runtime, State};

pub struct Outcome {
    pub text: String,
    pub images: Vec<Vec<u8>>,
}

const APP: &str = "Running app name (or pid) exactly as `list_apps` reports it";
const WINDOW: &str =
    "Bind to one window: a zero-based index or a title substring, as listed by get_app_state";
const MAX_OUTPUT: &str = "Cap on the reply text in characters (default 20000)";

fn readonly(properties: Value, required: &[&str], description: &str, name: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "annotations": { "readOnlyHint": true, "destructiveHint": false },
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

fn action(
    properties: Value,
    required: &[&str],
    description: &str,
    name: &str,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": idempotent },
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

pub fn specs() -> Value {
    json!([
        readonly(json!({}), &[],
            "Check Accessibility and Screen Recording permissions in this MCP process, with the launching app and setup instructions. Does not request or grant permissions. Use it before driving the desktop, or when a GUI action fails with a permission error.",
            "permissions"
        ),
        readonly(
            json!({
                "include_helpers": { "type": "boolean", "description": "Also show window-server helpers such as a native file picker that is not part of its app." }
            }),
            &[],
            "List the running desktop apps that own windows, with their pid, window titles and the app policy marker on each. Use it to find the app to drive, native or browser, and to see which app still needs approval.",
            "list_apps"
        ),
        readonly(
            json!({
                "app": { "type": "string", "description": APP },
                "window": { "type": "string", "description": WINDOW },
                "text_limit": { "description": "Characters shown per text value; a number, or \"max\" for full values." },
                "max_tree_nodes": { "type": "integer", "description": "Node budget before the tree is reported truncated (default 1200)." },
                "max_tree_depth": { "type": "integer", "description": "Depth limit before the tree is reported truncated (default 64)." },
                "include_frames": { "type": "boolean", "description": "Append each element's on-screen center, for coordinate actions." },
                "include_screenshot": { "type": "boolean", "description": "Attach one PNG of the same window, for when the tree is not enough." },
                "max_output_chars": { "type": "integer", "description": MAX_OUTPUT }
            }),
            &["app"],
            "Read one app's accessibility tree as indexed lines. Element indexes from the newest snapshot are what click, set_value, select_text, perform_secondary_action, drag and scroll take, and they are bound to the window they came from. This is the GUI element listing behind every later click or type.",
            "get_app_state"
        ),
        readonly(
            json!({
                "app": { "type": "string", "description": APP },
                "window": { "type": "string", "description": WINDOW },
                "text": { "type": "string", "description": "Case-insensitive substring of role, title, description or value." },
                "role": { "type": "string", "description": "Case-insensitive role filter, such as AXButton." },
                "max_results": { "type": "integer", "description": "Default 20." },
                "max_tree_nodes": { "type": "integer", "description": "Node budget while searching (default 1200)." },
                "max_output_chars": { "type": "integer", "description": MAX_OUTPUT }
            }),
            &["app"],
            "Search one app's accessibility tree by text and/or role without printing the whole tree. Refreshes the element indexes. Use it to find a button, field or menu item in a long desktop or browser window.",
            "query"
        ),
        action(
            json!({
                "app": { "type": "string", "description": APP },
                "element_index": { "type": "integer", "description": "Index from the newest get_app_state or query." },
                "x": { "type": "number" },
                "y": { "type": "number" },
                "button": { "type": "string", "enum": ["left", "right"], "description": "Default left." }
            }),
            &["app"],
            "Click a GUI element: a button, link, menu item, checkbox or tab. Left-click uses AXPress when available; right-click and other clicks use pointer events at the element center (or x/y). The reply says which path ran.",
            "click",
            false
        ),
        action(
            json!({
                "app": { "type": "string", "description": APP },
                "element_index": { "type": "integer" },
                "action": { "type": "string", "description": "Default AXPress; an unsupported action is refused with the list of actions the element does support." }
            }),
            &["app", "element_index"],
            "Perform a named accessibility action on a GUI element, such as AXShowMenu, AXIncrement or AXConfirm. Use it for a context menu or a stepper that has no clickable target.",
            "perform_secondary_action",
            false
        ),
        action(
            json!({
                "app": { "type": "string", "description": APP },
                "element_index": { "type": "integer" },
                "value": { "type": "string" }
            }),
            &["app", "element_index", "value"],
            "Write a string into an editable element's AXValue and read it back. A control that is not settable, or that ignores the write, fails instead of reporting success. Use it to fill a text field or form input without focusing the app or touching the keyboard.",
            "set_value",
            true
        ),
        action(
            json!({
                "app": { "type": "string", "description": APP },
                "element_index": { "type": "integer", "description": "Defaults to the app's focused text element." },
                "text": { "type": "string", "description": "The text to select inside the element." }
            }),
            &["app", "text"],
            "Select a piece of text inside a text element, for reading or for a following action such as copy. Use it in a document or a text field before pressing a keyboard shortcut.",
            "select_text",
            true
        ),
        action(
            json!({
                "text": { "type": "string" },
                "app": { "type": "string", "description": APP },
                "element_index": { "type": "integer" },
                "background": { "type": "boolean", "description": "Deliver the keys to this app's process without activating it or touching the real keyboard focus. Off by default: the desktop changes hands only when asked." }
            }),
            &["text"],
            "Type text through the keyboard into an app: the focused control, after activating the app, or with background true, that app's process alone. A newline inside the text is pressed as Return. Use it for a search box, form field or terminal.",
            "type_text",
            false
        ),
        action(
            json!({
                "key": { "type": "string" },
                "modifiers": { "type": "string", "description": "For example \"cmd+shift\"." },
                "background": { "type": "boolean", "description": "Deliver the key to this app's process without activating it. Off by default." },
                "app": { "type": "string", "description": APP }
            }),
            &["key"],
            "Press one keyboard key, optionally with modifiers, in a GUI app, on the real desktop or with background true to that app's process alone. Key names: Return, Tab, Escape, Space, Delete, Forward_Delete, Home, End, Page_Up, Page_Down, Left, Right, Up, Down, F1-F12 or a single character; the spellings \"\\n\", \"\\t\" and \"\\r\" mean Return and Tab. Modifiers: cmd, shift, alt, ctrl. Use it to submit a form, dismiss a dialog or move between fields.",
            "press_key",
            false
        ),
        action(
            json!({
                "app": { "type": "string", "description": APP },
                "direction": { "type": "string", "enum": ["up", "down"], "description": "Default down." },
                "lines": { "type": "integer", "description": "Default 3." },
                "element_index": { "type": "integer" },
                "x": { "type": "number" },
                "y": { "type": "number" }
            }),
            &[],
            "Scroll a page, list or window with the mouse wheel: the app's frontmost window, or the pointer position when x/y or element_index is given. No snapshot is required.",
            "scroll",
            false
        ),
        action(
            json!({
                "app": { "type": "string", "description": APP },
                "element_index": { "type": "integer" },
                "from_x": { "type": "number" },
                "from_y": { "type": "number" },
                "to_x": { "type": "number" },
                "to_y": { "type": "number" }
            }),
            &["to_x", "to_y"],
            "Drag with the left mouse button from an element center or from_x/from_y to to_x/to_y: a window, a slider or a text selection. Paced pointer events, so it moves windows and selects text rather than posting one lost event. A person's input stops it.",
            "drag",
            false
        ),
        readonly(
            json!({
                "app": { "type": "string", "description": "Omit to capture the whole screen." },
                "window": { "type": "string", "description": "With app: an index or title substring to pick the window." }
            }),
            &[],
            "Capture a screenshot of the screen, or one app's window, as a PNG image. Needs Screen Recording permission. Use it to see a GUI that the accessibility tree does not describe; it is a snapshot, not a live stream.",
            "screenshot"
        )
    ])
}

/// Whether a tool only reads. A call that is not read-only may have changed the
/// screen, which is what a subscribed client is told about.
pub fn is_read_only(name: &str) -> bool {
    specs()
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"].as_str() == Some(name))
        })
        .and_then(|tool| tool["annotations"]["readOnlyHint"].as_bool())
        .unwrap_or(false)
}

pub fn execute(runtime: &mut Runtime, name: &str, args: &Value) -> Result<Outcome> {
    if name == "screenshot" {
        let bytes = runtime.screenshot(args)?;
        return Ok(Outcome {
            text: format!(
                "Captured a screenshot ({} KB PNG).{}",
                bytes.len() / 1024,
                if args.get("app").is_some() {
                    ""
                } else {
                    " It covers the whole screen."
                }
            ),
            images: vec![bytes],
        });
    }

    if name == "get_app_state" {
        let State { text, image } = runtime.get_app_state(args)?;
        return Ok(Outcome {
            text,
            images: image.into_iter().collect(),
        });
    }

    let text = match name {
        "list_apps" => runtime.list_apps(
            args.get("include_helpers")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )?,
        "permissions" => serde_json::to_string_pretty(&crate::permissions::status())?,
        "query" => runtime.query(args)?,
        "click" => runtime.click(args)?,
        "perform_secondary_action" => runtime.perform_action(args)?,
        "set_value" => runtime.set_value(args)?,
        "select_text" => runtime.select_text(args)?,
        "type_text" => runtime.type_text(args)?,
        "press_key" => runtime.press_key(args)?,
        "scroll" => runtime.scroll(args)?,
        "drag" => runtime.drag(args)?,
        other => bail!("unknown tool `{other}`; call tools/list for the available tools"),
    };
    Ok(Outcome {
        text,
        images: Vec::new(),
    })
}

/// The MCP `CallToolResult` for an outcome: text first, then any images.
pub fn content_json(outcome: &Outcome) -> Value {
    let mut content = vec![json!({ "type": "text", "text": outcome.text })];
    for image in &outcome.images {
        content.push(json!({
            "type": "image",
            "data": BASE64.encode(image),
            "mimeType": "image/png"
        }));
    }
    json!({ "content": content, "isError": false })
}

pub fn names() -> Vec<String> {
    specs()
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_declares_a_description_a_schema_and_annotations() {
        let tools = specs();
        let tools = tools.as_array().expect("specs is an array");
        assert_eq!(tools.len(), 13);
        for tool in tools {
            let name = tool["name"].as_str().expect("a tool has a name");
            assert!(
                tool["description"].as_str().unwrap_or_default().len() > 20,
                "{name} needs a usable description"
            );
            assert_eq!(tool["inputSchema"]["type"], json!("object"), "{name}");
            assert!(
                tool["annotations"]["readOnlyHint"].is_boolean(),
                "{name} needs a read-only hint so clients can approve it sensibly"
            );
        }
    }

    #[test]
    fn observers_are_read_only_and_actions_are_not() {
        let tools = specs();
        let tools = tools.as_array().expect("specs is an array");
        for tool in tools {
            let name = tool["name"].as_str().unwrap_or_default();
            let read_only = tool["annotations"]["readOnlyHint"]
                .as_bool()
                .unwrap_or(false);
            let expected = matches!(
                name,
                "permissions" | "list_apps" | "get_app_state" | "query" | "screenshot"
            );
            assert_eq!(read_only, expected, "{name} has the wrong read-only hint");
        }
    }

    #[test]
    fn every_tool_is_documented() {
        let tools_doc = include_str!("../docs/tools.md");
        let readme = include_str!("../README.md");
        for name in names() {
            assert!(
                tools_doc.contains(&format!("### `{name}`")),
                "docs/tools.md needs a section for {name}"
            );
            assert!(
                readme.contains(&format!("| `{name}` |")),
                "README.md needs a table row for {name}"
            );
        }
    }

    #[test]
    fn every_description_carries_the_words_an_agent_searches_for() {
        let vocabulary = [
            "gui",
            "desktop",
            "browser",
            "screenshot",
            "screen",
            "pointer",
            "mouse",
            "keyboard",
            "click",
            "type",
            "scroll",
            "drag",
            "element",
        ];
        for tool in specs().as_array().expect("specs is an array") {
            let name = tool["name"].as_str().expect("a tool has a name");
            let description = tool["description"].as_str().unwrap_or_default();
            let lower = description.to_lowercase();
            assert!(
                vocabulary.iter().any(|word| lower.contains(word)),
                "{name} names no app, element or input a search could match: {description}"
            );
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let mut names = names();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), specs().as_array().map(Vec::len).unwrap_or(0));
    }

    #[test]
    fn a_text_only_outcome_is_one_text_part() {
        let outcome = Outcome {
            text: "Clicked element 4.".to_string(),
            images: Vec::new(),
        };
        let value = content_json(&outcome);
        assert_eq!(value["isError"], json!(false));
        assert_eq!(value["content"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["content"][0]["type"], json!("text"));
        assert_eq!(value["content"][0]["text"], json!("Clicked element 4."));
    }

    #[test]
    fn an_image_outcome_adds_a_png_part_after_the_text() {
        let outcome = Outcome {
            text: "Captured a screenshot.".to_string(),
            images: vec![b"png".to_vec()],
        };
        let value = content_json(&outcome);
        assert_eq!(value["content"][0]["type"], json!("text"));
        assert_eq!(value["content"][1]["type"], json!("image"));
        assert_eq!(value["content"][1]["mimeType"], json!("image/png"));
        assert_eq!(value["content"][1]["data"], json!("cG5n"));
    }
}
