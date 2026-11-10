//! macOS oxide-reins backend: accessibility snapshots plus synthetic input.

pub mod apps;
pub mod ax;
pub mod cf;
pub mod input;
pub mod screen;
pub mod takeover;
pub mod tree;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::policy::{self, Policy};
use crate::runtime::State;

use ax::{CFRange, CGPoint, Element};
use tree::{Limits, Snapshot};

const DEFAULT_MAX_OUTPUT_CHARS: usize = 20_000;

struct Cached {
    element: Element,
    center: Option<CGPoint>,
    role: String,
    window: Option<Element>,
}

pub struct Runtime {
    nodes: Vec<Cached>,
    cached_app: Option<String>,
    policy: Policy,
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime {
            nodes: Vec::new(),
            cached_app: None,
            policy: Policy::load(),
        }
    }
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

/// A model writes a newline as `\n` inside a tool call often enough that typing
/// it literally is worth naming, so the next call uses a key instead.
fn literal_escape_note(text: &str) -> Option<String> {
    let escaped: Vec<&str> = ["\\n", "\\t", "\\r"]
        .into_iter()
        .filter(|sequence| text.contains(sequence))
        .collect();
    if escaped.is_empty() {
        return None;
    }
    Some(format!(
        "The text contains {} as literal characters and they were typed as written. A real newline in `text` is sent as the Return key; `press_key` with Return or Tab moves between fields.",
        escaped.join(", ")
    ))
}

fn describe_typing(typed: &input::Typed, text: &str) -> String {
    let mut reply = format!(
        "Typed {} characters{}.",
        typed.characters,
        match typed.line_breaks {
            0 => String::new(),
            1 => " and one line break with Return".to_string(),
            count => format!(" and {count} line breaks with Return"),
        }
    );
    if let Some(note) = literal_escape_note(text) {
        reply.push_str(&format!("\n{note}"));
    }
    if let Some(kind) = typed.takeover {
        reply.push_str(&format!(
            "\n{kind} arrived from a person while typing, so the rest of the text was not sent. That is user takeover: leave the desktop to them and ask what is next."
        ));
    }
    reply
}

fn required_app(args: &Value) -> Result<String> {
    string_arg(args, "app")
        .ok_or_else(|| anyhow!("`app` is required; call list_apps to see the running apps"))
}

fn number_arg(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(|value| match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    })
}

fn integer_arg(args: &Value, key: &str) -> Option<i64> {
    number_arg(args, key).map(|value| value as i64)
}

fn usize_arg(args: &Value, key: &str) -> Option<usize> {
    integer_arg(args, key).and_then(|value| usize::try_from(value).ok())
}

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn text_limit(args: &Value) -> Option<usize> {
    match args.get("text_limit") {
        Some(Value::Number(number)) => number.as_u64().map(|value| value as usize),
        Some(Value::String(text)) if text.eq_ignore_ascii_case("max") => Some(usize::MAX),
        _ => None,
    }
}

/// Keeps one reply inside an agent's context budget, and says so when it does.
fn cap_output(text: String, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text;
    }
    let kept: String = text.chars().take(limit).collect();
    format!(
        "{kept}\n\n[output truncated at {limit} of {count} characters; narrow it with `query`, a lower `max_tree_nodes`, or a higher max_output_chars]"
    )
}

/// Character range of `needle` in `haystack`, counted the way Cocoa text
/// ranges are: UTF-16 code units.
pub fn find_range(haystack: &str, needle: &str) -> Option<CFRange> {
    if needle.is_empty() {
        return None;
    }
    let units: Vec<u16> = haystack.encode_utf16().collect();
    let target: Vec<u16> = needle.encode_utf16().collect();
    if target.len() > units.len() {
        return None;
    }
    for start in 0..=units.len() - target.len() {
        if units[start..start + target.len()] == target[..] {
            return Some(CFRange {
                location: start as isize,
                length: target.len() as isize,
            });
        }
    }
    let lower_units: Vec<u16> = haystack.to_lowercase().encode_utf16().collect();
    let lower_target: Vec<u16> = needle.to_lowercase().encode_utf16().collect();
    if lower_target.len() != target.len() || lower_units.len() != units.len() {
        return None;
    }
    for start in 0..=lower_units.len() - lower_target.len() {
        if lower_units[start..start + lower_target.len()] == lower_target[..] {
            return Some(CFRange {
                location: start as isize,
                length: lower_target.len() as isize,
            });
        }
    }
    None
}

