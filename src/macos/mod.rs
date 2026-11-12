//! macOS oxide-computer-use backend: accessibility snapshots plus synthetic input.

pub mod apps;
pub mod ax;
pub mod cf;
pub mod input;
pub mod instructions;
pub mod pasteboard;
pub mod screen;
pub mod takeover;
pub mod tree;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::policy::{self, Access, Policy};
use crate::runtime::State;

use ax::{CFRange, CGPoint, Element};
use tree::{Limits, Snapshot};

const DEFAULT_MAX_OUTPUT_CHARS: usize = 20_000;
const SETTLE_AFTER_ACTION: Duration = Duration::from_secs(1);
const MAX_SETTLE_WAIT: Duration = Duration::from_secs(5);
const SETTLE_POLL: Duration = Duration::from_millis(250);

struct Cached {
    element: Element,
    center: Option<CGPoint>,
    role: String,
    text: String,
    window: Option<Element>,
}

#[derive(Clone)]
struct CoordinateWindow {
    app: String,
    window: apps::Window,
    scale_x: f64,
    scale_y: f64,
}

pub struct Runtime {
    nodes: Vec<Cached>,
    cached_app: Option<String>,
    cached_window: Option<String>,
    coordinate_window: Option<CoordinateWindow>,
    policy: Policy,
    last_action: Option<Instant>,
    delivered_instructions: HashSet<String>,
    /// The `.app` that launched this server, resolved once: it is the app a
    /// self-automation rule may refuse to drive.
    host: Option<String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime {
            nodes: Vec::new(),
            cached_app: None,
            cached_window: None,
            coordinate_window: None,
            policy: Policy::load(),
            last_action: None,
            delivered_instructions: HashSet::new(),
            host: crate::permissions::launching_app(),
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

/// Whether the app being resolved is the bundle that hosts this server. An
/// unreadable identity is not the host: refusing on a missing identity would
/// block every unbundled app.
fn drives_itself(host: Option<&str>, running: Option<&str>) -> bool {
    match (host, running) {
        (Some(host), Some(running)) => {
            !running.is_empty() && running.trim_end_matches('/') == host.trim_end_matches('/')
        }
        _ => false,
    }
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
    optional_app(args)
        .ok_or_else(|| anyhow!("`app` is required; call list_apps to see the running apps"))
}

fn optional_app(args: &Value) -> Option<String> {
    string_arg(args, "app").map(|app| app.trim().trim_start_matches('@').trim().to_string())
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

fn duration_arg(args: &Value, key: &str) -> Result<Option<Duration>> {
    let Some(seconds) = number_arg(args, key) else {
        return Ok(None);
    };
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("`{key}` must be a finite number of seconds >= 0");
    }
    Ok(Some(Duration::from_secs_f64(seconds)))
}

fn mouse_button(args: &Value) -> Result<input::Button> {
    let value = args.get("mouse_button").or_else(|| args.get("button"));
    let value = match value {
        Some(Value::String(text)) => text.trim().to_lowercase(),
        Some(Value::Number(number)) => number.to_string(),
        Some(other) => bail!("`mouse_button` must be a string or number; got {other}"),
        None => "left".to_string(),
    };
    match value.as_str() {
        "left" | "l" | "0" => Ok(input::Button::Left),
        "right" | "r" | "1" => Ok(input::Button::Right),
        "middle" | "m" | "2" => Ok(input::Button::Middle),
        other => bail!("`mouse_button` must be left, right or middle (or l/r/m); got `{other}`"),
    }
}

fn button_name(button: &input::Button) -> &'static str {
    match button {
        input::Button::Left => "left",
        input::Button::Right => "right",
        input::Button::Middle => "middle",
    }
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
#[cfg(test)]
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

fn find_range_with_context(
    haystack: &str,
    needle: &str,
    prefix: Option<&str>,
    suffix: Option<&str>,
) -> Option<CFRange> {
    fn locate(
        units: &[u16],
        target: &[u16],
        prefix: Option<&[u16]>,
        suffix: Option<&[u16]>,
    ) -> Option<CFRange> {
        if target.is_empty() || target.len() > units.len() {
            return None;
        }
        for start in 0..=units.len() - target.len() {
            if units[start..start + target.len()] != target[..] {
                continue;
            }
            let prefix_ok = prefix.is_none_or(|prefix| {
                prefix.len() <= start && units[start - prefix.len()..start] == prefix[..]
            });
            let end = start + target.len();
            let suffix_ok = suffix.is_none_or(|suffix| {
                suffix.len() <= units.len() - end && units[end..end + suffix.len()] == suffix[..]
            });
            if prefix_ok && suffix_ok {
                return Some(CFRange {
                    location: start as isize,
                    length: target.len() as isize,
                });
            }
        }
        None
    }

    let units: Vec<u16> = haystack.encode_utf16().collect();
    let target: Vec<u16> = needle.encode_utf16().collect();
    let prefix_units: Option<Vec<u16>> = prefix.map(|text| text.encode_utf16().collect());
    let suffix_units: Option<Vec<u16>> = suffix.map(|text| text.encode_utf16().collect());
    locate(
        &units,
        &target,
        prefix_units.as_deref(),
        suffix_units.as_deref(),
    )
    .or_else(|| {
        let lower_units: Vec<u16> = haystack.to_lowercase().encode_utf16().collect();
        let lower_target: Vec<u16> = needle.to_lowercase().encode_utf16().collect();
        let lower_prefix: Option<Vec<u16>> =
            prefix.map(|text| text.to_lowercase().encode_utf16().collect());
        let lower_suffix: Option<Vec<u16>> =
            suffix.map(|text| text.to_lowercase().encode_utf16().collect());
        locate(
            &lower_units,
            &lower_target,
            lower_prefix.as_deref(),
            lower_suffix.as_deref(),
        )
    })
}

fn png_size(bytes: &[u8]) -> Option<(f64, f64)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width as f64, height as f64))
}

