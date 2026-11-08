//! Accessibility tree snapshots: a flat, indexable rendering of an app's UI.

use super::ax::{CGPoint, Element};
use super::cf;

pub struct Limits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub text_limit: usize,
    pub include_frames: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_nodes: 1200,
            max_depth: 64,
            text_limit: 200,
            include_frames: false,
        }
    }
}

pub struct Node {
    pub element: Element,
    pub depth: usize,
    pub text: String,
    pub center: Option<CGPoint>,
    /// The `AXWindow` this element lives in, so an action can tell that the
    /// element moved to another window instead of acting on the wrong one.
    pub window: Option<Element>,
}

pub struct Snapshot {
    pub nodes: Vec<Node>,
    pub truncated: bool,
}

const FLAT_ROLES: [&str; 4] = ["AXGroup", "AXUnknown", "AXLayoutArea", "AXLayoutItem"];

fn truncated_text(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    let count = trimmed.chars().count();
    if count <= limit {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(limit).collect();
    format!("{head}… ({count} chars)")
}

fn describe(element: &Element, limits: &Limits, center: Option<CGPoint>) -> String {
    let role = element.role();
    let mut parts = vec![if role.is_empty() {
        "AXUnknown".to_string()
    } else {
        role.clone()
    }];

    let title = element.string_attribute("AXTitle").unwrap_or_default();
    let description = element
        .string_attribute("AXDescription")
        .or_else(|| element.string_attribute("AXHelp"))
        .unwrap_or_default();
    let identifier = element.string_attribute("AXIdentifier").unwrap_or_default();
    let label = if !title.trim().is_empty() {
        title
    } else if !description.trim().is_empty() {
        description
    } else {
        String::new()
    };
    if !label.is_empty() {
        parts.push(format!("\"{}\"", truncated_text(&label, limits.text_limit)));
    }

    if let Some(value) = element.attribute("AXValue") {
        let raw = value.raw();
        let rendered = if let Some(text) = unsafe { cf::string_from(raw) } {
            if text.trim().is_empty() {
                None
            } else {
                Some(format!("\"{}\"", truncated_text(&text, limits.text_limit)))
            }
        } else if let Some(number) = cf::number(raw) {
            Some(format!("{number}"))
        } else {
            cf::boolean(raw).map(|flag| flag.to_string())
        };
        if let Some(rendered) = rendered {
            parts.push(format!("value={rendered}"));
        }
    }

    if !identifier.trim().is_empty() {
        parts.push(format!("id={identifier}"));
    }
    if element.bool_attribute("AXFocused") == Some(true) {
        parts.push("· focused".to_string());
    }
    if element.bool_attribute("AXEnabled") == Some(false) {
        parts.push("· disabled".to_string());
    }
    if limits.include_frames {
        if let Some(center) = center {
            parts.push(format!("@({:.0},{:.0})", center.x, center.y));
        }
    }
    parts.join(" ")
}

fn walk(
    element: &Element,
    depth: usize,
    limits: &Limits,
    window: Option<&Element>,
    out: &mut Vec<Node>,
) -> bool {
    if out.len() >= limits.max_nodes || depth > limits.max_depth {
        return false;
    }
    let children = element.elements_attribute("AXChildren");
    let role = element.role();
    let center = element.center();
    let flat = FLAT_ROLES.contains(&role.as_str());
    let bound = if role == "AXWindow" {
        Some(element.retained())
    } else {
        window.map(Element::retained)
    };

    let label = describe(element, limits, center);
    let visible =
        !(flat && !label.contains('"') && !label.contains("value=") && !label.contains("id="));

    let node_depth = if flat { depth.saturating_sub(1) } else { depth };
    if visible {
        out.push(Node {
            element: element.retained(),
            depth: node_depth,
            text: label,
            center,
            window: bound.clone(),
        });
    }

    let mut complete = true;
    for child in children {
        if out.len() >= limits.max_nodes {
            return false;
        }
        if !walk(&child, depth + 1, limits, bound.as_ref(), out) {
            complete = false;
        }
    }
    complete
}

pub fn snapshot(app: &Element, limits: &Limits, window: Option<&Element>) -> Snapshot {
    let mut nodes = Vec::new();
    let complete = walk(app, 0, limits, window, &mut nodes);
    Snapshot {
        nodes,
        truncated: !complete,
    }
}

pub fn render(nodes: &[Node]) -> String {
    render_lines(nodes.iter().map(|node| (node.depth, node.text.as_str())))
}

fn render_lines<'a>(lines: impl Iterator<Item = (usize, &'a str)>) -> String {
    let mut out = String::new();
    for (index, (depth, text)) in lines.enumerate() {
        out.push_str(&format!(
            "{index:>4} {}{}\n",
            "  ".repeat(depth),
            text.replace('\n', " ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_values_are_truncated_with_their_length() {
        let text = "a".repeat(300);
        let rendered = truncated_text(&text, 10);
        assert!(rendered.starts_with("aaaaaaaaaa"));
        assert!(rendered.contains("300 chars"));
    }

    #[test]
    fn short_values_are_trimmed_but_kept() {
        assert_eq!(truncated_text("  hello  ", 10), "hello");
    }

    #[test]
    fn rendering_prefixes_each_line_with_its_index_and_indent() {
        let rendered = render_lines([(0, "AXWindow"), (1, "AXButton \"OK\"")].into_iter());
        assert_eq!(rendered, "   0 AXWindow\n   1   AXButton \"OK\"\n");
    }

    #[test]
    fn rendering_flattens_multiline_values() {
        let rendered = render_lines([(0, "AXTextArea value=a\nb")].into_iter());
        assert_eq!(rendered, "   0 AXTextArea value=a b\n");
    }
}