fn describe_windows(windows: &[Element]) -> String {
    let described: Vec<String> = windows
        .iter()
        .enumerate()
        .map(|(index, window)| {
            let title = window.string_attribute("AXTitle").unwrap_or_default();
            format!("{index} \"{}\"", title.trim())
        })
        .collect();
    described.join(", ")
}

impl Runtime {
    pub fn new() -> Self {
        Runtime::default()
    }

    fn resolved(&self, name: &str) -> Result<apps::App> {
        let app = apps::resolve(name).ok_or_else(|| {
            let known = apps::app_names();
            let hint = if known.is_empty() {
                "no app with a window is running".to_string()
            } else {
                format!("running apps: {}", known.join(", "))
            };
            anyhow!("no running app matches `{name}`; {hint}")
        })?;
        let pid = app.pid;
        self.policy.authorize(&app.name, || apps::identity(pid))?;
        Ok(app)
    }

    /// Only a decision pinned to a bundle identity is worth reading the running
    /// app's identity for: the common path is a name match on an empty policy.
    fn identity_for(&self, app: &apps::App) -> Option<policy::Identity> {
        if self.policy.needs_identity(&app.name) {
            apps::identity(app.pid)
        } else {
            None
        }
    }

    fn access_marker(&self, app: &apps::App) -> &'static str {
        let identity = self.identity_for(app);
        self.policy.access(&app.name, identity.as_ref()).marker()
    }

    fn app_element(app: &apps::App) -> Result<Element> {
        ax::application(app.pid)
            .ok_or_else(|| anyhow!("could not attach to {} ({})", app.name, app.pid))
    }

    /// `None` means the whole app; `Some` binds the snapshot to one window so an
    /// index can never silently address a sibling window.
    fn select_window(app_element: &Element, spec: Option<&str>) -> Result<Option<Element>> {
        let Some(spec) = spec else {
            return Ok(None);
        };
        let windows = app_element.elements_attribute("AXWindows");
        if windows.is_empty() {
            bail!("this app exposes no window to select; call get_app_state without `window`");
        }
        if let Ok(index) = spec.trim().parse::<usize>() {
            let window = windows.get(index).map(Element::retained).ok_or_else(|| {
                anyhow!(
                    "window {index} does not exist; windows: {}",
                    describe_windows(&windows)
                )
            })?;
            return Ok(Some(window));
        }
        let needle = spec.trim().to_lowercase();
        let window = windows
            .iter()
            .find(|window| {
                window
                    .string_attribute("AXTitle")
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&needle)
            })
            .map(Element::retained)
            .ok_or_else(|| {
                anyhow!(
                    "no window title contains `{spec}`; windows: {}",
                    describe_windows(&windows)
                )
            })?;
        Ok(Some(window))
    }

    fn store(&mut self, app: &apps::App, snapshot: Snapshot) {
        self.cached_app = Some(app.name.clone());
        self.nodes = snapshot
            .nodes
            .into_iter()
            .map(|node| Cached {
                element: node.element,
                center: node.center,
                role: node.text.split(' ').next().unwrap_or_default().to_string(),
                window: node.window,
            })
            .collect();
    }

    fn element(&self, index: i64, app: Option<&str>) -> Result<&Cached> {
        let Some(cached) = &self.cached_app else {
            bail!("no snapshot yet; call get_app_state before using element_index");
        };
        if let Some(app) = app {
            if !app.eq_ignore_ascii_case(cached) {
                bail!(
                    "the cached elements belong to `{cached}`, not `{app}`; call get_app_state for `{app}` first"
                );
            }
        }
        let index = usize::try_from(index).map_err(|_| anyhow!("element_index must be >= 0"))?;
        self.nodes.get(index).ok_or_else(|| {
            anyhow!(
                "element_index {index} is out of range; the last snapshot has {} elements — call get_app_state again",
                self.nodes.len()
            )
        })
    }

    /// An element index is only usable while the element still lives in the
    /// window the snapshot bound it to.
    fn checked_element(&self, index: i64, app: Option<&str>) -> Result<&Cached> {
        let cached = self.element(index, app)?;
        if let Some(bound) = &cached.window {
            let same = cached
                .element
                .window()
                .map(|window| window.same_element(bound))
                .unwrap_or(false);
            if !same {
                bail!(
                    "element {index} ({}) is no longer in the window the snapshot bound it to; call get_app_state again before acting",
                    cached.role
                );
            }
        }
        Ok(cached)
    }

    pub fn list_apps(&self, include_helpers: bool) -> Result<String> {
        self.policy.reload();
        let mut lines = Vec::new();
        for app in apps::running_apps_with(include_helpers) {
            let windows: Vec<&apps::Window> = app
                .windows
                .iter()
                .filter(|window| window.layer == 0)
                .collect();
            if windows.is_empty() {
                continue;
            }
            let mut described: Vec<String> = Vec::new();
            for window in &windows {
                let label = if window.title.trim().is_empty() {
                    format!("{:.0}x{:.0}", window.width, window.height)
                } else {
                    format!(
                        "\"{}\" {:.0}x{:.0}",
                        window.title.trim(),
                        window.width,
                        window.height
                    )
                };
                if !described.contains(&label) {
                    described.push(label);
                }
            }
            let marker = self.access_marker(&app);
            lines.push(format!(
                "{} (pid {}) — {}{marker}",
                app.name,
                app.pid,
                described.join(", ")
            ));
        }
        if lines.is_empty() {
            return Ok("No app with an on-screen window is running.".to_string());
        }
        Ok(lines.join("\n"))
    }

    pub fn get_app_state(&mut self, args: &Value) -> Result<State> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let app_element = Self::app_element(&app)?;
        let window_spec = string_arg(args, "window");
        let scoped = Self::select_window(&app_element, window_spec.as_deref())?;
        let root = scoped.as_ref().unwrap_or(&app_element);

        let mut limits = Limits::default();
        if let Some(value) = usize_arg(args, "max_tree_nodes") {
            limits.max_nodes = value;
        }
        if let Some(value) = usize_arg(args, "max_tree_depth") {
            limits.max_depth = value;
        }
        if let Some(value) = text_limit(args) {
            limits.text_limit = value;
        }
        limits.include_frames = bool_arg(args, "include_frames");

        let snapshot = tree::snapshot(root, &limits, scoped.as_ref());
        let body = tree::render(&snapshot.nodes);
        let truncated = snapshot.truncated;
        let count = snapshot.nodes.len();
        self.store(&app, snapshot);

        let mut header = format!("App: {} (pid {})", app.name, app.pid);
        if let Some(window) = &scoped {
            header.push_str(&format!(
                "\nWindow: \"{}\" (element indexes are bound to this window)",
                window
                    .string_attribute("AXTitle")
                    .unwrap_or_default()
                    .trim()
            ));
        } else {
            let windows: Vec<String> = apps::windows_for(app.pid)
                .into_iter()
                .filter(|window| window.width > 0.0 && window.height > 0.0)
                .map(|window| {
                    format!(
                        "\"{}\" @({:.0},{:.0}) {:.0}x{:.0}",
                        window.title.trim(),
                        window.x,
                        window.y,
                        window.width,
                        window.height
                    )
                })
                .collect();
            if !windows.is_empty() {
                header.push_str(&format!("\nWindows: {}", windows.join(" | ")));
            }
            let ax_windows = app_element.elements_attribute("AXWindows");
            if ax_windows.len() > 1 {
                header.push_str(&format!(
                    "\nPick one with `window`: {}",
                    describe_windows(&ax_windows)
                ));
            }
        }
        header.push_str(&format!("\nElements: {count}"));
        if truncated {
            header.push_str(" (truncated; raise max_tree_nodes or max_tree_depth)");
        }

        let limit = usize_arg(args, "max_output_chars").unwrap_or(DEFAULT_MAX_OUTPUT_CHARS);
        let mut text = cap_output(format!("{header}\n\n{body}"), limit);

        let mut image = None;
        if bool_arg(args, "include_screenshot") {
            let title = scoped
                .as_ref()
                .and_then(|window| window.string_attribute("AXTitle"));
            match self.capture_window(&app, title.as_deref()) {
                Ok(bytes) => {
                    text.push_str("\n\nScreenshot attached for the same app.");
                    image = Some(bytes);
                }
                Err(error) => text.push_str(&format!("\n\nScreenshot unavailable: {error:#}")),
            }
        }
        Ok(State { text, image })
    }

    fn capture_window(&self, app: &apps::App, title: Option<&str>) -> Result<Vec<u8>> {
        let windows = apps::windows_for(app.pid);
        let wanted = title
            .map(|title| title.trim().to_lowercase())
            .filter(|title| !title.is_empty());
        let matched = wanted.as_deref().and_then(|title| {
            windows
                .iter()
                .find(|window| window.title.trim().to_lowercase().contains(title))
        });
        let target = matched.or_else(|| {
            windows.iter().max_by(|left, right| {
                (left.width * left.height)
                    .partial_cmp(&(right.width * right.height))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let number = target
            .map(|window| window.number)
            .ok_or_else(|| anyhow!("{} has no on-screen window to capture", app.name))?;
        screen::capture(Some(number))
    }

    pub fn query(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let app_element = Self::app_element(&app)?;
        let scoped = Self::select_window(&app_element, string_arg(args, "window").as_deref())?;
        let root = scoped.as_ref().unwrap_or(&app_element);

        let mut limits = Limits::default();
        if let Some(value) = usize_arg(args, "max_tree_nodes") {
            limits.max_nodes = value;
        }
        let snapshot = tree::snapshot(root, &limits, scoped.as_ref());
        let needle = string_arg(args, "text").map(|text| text.to_lowercase());
        let role = string_arg(args, "role").map(|role| role.to_uppercase());
        let limit = usize_arg(args, "max_results").unwrap_or(20);

        let matches: Vec<(usize, String)> = snapshot
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                let text_ok = needle
                    .as_deref()
                    .map(|needle| node.text.to_lowercase().contains(needle))
                    .unwrap_or(true);
                let role_ok = role
                    .as_deref()
                    .map(|role| node.text.to_uppercase().contains(role))
                    .unwrap_or(true);
                text_ok && role_ok
            })
            .take(limit)
            .map(|(index, node)| (index, node.text.clone()))
            .collect();

        self.store(&app, snapshot);

        if matches.is_empty() {
            return Ok(format!(
                "No element in {} matched {}{}.",
                app.name,
                needle
                    .map(|needle| format!("text `{needle}` "))
                    .unwrap_or_default(),
                role.map(|role| format!("role `{role}`"))
                    .unwrap_or_default()
            ));
        }
        let mut out = format!("{} matches in {}:", matches.len(), app.name);
        for (index, text) in matches {
            out.push_str(&format!("\n{index:>4} {text}"));
        }
        let limit = usize_arg(args, "max_output_chars").unwrap_or(DEFAULT_MAX_OUTPUT_CHARS);
        Ok(cap_output(out, limit))
    }

    pub fn click(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let index = integer_arg(args, "element_index");
        let button = match string_arg(args, "button").as_deref() {
            Some("right") => input::Button::Right,
            _ => input::Button::Left,
        };

        if let Some(index) = index {
            let cached = self.checked_element(index, Some(&app.name))?;
            let element = &cached.element;
            if matches!(button, input::Button::Left) && element.has_action("AXPress") {
                element.perform("AXPress")?;
                return Ok(format!(
                    "Clicked element {index} ({}) with AXPress; it never needed the pointer.",
                    cached.role
                ));
            }
            let center = cached.center.ok_or_else(|| {
                anyhow!("element {index} has no on-screen frame; pass x and y instead")
            })?;
            let role = cached.role.clone();
            Self::app_element(&app)?.activate_app()?;
            input::click(center, button)?;
            return Ok(format!(
                "Clicked element {index} ({role}) at ({:.0},{:.0}) with a pointer event; the app is now frontmost.",
                center.x, center.y
            ));
        }

        let (x, y) = match (number_arg(args, "x"), number_arg(args, "y")) {
            (Some(x), Some(y)) => (x, y),
            _ => bail!("pass element_index, or x and y together"),
        };
        Self::app_element(&app)?.activate_app()?;
        input::click(CGPoint { x, y }, button)?;
        Ok(format!(
            "Clicked ({x:.0},{y:.0}) with a pointer event; the app is now frontmost."
        ))
    }

    pub fn perform_action(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let index = integer_arg(args, "element_index")
            .ok_or_else(|| anyhow!("`element_index` is required"))?;
        let action = string_arg(args, "action").unwrap_or_else(|| "AXPress".to_string());
        let cached = self.checked_element(index, Some(&app.name))?;
        if !cached.element.has_action(&action) {
            let available = cached.element.actions();
            bail!(
                "element {index} ({}) does not support {action}; it supports: {}",
                cached.role,
                if available.is_empty() {
                    "no actions".to_string()
                } else {
                    available.join(", ")
                }
            );
        }
        cached.element.perform(&action)?;
        Ok(format!(
            "Performed {action} on element {index} ({}).",
            cached.role
        ))
    }

    pub fn set_value(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let index = integer_arg(args, "element_index")
            .ok_or_else(|| anyhow!("`element_index` is required"))?;
        let value = args
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`value` must be a string"))?;
        let cached = self.checked_element(index, Some(&app.name))?;
        if !cached.element.is_settable("AXValue") {
            bail!(
                "element {index} ({}) reports AXValue as not settable; focus it and use press_key or type_text instead",
                cached.role
            );
        }
        cached.element.set_string("AXValue", value)?;
        let read_back = cached.element.string_value().unwrap_or_default();
        if read_back != value {
            bail!(
                "element {index} still reads {read_back:?} after writing {value:?}; the control ignored the write, so nothing was changed"
            );
        }
        Ok(format!(
            "Set element {index} value and read {value:?} back."
        ))
    }

    pub fn select_text(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let needle = string_arg(args, "text").ok_or_else(|| anyhow!("`text` is required"))?;
        let element = match integer_arg(args, "element_index") {
            Some(index) => self
                .checked_element(index, Some(&app.name))?
                .element
                .retained(),
            None => Self::app_element(&app)?.focused_element().ok_or_else(|| {
                anyhow!("no focused text element; pass element_index for the element to select in")
            })?,
        };
        let text = element
            .text()
            .ok_or_else(|| anyhow!("the element exposes no text to select"))?;
        let range = find_range(&text, &needle).ok_or_else(|| {
            let preview: String = text.chars().take(120).collect();
            anyhow!("`{needle}` is not in the element text; it starts with {preview:?}")
        })?;
        element.select_range(range)?;
        let selected = element.selected_text().unwrap_or_default();
        if !selected.is_empty() && selected != needle {
            bail!("the element selected {selected:?} instead of {needle:?}");
        }
        Ok(format!(
            "Selected {} characters at {} ({needle:?}).",
            range.length, range.location
        ))
    }

    pub fn type_text(&mut self, args: &Value) -> Result<String> {
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`text` is required"))?;
        match string_arg(args, "app") {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.guard()?;
                if let Some(index) = integer_arg(args, "element_index") {
                    self.checked_element(index, Some(&app.name))?
                        .element
                        .focus()?;
                }
                Self::app_element(&app)?.activate_app()?;
            }
            None => self.guard()?,
        }
        let typed = input::type_text(text)?;
        Ok(describe_typing(&typed, text))
    }

    pub fn press_key(&mut self, args: &Value) -> Result<String> {
        let key = string_arg(args, "key").ok_or_else(|| anyhow!("`key` is required"))?;
        let modifiers = string_arg(args, "modifiers").unwrap_or_default();
        input::check(&key, &modifiers)?;
        match string_arg(args, "app") {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.guard()?;
                Self::app_element(&app)?.activate_app()?;
            }
            None => self.guard()?,
        }
        let pressed = input::press(&key, &modifiers)?;
        let spelling = match key.rsplit_once('+') {
            Some((_, rest)) if !rest.is_empty() => rest.trim(),
            _ => key.trim(),
        };
        Ok(if pressed == spelling {
            format!("Pressed `{}`.", key.trim())
        } else {
            format!("Pressed `{pressed}` (for `{}`).", key.trim())
        })
    }

    pub fn scroll(&mut self, args: &Value) -> Result<String> {
        let direction = string_arg(args, "direction").unwrap_or_else(|| "down".to_string());
        let lines = integer_arg(args, "lines").unwrap_or(3).abs().max(1) as i32;
        let signed = if direction.eq_ignore_ascii_case("up") {
            lines
        } else if direction.eq_ignore_ascii_case("down") {
            -lines
        } else {
            bail!("`direction` must be `up` or `down`")
        };

        let point = if let Some(index) = integer_arg(args, "element_index") {
            self.checked_element(index, string_arg(args, "app").as_deref())?
                .center
        } else {
            match (number_arg(args, "x"), number_arg(args, "y")) {
                (Some(x), Some(y)) => Some(CGPoint { x, y }),
                _ => None,
            }
        };

        match string_arg(args, "app") {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.guard()?;
                Self::app_element(&app)?.activate_app()?;
            }
            None => self.guard()?,
        }
        input::scroll(point, signed)?;
        Ok(format!("Scrolled {direction} {lines} lines."))
    }

    pub fn drag(&mut self, args: &Value) -> Result<String> {
        let start = match integer_arg(args, "element_index") {
            Some(index) => self
                .checked_element(index, string_arg(args, "app").as_deref())?
                .center
                .ok_or_else(|| anyhow!("element {index} has no on-screen frame"))?,
            None => CGPoint {
                x: number_arg(args, "from_x")
                    .ok_or_else(|| anyhow!("pass element_index or from_x/from_y"))?,
                y: number_arg(args, "from_y")
                    .ok_or_else(|| anyhow!("pass element_index or from_x/from_y"))?,
            },
        };
        let end = CGPoint {
            x: number_arg(args, "to_x").ok_or_else(|| anyhow!("`to_x` is required"))?,
            y: number_arg(args, "to_y").ok_or_else(|| anyhow!("`to_y` is required"))?,
        };
        match string_arg(args, "app") {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.guard()?;
                Self::app_element(&app)?.activate_app()?;
            }
            None => self.guard()?,
        }
        input::drag(start, end)?;
        Ok(format!(
            "Dragged from ({:.0},{:.0}) to ({:.0},{:.0}).",
            start.x, start.y, end.x, end.y
        ))
    }

    pub fn screenshot(&self, args: &Value) -> Result<Vec<u8>> {
        match string_arg(args, "app") {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.capture_window(&app, string_arg(args, "window").as_deref())
            }
            None => screen::capture(None),
        }
    }

    pub fn guard(&self) -> Result<()> {
        if !ax::trusted() {
            bail!(
                "Accessibility permission is missing. {}",
                crate::permissions::guidance()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_counted_in_utf16_units() {
        let text = "héllo wörld";
        let range = find_range(text, "wörld").expect("the needle is present");
        assert_eq!(range.location, 6);
        assert_eq!(range.length, 5);
    }

    #[test]
    fn an_emoji_needle_uses_its_utf16_length() {
        let range = find_range("a🚀b", "🚀").expect("the emoji is present");
        assert_eq!(range.location, 1);
        assert_eq!(range.length, 2);
    }

    #[test]
    fn matching_falls_back_to_case_insensitive() {
        let range = find_range("Hello World", "world").expect("a case-insensitive match");
        assert_eq!(range.location, 6);
    }

    #[test]
    fn a_missing_or_empty_needle_has_no_range() {
        assert!(find_range("hello", "bye").is_none());
        assert!(find_range("hello", "").is_none());
    }

    #[test]
    fn a_reply_counts_the_line_breaks_that_were_sent() {
        let typed = input::Typed {
            characters: 7,
            line_breaks: 0,
            takeover: None,
        };
        assert_eq!(describe_typing(&typed, "a\nb"), "Typed 7 characters.");
        let typed = input::Typed {
            characters: 5,
            line_breaks: 1,
            takeover: None,
        };
        assert!(describe_typing(&typed, "x\ny").contains("one line break with Return"));
        let typed = input::Typed {
            characters: 5,
            line_breaks: 2,
            takeover: Some("a keystroke"),
        };
        let reply = describe_typing(&typed, "x\ny\nz");
        assert!(reply.contains("2 line breaks"), "{reply}");
        assert!(reply.contains("user takeover"), "{reply}");
    }

    #[test]
    fn a_literal_escape_sequence_in_the_text_is_named() {
        assert!(literal_escape_note("hello").is_none());
        let note = literal_escape_note(r"first\nsecond").unwrap();
        assert!(note.contains(r"\n"), "{note}");
        assert!(note.contains("Return"), "{note}");
        assert!(literal_escape_note(r"tab\there").is_some());
    }

    #[test]
    fn output_is_capped_with_a_visible_note() {
        let text = "x".repeat(50);
        let capped = cap_output(text.clone(), 10);
        assert!(capped.starts_with(&"x".repeat(10)));
        assert!(capped.contains("truncated at 10 of 50 characters"));
        assert_eq!(cap_output(text.clone(), 100), text);
    }

    #[test]
    fn arguments_are_read_leniently() {
        let args = serde_json::json!({ "x": "12.5", "y": 3, "flag": true });
        assert_eq!(number_arg(&args, "x"), Some(12.5));
        assert_eq!(number_arg(&args, "y"), Some(3.0));
        assert!(bool_arg(&args, "flag"));
        assert_eq!(usize_arg(&args, "y"), Some(3));
    }
}