fn render_diff(previous: &[Cached], current: &[tree::Node]) -> String {
    if previous.len() != current.len() {
        return format!(
            "The accessibility structure changed ({} to {} elements). Current indexes:\n{}",
            previous.len(),
            current.len(),
            tree::render(current)
        );
    }
    let mut lines = Vec::new();
    for index in 0..current.len() {
        if previous[index].text == current[index].text {
            continue;
        }
        lines.push(format!(
            "{index:>4} ~ {}{} (was: {})",
            "  ".repeat(current[index].depth),
            current[index].text,
            previous[index].text
        ));
    }
    if lines.is_empty() {
        "No accessibility changes since the previous state.".to_string()
    } else {
        format!("Changed accessibility elements:\n{}", lines.join("\n"))
    }
}

fn settle_wait(last_action: Option<Instant>, now: Instant) -> Duration {
    last_action
        .map(|action| SETTLE_AFTER_ACTION.saturating_sub(now.saturating_duration_since(action)))
        .unwrap_or_default()
}

fn line_looks_loading(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("axprogressindicator")
        || lower.contains("· busy")
        || lower.contains("value=\"loading")
        || lower.contains("value=\"正在加载")
        || lower.contains("value=\"载入中")
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

/// One snapshot, or an error that leaves the previous one in place. An app that
/// stops answering must not render as an empty window, and its old indexes must
/// keep pointing at the app they were read from.
fn snapshot(
    app: &apps::App,
    root: &Element,
    limits: &Limits,
    window: Option<&Element>,
) -> Result<Snapshot> {
    tree::snapshot(root, limits, window).map_err(|error| {
        anyhow!(
            "could not read {}'s accessibility tree: {error:#}. The read was abandoned, so the newest snapshot is still the previous one and its element indexes still address the app they came from. Retry once the app answers",
            app.name
        )
    })
}

impl Runtime {
    pub fn new() -> Self {
        Runtime::default()
    }

    fn resolved(&self, name: &str) -> Result<apps::App> {
        let app = match apps::resolve(name) {
            Some(app) => app,
            None => {
                let Some(installed) = apps::resolve_installed(name) else {
                    let known = apps::app_names();
                    let hint = if known.is_empty() {
                        "no launchable app matches, and no app with a window is running".to_string()
                    } else {
                        format!("running apps: {}", known.join(", "))
                    };
                    bail!("no app matches `{name}`; {hint}");
                };
                self.check_installed_rules(&installed)?;
                apps::launch_installed(&installed)?
            }
        };
        let pid = app.pid;
        self.policy.authorize(&app.name, || apps::identity(pid))?;
        self.check_self_automation(&app)?;
        self.check_terminal(&app)?;
        Ok(app)
    }

    fn check_installed_rules(&self, app: &apps::InstalledApp) -> Result<()> {
        if let Access::Blocked(message) = self.policy.check(&app.name, app.identity.as_ref())? {
            bail!("{message}");
        }
        if self.policy.terminals() == policy::Fallback::Deny
            && self.policy.is_terminal(&app.name, app.identity.as_ref())
        {
            bail!(
                "`{}` is a terminal{}, and the policy refuses terminal automation: typing into a shell can start another agent. Allow it with `oxide-computer-use apps terminals allow` if that is intended",
                app.name,
                app.identity
                    .as_ref()
                    .map(|identity| format!(" ({})", identity.bundle_id))
                    .unwrap_or_default()
            );
        }
        if self.policy.self_automation() == policy::Fallback::Deny {
            if let (Some(host), Some(identity)) = (self.host.as_deref(), app.identity.as_ref()) {
                if drives_itself(Some(host), Some(&identity.path)) {
                    bail!(
                        "`{}` is the app hosting this MCP server ({host}), and the policy refuses self-automation. Name another app, or allow it with `oxide-computer-use apps self-automation allow`",
                        app.name
                    );
                }
            }
        }
        Ok(())
    }

    /// A terminal is a way to start another agent, so it is refused by default.
    /// The identity decides when it can be read; the name list is the fallback.
    fn check_terminal(&self, app: &apps::App) -> Result<()> {
        if self.policy.terminals() == policy::Fallback::Allow {
            return Ok(());
        }
        let identity = apps::identity(app.pid);
        if !self.policy.is_terminal(&app.name, identity.as_ref()) {
            return Ok(());
        }
        bail!(
            "`{}` is a terminal{}, and the policy refuses terminal automation: typing into a shell can start another agent. Allow it with `oxide-computer-use apps terminals allow` if that is intended",
            app.name,
            match &identity {
                Some(identity) => format!(" ({})", identity.bundle_id),
                None => String::new(),
            }
        )
    }

    /// Refuse to drive the window this server is hosted in: an agent clicking its
    /// own transcript is a loop, and the policy can say so. Only the hosting
    /// bundle matches, compared by path.
    fn check_self_automation(&self, app: &apps::App) -> Result<()> {
        if self.policy.self_automation() == policy::Fallback::Allow {
            return Ok(());
        }
        let Some(host) = self.host.as_deref() else {
            return Ok(());
        };
        let running = apps::identity(app.pid).map(|identity| identity.path);
        if !drives_itself(Some(host), running.as_deref()) {
            return Ok(());
        }
        bail!(
            "`{}` is the app hosting this MCP server ({host}), and the policy refuses self-automation: clicking the window you are shown in loops. Name another app, or allow it with `oxide-computer-use apps self-automation allow`",
            app.name
        )
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
        let windows = app_element.elements_checked("AXWindows").map_err(|error| {
            anyhow!(
                "could not read this app's window list: {error:#}; retry once the app answers, or call get_app_state without `window`"
            )
        })?;
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

    fn store(&mut self, app: &apps::App, window: Option<String>, snapshot: Snapshot) {
        self.cached_app = Some(app.name.clone());
        self.cached_window = window;
        self.nodes = snapshot
            .nodes
            .into_iter()
            .map(|node| Cached {
                element: node.element,
                center: node.center,
                role: node.text.split(' ').next().unwrap_or_default().to_string(),
                text: node.text,
                window: node.window,
            })
            .collect();
    }

    fn mark_action(&mut self) {
        self.last_action = Some(Instant::now());
    }

    fn snapshot_after_action(
        &mut self,
        app: &apps::App,
        root: &Element,
        limits: &Limits,
        window: Option<&Element>,
    ) -> Result<Snapshot> {
        let action = self.last_action;
        let wait = settle_wait(action, Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let deadline = action
            .map(|started| started + MAX_SETTLE_WAIT)
            .unwrap_or_else(|| Instant::now() + SETTLE_AFTER_ACTION);
        let mut current = snapshot(app, root, limits, window)?;
        while action.is_some()
            && current
                .nodes
                .iter()
                .any(|node| line_looks_loading(&node.text))
            && Instant::now() < deadline
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(remaining.min(SETTLE_POLL));
            current = snapshot(app, root, limits, window)?;
        }
        self.last_action = None;
        Ok(current)
    }

    fn first_instructions(&mut self, app: &apps::App) -> Option<&'static str> {
        let bundle_id = apps::identity(app.pid).map(|identity| identity.bundle_id);
        let key = bundle_id.clone().unwrap_or_else(|| app.name.clone());
        let instructions = instructions::for_app(bundle_id.as_deref(), &app.name)?;
        self.delivered_instructions
            .insert(key)
            .then_some(instructions)
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
        let mut entries = Vec::new();
        for app in apps::installed_apps() {
            let marker = self
                .policy
                .access(&app.name, app.identity.as_ref())
                .marker()
                .trim();
            entries.push(serde_json::json!({
                "id": app.id,
                "displayName": app.name,
                "isRunning": app.pid.is_some(),
                "path": app.path,
                "policy": if marker.is_empty() { "allow" } else { marker },
            }));
        }
        if include_helpers {
            for app in apps::running_apps_with(true) {
                if entries.iter().any(|entry| {
                    entry["displayName"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&app.name))
                }) {
                    continue;
                }
                entries.push(serde_json::json!({
                    "id": app.pid.to_string(),
                    "displayName": app.name,
                    "isRunning": true,
                    "path": Value::Null,
                    "policy": self.access_marker(&app).trim(),
                }));
            }
        }
        entries.sort_by(|left, right| {
            left["displayName"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .cmp(
                    &right["displayName"]
                        .as_str()
                        .unwrap_or_default()
                        .to_lowercase(),
                )
        });
        Ok(serde_json::to_string_pretty(&entries)?)
    }

    pub fn app_policy(&mut self, args: &Value) -> Result<String> {
        let action = string_arg(args, "action").unwrap_or_else(|| "list".to_string());
        let requested_app =
            || optional_app(args).ok_or_else(|| anyhow!("`app` is required for `{action}`"));
        let identity_for = |app: &str| {
            apps::resolve(app)
                .and_then(|running| apps::identity(running.pid))
                .or_else(|| apps::resolve_installed(app)?.identity)
        };
        match action.as_str() {
            "list" => {}
            "allow" => {
                let app = requested_app()?;
                let identity = identity_for(&app);
                self.policy.allow(&app, identity)?;
            }
            "grant_once" | "grant-once" => {
                let app = requested_app()?;
                let identity = identity_for(&app);
                self.policy.grant_once(&app, identity)?;
            }
            "deny" => self.policy.deny(&requested_app()?)?,
            "revoke" => self.policy.revoke(&requested_app()?)?,
            "set_default" | "set-default" => {
                let mode = string_arg(args, "mode")
                    .and_then(|mode| policy::Fallback::parse(&mode))
                    .ok_or_else(|| anyhow!("`mode` must be `allow` or `deny`"))?;
                self.policy.set_fallback(mode)?;
            }
            "set_self_automation" | "set-self-automation" => {
                let mode = string_arg(args, "mode")
                    .and_then(|mode| policy::Fallback::parse(&mode))
                    .ok_or_else(|| anyhow!("`mode` must be `allow` or `deny`"))?;
                self.policy.set_self_automation(mode)?;
            }
            "set_terminals" | "set-terminals" => {
                let mode = string_arg(args, "mode")
                    .and_then(|mode| policy::Fallback::parse(&mode))
                    .ok_or_else(|| anyhow!("`mode` must be `allow` or `deny`"))?;
                self.policy.set_terminals(mode)?;
            }
            "reset" => self.policy.clear()?,
            other => bail!(
                "unknown app policy action `{other}`; use list, allow, grant_once, deny, revoke, set_default, set_self_automation, set_terminals or reset"
            ),
        }
        self.policy.reload();

        let apps: Vec<Value> = apps::installed_apps()
            .into_iter()
            .map(|app| {
                let access = self.policy.access(&app.name, app.identity.as_ref());
                serde_json::json!({
                    "id": app.id,
                    "displayName": app.name,
                    "path": app.path,
                    "isRunning": app.pid.is_some(),
                    "policy": access.marker().trim(),
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "default": match self.policy.fallback() {
                policy::Fallback::Allow => "allow",
                policy::Fallback::Deny => "deny",
            },
            "allowed": self.policy.allowed(),
            "denied": self.policy.denied(),
            "identities": self.policy.identities(),
            "oneTime": self.policy.one_time(),
            "selfAutomation": match self.policy.self_automation() {
                policy::Fallback::Allow => "allow",
                policy::Fallback::Deny => "deny",
            },
            "terminals": match self.policy.terminals() {
                policy::Fallback::Allow => "allow",
                policy::Fallback::Deny => "deny",
            },
            "apps": apps,
        }))?)
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

        let snapshot = self.snapshot_after_action(&app, root, &limits, scoped.as_ref())?;
        let window_label = scoped
            .as_ref()
            .and_then(|window| window.string_attribute("AXTitle"))
            .map(|title| title.trim().to_string());
        let disable_diff = bool_arg(args, "disableDiff") || bool_arg(args, "disable_diff");
        let body = if !disable_diff
            && self.cached_app.as_deref() == Some(&app.name)
            && self.cached_window == window_label
            && !self.nodes.is_empty()
        {
            render_diff(&self.nodes, &snapshot.nodes)
        } else {
            tree::render(&snapshot.nodes)
        };
        let truncated = snapshot.truncated;
        let count = snapshot.nodes.len();
        self.store(&app, window_label, snapshot);
        let guidance = self
            .first_instructions(&app)
            .map(|instructions| {
                format!(
                    "<app_specific_instructions>\n{instructions}\n</app_specific_instructions>\n\n"
                )
            })
            .unwrap_or_default();

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
        let mut text = cap_output(format!("{guidance}{header}\n\n{body}"), limit);

        let title = scoped
            .as_ref()
            .and_then(|window| window.string_attribute("AXTitle"));
        let include_screenshot = args
            .get("include_screenshot")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut image = None;
        match self.target_window(&app, title.as_deref()) {
            Ok(window) => {
                let mut scale_x = 1.0;
                let mut scale_y = 1.0;
                if include_screenshot {
                    match screen::capture(Some(window.number)) {
                        Ok(bytes) => {
                            if let Some((pixel_width, pixel_height)) = png_size(&bytes) {
                                if window.width > 0.0 && window.height > 0.0 {
                                    scale_x = pixel_width / window.width;
                                    scale_y = pixel_height / window.height;
                                }
                            }
                            image = Some(bytes);
                        }
                        Err(error) => {
                            text.push_str(&format!("\n\nScreenshot unavailable: {error:#}"))
                        }
                    }
                }
                self.coordinate_window = Some(CoordinateWindow {
                    app: app.name.clone(),
                    window,
                    scale_x,
                    scale_y,
                });
                if image.is_some() {
                    text.push_str("\n\nScreenshot attached for the same app.");
                }
            }
            Err(error) if include_screenshot => {
                text.push_str(&format!("\n\nScreenshot unavailable: {error:#}"))
            }
            Err(_) => self.coordinate_window = None,
        }
        Ok(State { text, image })
    }

    fn target_window(&self, app: &apps::App, title: Option<&str>) -> Result<apps::Window> {
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
        target
            .cloned()
            .ok_or_else(|| anyhow!("{} has no on-screen window to capture", app.name))
    }

    fn capture_window(&self, app: &apps::App, title: Option<&str>) -> Result<Vec<u8>> {
        let window = self.target_window(app, title)?;
        screen::capture(Some(window.number))
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
        let snapshot = snapshot(&app, root, &limits, scoped.as_ref())?;
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

        let window_label = scoped
            .as_ref()
            .and_then(|window| window.string_attribute("AXTitle"))
            .map(|title| title.trim().to_string());
        self.store(&app, window_label, snapshot);

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

    fn point_in_window(&self, app: &apps::App, x: f64, y: f64) -> Result<CGPoint> {
        let current = self
            .coordinate_window
            .as_ref()
            .filter(|current| current.app.eq_ignore_ascii_case(&app.name))
            .cloned();
        let current = match current {
            Some(current) => current,
            None => CoordinateWindow {
                app: app.name.clone(),
                window: self.target_window(app, None)?,
                scale_x: 1.0,
                scale_y: 1.0,
            },
        };
        Ok(CGPoint {
            x: current.window.x + x / current.scale_x.max(0.01),
            y: current.window.y + y / current.scale_y.max(0.01),
        })
    }

    fn window_size(&self, app: &apps::App) -> Result<(f64, f64)> {
        let window = self
            .coordinate_window
            .as_ref()
            .filter(|current| current.app.eq_ignore_ascii_case(&app.name))
            .map(|current| current.window.clone())
            .unwrap_or(self.target_window(app, None)?);
        Ok((window.width, window.height))
    }

    pub fn click(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let index = integer_arg(args, "element_index");
        let button = mouse_button(args)?;
        let click_count = usize_arg(args, "click_count").unwrap_or(1);
        if click_count == 0 {
            bail!("`click_count` must be at least 1");
        }
        let hold = duration_arg(args, "duration")?.unwrap_or(Duration::from_millis(35));
        let key_flags = match string_arg(args, "key") {
            Some(key) => input::click_flags(&key)?,
            None => 0,
        };
        let plain_click = index.is_some()
            && matches!(button, input::Button::Left)
            && click_count == 1
            && key_flags == 0
            && args.get("duration").is_none();

        if let Some(index) = index {
            let cached = self.checked_element(index, Some(&app.name))?;
            let element = &cached.element;
            if plain_click && element.has_action("AXPress") {
                let role = cached.role.clone();
                element.perform("AXPress")?;
                self.mark_action();
                return Ok(format!(
                    "Clicked element {index} ({}) with AXPress; it never needed the pointer.",
                    role
                ));
            }
            let center = cached.center.ok_or_else(|| {
                anyhow!("element {index} has no on-screen frame; pass x and y instead")
            })?;
            let role = cached.role.clone();
            Self::app_element(&app)?.activate_app()?;
            input::click_with(center, button, click_count, hold, key_flags)?;
            self.mark_action();
            return Ok(format!(
                "Clicked element {index} ({role}) at ({:.0},{:.0}) with {} button click{} and a pointer event; the app is now frontmost.",
                center.x,
                center.y,
                button_name(&button),
                if click_count == 1 { "" } else { "s" },
            ));
        }

        let (x, y) = match (number_arg(args, "x"), number_arg(args, "y")) {
            (Some(x), Some(y)) => (x, y),
            _ => bail!("pass element_index, or x and y together"),
        };
        let point = self.point_in_window(&app, x, y)?;
        Self::app_element(&app)?.activate_app()?;
        input::click_with(point, button, click_count, hold, key_flags)?;
        self.mark_action();
        Ok(format!(
            "Clicked app-window coordinate ({x:.0},{y:.0}) with {} button click{} and a pointer event; the app is now frontmost.",
            button_name(&button),
            if click_count == 1 { "" } else { "s" },
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
        let role = cached.role.clone();
        self.mark_action();
        Ok(format!("Performed {action} on element {index} ({}).", role))
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
        self.mark_action();
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
        let prefix = string_arg(args, "prefix");
        let suffix = string_arg(args, "suffix");
        let found = find_range_with_context(&text, &needle, prefix.as_deref(), suffix.as_deref())
            .ok_or_else(|| {
            let preview: String = text.chars().take(120).collect();
            anyhow!(
                "`{needle}`{} is not in the element text; it starts with {preview:?}",
                if prefix.is_some() || suffix.is_some() {
                    " with the requested prefix/suffix"
                } else {
                    ""
                }
            )
        })?;
        let selection_type = string_arg(args, "selection_type")
            .or_else(|| string_arg(args, "selection"))
            .unwrap_or_else(|| "text".to_string());
        let range = match selection_type.trim().to_lowercase().as_str() {
            "text" => found,
            "cursor_before" => CFRange {
                location: found.location,
                length: 0,
            },
            "cursor_after" => CFRange {
                location: found.location + found.length,
                length: 0,
            },
            other => {
                bail!("`selection_type` must be text, cursor_before or cursor_after; got `{other}`")
            }
        };
        element.select_range(range)?;
        if range.length == 0 {
            let selected = element.selected_range();
            if selected != Some(range) {
                bail!("the element moved the cursor to {selected:?} instead of {range:?}");
            }
        } else {
            let selected = element.selected_text().unwrap_or_default();
            if !selected.is_empty() && selected != needle {
                bail!("the element selected {selected:?} instead of {needle:?}");
            }
        }
        self.mark_action();
        Ok(format!(
            "Selected {} characters at {} ({needle:?}) with {selection_type}.",
            found.length, found.location
        ))
    }

    pub fn type_text(&mut self, args: &Value) -> Result<String> {
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`text` is required"))?;
        let background = bool_arg(args, "background");
        let mut target: Option<apps::App> = None;
        match optional_app(args) {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.guard()?;
                if let Some(index) = integer_arg(args, "element_index") {
                    self.checked_element(index, Some(&app.name))?
                        .element
                        .focus()?;
                }
                if background {
                    target = Some(app);
                } else {
                    Self::app_element(&app)?.activate_app()?;
                }
            }
            None => {
                if background {
                    bail!("`background` needs `app`: the input is addressed to that process");
                }
                self.guard()?;
            }
        }
        let typed = match &target {
            Some(app) => input::type_text_in(app.pid, text)?,
            None => input::type_text(text)?,
        };
        self.mark_action();
        let mut reply = describe_typing(&typed, text);
        if let Some(app) = &target {
            reply.push_str(&format!(
                "\nDelivered to {} (pid {}) without activating it; an app that keeps its own key focus can ignore input sent this way, so verify with get_app_state.",
                app.name, app.pid
            ));
        }
        Ok(reply)
    }

    pub fn press_key(&mut self, args: &Value) -> Result<String> {
        let key = string_arg(args, "key").ok_or_else(|| anyhow!("`key` is required"))?;
        let modifiers = string_arg(args, "modifiers").unwrap_or_default();
        input::check(&key, &modifiers)?;
        let duration = duration_arg(args, "duration")?.unwrap_or(Duration::ZERO);
        let background = bool_arg(args, "background");
        let mut target: Option<apps::App> = None;
        match optional_app(args) {
            Some(name) => {
                let app = self.resolved(&name)?;
                self.guard()?;
                if background {
                    target = Some(app);
                } else {
                    Self::app_element(&app)?.activate_app()?;
                }
            }
            None => {
                if background {
                    bail!("`background` needs `app`: the input is addressed to that process");
                }
                self.guard()?;
            }
        }
        let pressed = match &target {
            Some(app) => input::press_in_for(app.pid, &key, &modifiers, duration)?,
            None => input::press_for(&key, &modifiers, duration)?,
        };
        self.mark_action();
        let spelling = match key.rsplit_once('+') {
            Some((_, rest)) if !rest.is_empty() => rest.trim(),
            _ => key.trim(),
        };
        let mut reply = if pressed == spelling {
            format!("Pressed `{}`.", key.trim())
        } else {
            format!("Pressed `{pressed}` (for `{}`).", key.trim())
        };
        if let Some(app) = &target {
            reply.push_str(&format!(
                "\nDelivered to {} (pid {}) without activating it; an app that keeps its own key focus can ignore input sent this way, so verify with get_app_state.",
                app.name, app.pid
            ));
        }
        Ok(reply)
    }

    pub fn scroll(&mut self, args: &Value) -> Result<String> {
        let direction = string_arg(args, "direction").unwrap_or_else(|| "down".to_string());
        let pages = number_arg(args, "pages")
            .or_else(|| number_arg(args, "lines").map(|lines| lines / 10.0))
            .unwrap_or(1.0);
        if !pages.is_finite() || pages <= 0.0 {
            bail!("`pages` must be a finite number > 0");
        }
        let target_app = match optional_app(args) {
            Some(name) => Some(self.resolved(&name)?),
            None => None,
        };
        self.guard()?;
        let (view_width, view_height) = match &target_app {
            Some(app) => self.window_size(app)?,
            None => (1000.0, 800.0),
        };
        let (horizontal, vertical) = match direction.trim().to_lowercase().as_str() {
            "up" | "u" => (0, (pages * view_height).round() as i32),
            "down" | "d" => (0, -(pages * view_height).round() as i32),
            "left" | "l" => ((pages * view_width).round() as i32, 0),
            "right" | "r" => (-(pages * view_width).round() as i32, 0),
            _ => bail!("`direction` must be up, down, left or right"),
        };

        let point = if let Some(index) = integer_arg(args, "element_index") {
            self.checked_element(index, target_app.as_ref().map(|app| app.name.as_str()))?
                .center
        } else {
            match (number_arg(args, "x"), number_arg(args, "y")) {
                (Some(x), Some(y)) => Some(match &target_app {
                    Some(app) => self.point_in_window(app, x, y)?,
                    None => CGPoint { x, y },
                }),
                _ => None,
            }
        };

        if let Some(app) = &target_app {
            Self::app_element(app)?.activate_app()?;
        }
        input::scroll(point, horizontal, vertical)?;
        self.mark_action();
        Ok(format!("Scrolled {direction} {pages} page(s)."))
    }

    pub fn drag(&mut self, args: &Value) -> Result<String> {
        let target_app = match optional_app(args) {
            Some(name) => Some(self.resolved(&name)?),
            None => None,
        };
        self.guard()?;
        let start = match integer_arg(args, "element_index") {
            Some(index) => self
                .checked_element(index, target_app.as_ref().map(|app| app.name.as_str()))?
                .center
                .ok_or_else(|| anyhow!("element {index} has no on-screen frame"))?,
            None => {
                let x = number_arg(args, "from_x")
                    .ok_or_else(|| anyhow!("pass element_index or from_x/from_y"))?;
                let y = number_arg(args, "from_y")
                    .ok_or_else(|| anyhow!("pass element_index or from_x/from_y"))?;
                match &target_app {
                    Some(app) => self.point_in_window(app, x, y)?,
                    None => CGPoint { x, y },
                }
            }
        };
        let end = match &target_app {
            Some(app) => self.point_in_window(
                app,
                number_arg(args, "to_x").ok_or_else(|| anyhow!("`to_x` is required"))?,
                number_arg(args, "to_y").ok_or_else(|| anyhow!("`to_y` is required"))?,
            )?,
            None => CGPoint {
                x: number_arg(args, "to_x").ok_or_else(|| anyhow!("`to_x` is required"))?,
                y: number_arg(args, "to_y").ok_or_else(|| anyhow!("`to_y` is required"))?,
            },
        };
        if let Some(app) = &target_app {
            Self::app_element(app)?.activate_app()?;
        }
        input::drag(start, end)?;
        self.mark_action();
        Ok(format!(
            "Dragged from ({:.0},{:.0}) to ({:.0},{:.0}).",
            start.x, start.y, end.x, end.y
        ))
    }

    pub fn paste(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolved(&name)?;
        self.guard()?;
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`text` is required"))?;
        let format = string_arg(args, "format").unwrap_or_else(|| "text".to_string());
        if !matches!(format.as_str(), "text" | "md" | "html") {
            bail!("`format` must be text, md or html");
        }
        Self::app_element(&app)?.activate_app()?;
        pasteboard::paste_with_restore(text, &format, || {
            input::press("v", "cmd")?;
            std::thread::sleep(Duration::from_millis(200));
            Ok(())
        })?;
        self.mark_action();
        Ok(format!(
            "Pasted {} characters as {format} into {} and restored the previous clipboard contents.",
            text.chars().count(),
            app.name
        ))
    }

    pub fn screenshot(&self, args: &Value) -> Result<Vec<u8>> {
        match optional_app(args) {
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
    use serde_json::json;

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
    fn context_disambiguates_repeated_text() {
        let text = "one target two target three";
        let range = find_range_with_context(text, "target", Some("two "), None)
            .expect("the second occurrence has the requested prefix");
        assert_eq!(range.location, 15);
        assert_eq!(range.length, 6);

        let range = find_range_with_context(text, "target", None, Some(" three"))
            .expect("the second occurrence has the requested suffix");
        assert_eq!(range.location, 15);
        assert!(find_range_with_context(text, "target", Some("three "), None).is_none());
    }

    #[test]
    fn png_header_dimensions_are_read_for_coordinate_scale() {
        let mut png = vec![0u8; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[16..20].copy_from_slice(&2880u32.to_be_bytes());
        png[20..24].copy_from_slice(&1800u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((2880.0, 1800.0)));
        assert_eq!(png_size(b"not a png"), None);
    }

    #[test]
    fn computer_use_mouse_button_spellings_are_accepted() {
        for (value, name) in [
            (json!("left"), "left"),
            (json!("r"), "right"),
            (json!(2), "middle"),
        ] {
            let button = mouse_button(&json!({ "mouse_button": value })).unwrap();
            assert_eq!(button_name(&button), name);
        }
        assert!(mouse_button(&json!({ "mouse_button": "fourth" })).is_err());
    }

    #[test]
    fn only_the_hosting_bundle_counts_as_driving_itself() {
        assert!(drives_itself(
            Some("/Applications/Oxide.app"),
            Some("/Applications/Oxide.app/")
        ));
        assert!(!drives_itself(
            Some("/Applications/Oxide.app"),
            Some("/Applications/TextEdit.app")
        ));
        assert!(!drives_itself(Some("/Applications/Oxide.app"), None));
        assert!(!drives_itself(None, Some("/Applications/Oxide.app")));
        assert!(!drives_itself(Some("/Applications/Oxide.app"), Some("")));
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
    fn an_action_reserves_a_one_second_settling_window() {
        let action = Instant::now();
        let wait = settle_wait(Some(action), action + Duration::from_millis(250));
        assert_eq!(wait, Duration::from_millis(750));
        assert_eq!(
            settle_wait(Some(action), action + Duration::from_secs(2)),
            Duration::ZERO
        );
        assert_eq!(settle_wait(None, action), Duration::ZERO);
    }

    #[test]
    fn loading_markers_trigger_bounded_settling() {
        assert!(line_looks_loading("AXProgressIndicator value=42"));
        assert!(line_looks_loading("AXButton \"Loading\" · busy"));
        assert!(line_looks_loading("AXStaticText value=\"Loading…\""));
        assert!(!line_looks_loading("AXButton \"Load file\""));
    }

    #[test]
    fn arguments_are_read_leniently() {
        let args = serde_json::json!({ "x": "12.5", "y": 3, "flag": true });
        assert_eq!(number_arg(&args, "x"), Some(12.5));
        assert_eq!(number_arg(&args, "y"), Some(3.0));
        assert!(bool_arg(&args, "flag"));
        assert_eq!(usize_arg(&args, "y"), Some(3));
    }

    #[test]
    fn app_mentions_are_accepted_as_identifiers() {
        assert_eq!(
            optional_app(&json!({ "app": " @Google Chrome " })),
            Some("Google Chrome".to_string())
        );
        assert_eq!(
            optional_app(&json!({ "app": "@com.apple.TextEdit" })),
            Some("com.apple.TextEdit".to_string())
        );
    }
}
