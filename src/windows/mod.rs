//! Windows backend over UI Automation and foreground SendInput.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use uiautomation::inputs::{Keyboard, Mouse, MouseButton};
use uiautomation::patterns::{UIScrollPattern, UITextPattern, UITogglePattern, UIValuePattern};
use uiautomation::screenshots::Screenshot;
use uiautomation::types::{ControlType, Point, ScrollAmount};
use uiautomation::{UIAutomation, UIElement, UITreeWalker};
use windows::core::PWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::policy::{self, Access, Identity, Policy};
use crate::runtime::State;

const DEFAULT_MAX_OUTPUT_CHARS: usize = 20_000;
const DEFAULT_MAX_TREE_NODES: usize = 1200;
const DEFAULT_MAX_TREE_DEPTH: usize = 64;
const DEFAULT_TEXT_LIMIT: usize = 200;
const ACTION_SETTLE: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct Window {
    hwnd: isize,
    pid: u32,
    title: String,
    class: String,
    process_path: Option<String>,
    process_name: String,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    element: UIElement,
}

#[derive(Clone)]
struct App {
    pid: u32,
    name: String,
    process_path: Option<String>,
    windows: Vec<Window>,
}

#[derive(Clone)]
struct Cached {
    element: UIElement,
    runtime_id: Vec<i32>,
    role: String,
    text: String,
    center: Option<Point>,
}

struct Snapshot {
    nodes: Vec<Cached>,
    truncated: bool,
}

pub struct Runtime {
    automation: Result<UIAutomation, String>,
    nodes: Vec<Cached>,
    cached_app: Option<String>,
    cached_window: Option<isize>,
    policy: Policy,
    last_action: Option<std::time::Instant>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            automation: UIAutomation::new().map_err(|error| error.to_string()),
            nodes: Vec::new(),
            cached_app: None,
            cached_window: None,
            policy: Policy::load(),
            last_action: None,
        }
    }
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn app_arg(args: &Value) -> Option<String> {
    string_arg(args, "app").map(|app| app.trim_start_matches('@').trim().to_string())
}

fn required_app(args: &Value) -> Result<String> {
    app_arg(args).ok_or_else(|| anyhow!("`app` is required; call list_apps first"))
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

fn text_limit(args: &Value) -> usize {
    match args.get("text_limit") {
        Some(Value::String(text)) if text.eq_ignore_ascii_case("max") => usize::MAX,
        Some(Value::Number(number)) => {
            number.as_u64().unwrap_or(DEFAULT_TEXT_LIMIT as u64) as usize
        }
        _ => DEFAULT_TEXT_LIMIT,
    }
}

fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(limit).collect();
    format!("{head}… ({} chars)", trimmed.chars().count())
}

fn cap_output(text: String, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text;
    }
    let kept: String = text.chars().take(limit).collect();
    format!(
        "{kept}\n\n[output truncated at {limit} of {count} characters; narrow it with `query`, `max_tree_nodes`, or a higher max_output_chars]"
    )
}

fn control_name(kind: ControlType) -> &'static str {
    match kind {
        ControlType::Button => "Button",
        ControlType::Calendar => "Calendar",
        ControlType::CheckBox => "CheckBox",
        ControlType::ComboBox => "ComboBox",
        ControlType::Edit => "Edit",
        ControlType::Hyperlink => "Hyperlink",
        ControlType::Image => "Image",
        ControlType::ListItem => "ListItem",
        ControlType::List => "List",
        ControlType::Menu => "Menu",
        ControlType::MenuBar => "MenuBar",
        ControlType::MenuItem => "MenuItem",
        ControlType::ProgressBar => "ProgressBar",
        ControlType::RadioButton => "RadioButton",
        ControlType::ScrollBar => "ScrollBar",
        ControlType::Slider => "Slider",
        ControlType::Spinner => "Spinner",
        ControlType::StatusBar => "StatusBar",
        ControlType::Tab => "Tab",
        ControlType::TabItem => "TabItem",
        ControlType::Text => "Text",
        ControlType::ToolBar => "ToolBar",
        ControlType::ToolTip => "ToolTip",
        ControlType::Tree => "Tree",
        ControlType::TreeItem => "TreeItem",
        ControlType::Custom => "Custom",
        ControlType::Group => "Group",
        ControlType::Thumb => "Thumb",
        ControlType::DataGrid => "DataGrid",
        ControlType::DataItem => "DataItem",
        ControlType::Document => "Document",
        ControlType::SplitButton => "SplitButton",
        ControlType::Window => "Window",
        ControlType::Pane => "Pane",
        ControlType::Header => "Header",
        ControlType::HeaderItem => "HeaderItem",
        ControlType::Table => "Table",
        ControlType::TitleBar => "TitleBar",
        ControlType::Separator => "Separator",
        _ => "Control",
    }
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        if handle.is_invalid() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        buffer.truncate(len as usize);
        Some(String::from_utf16_lossy(&buffer))
    }
}

fn process_name(path: Option<&str>) -> String {
    path.and_then(|path| Path::new(path).file_stem())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Application")
        .to_string()
}

fn identity(app: &App) -> Option<Identity> {
    let path = app.process_path.clone()?;
    let name = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("application")
        .to_string();
    Some(Identity::new(name, path))
}

fn rect_center(element: &UIElement) -> Option<Point> {
    let rect = element.get_bounding_rectangle().ok()?;
    if rect.get_width() <= 0 || rect.get_height() <= 0 {
        return None;
    }
    Some(Point::new(
        rect.get_left() + rect.get_width() / 2,
        rect.get_top() + rect.get_height() / 2,
    ))
}

fn describe(element: &UIElement, text_limit: usize, include_frames: bool) -> String {
    let role = element
        .get_control_type()
        .map(control_name)
        .unwrap_or("Control");
    let mut parts = vec![role.to_string()];
    if let Ok(name) = element.get_name() {
        if !name.trim().is_empty() {
            parts.push(format!("\"{}\"", truncate(&name, text_limit)));
        }
    }
    if let Ok(value) = element.get_pattern::<UIValuePattern>() {
        if let Ok(value) = value.get_value() {
            if !value.trim().is_empty() {
                parts.push(format!("value=\"{}\"", truncate(&value, text_limit)));
            }
        }
    }
    if let Ok(id) = element.get_automation_id() {
        if !id.trim().is_empty() {
            parts.push(format!("id={id}"));
        }
    }
    if let Ok(help) = element.get_help_text() {
        if !help.trim().is_empty()
            && !parts
                .iter()
                .any(|part| part.contains(&truncate(&help, text_limit)))
        {
            parts.push(format!("help=\"{}\"", truncate(&help, text_limit)));
        }
    }
    if element.is_enabled() == Ok(false) {
        parts.push("· disabled".to_string());
    }
    if include_frames {
        if let Some(center) = rect_center(element) {
            parts.push(format!("@({},{})", center.get_x(), center.get_y()));
        }
    }
    parts.join(" ")
}

struct Walk<'a> {
    walker: &'a UITreeWalker,
    max_depth: usize,
    max_nodes: usize,
    text_limit: usize,
    include_frames: bool,
}

fn walk(context: &Walk<'_>, element: &UIElement, depth: usize, out: &mut Vec<Cached>) -> bool {
    if out.len() >= context.max_nodes {
        return false;
    }
    let runtime_id = element.get_runtime_id().unwrap_or_default();
    let role = element
        .get_control_type()
        .map(control_name)
        .unwrap_or("Control")
        .to_string();
    out.push(Cached {
        element: element.clone(),
        runtime_id,
        role,
        text: describe(element, context.text_limit, context.include_frames),
        center: rect_center(element),
    });
    if depth >= context.max_depth {
        return false;
    }
    let mut complete = true;
    if let Ok(mut child) = context.walker.get_first_child(element) {
        loop {
            if !walk(context, &child, depth + 1, out) {
                complete = false;
            }
            match context.walker.get_next_sibling(&child) {
                Ok(next) => child = next,
                Err(_) => break,
            }
        }
    }
    complete
}

fn render(nodes: &[Cached]) -> String {
    let mut out = String::new();
    for (index, node) in nodes.iter().enumerate() {
        let depth = node.text.chars().take_while(|ch| *ch == ' ').count() / 2;
        out.push_str(&format!(
            "{index:>4} {}{}\n",
            "  ".repeat(depth),
            node.text.replace('\n', " ")
        ));
    }
    out
}

fn diff(previous: &[Cached], current: &[Cached]) -> String {
    if previous.len() != current.len() {
        return format!(
            "The accessibility structure changed ({} to {} elements). Current indexes:\n{}",
            previous.len(),
            current.len(),
            render(current)
        );
    }
    let mut lines = Vec::new();
    for (index, (old, new)) in previous.iter().zip(current).enumerate() {
        if old.text != new.text {
            lines.push(format!("{index:>4} ~ {} (was: {})", new.text, old.text));
        }
    }
    if lines.is_empty() {
        "No accessibility changes since the previous state.".to_string()
    } else {
        format!("Changed accessibility elements:\n{}", lines.join("\n"))
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self::default()
    }

    fn automation(&self) -> Result<&UIAutomation> {
        self.automation
            .as_ref()
            .map_err(|error| anyhow!("Windows UI Automation is unavailable: {error}"))
    }

    fn windows(&self) -> Result<Vec<Window>> {
        let automation = self.automation()?;
        let walker = automation
            .get_control_view_walker()
            .map_err(|error| anyhow!("reading the Windows control-view walker: {error}"))?;
        let root = automation
            .get_root_element()
            .map_err(|error| anyhow!("reading the Windows desktop root: {error}"))?;
        let mut windows = Vec::new();
        let mut current = walker.get_first_child(&root).ok();
        while let Some(element) = current {
            if let (Ok(handle), Ok(pid), Ok(title), Ok(kind)) = (
                element.get_native_window_handle(),
                element.get_process_id(),
                element.get_name(),
                element.get_control_type(),
            ) {
                let hwnd: isize = handle.into();
                let rect = element.get_bounding_rectangle().ok();
                if hwnd != 0
                    && !title.trim().is_empty()
                    && rect.is_some_and(|rect| rect.get_width() > 0 && rect.get_height() > 0)
                    && matches!(kind, ControlType::Window | ControlType::Pane)
                {
                    let rect = rect.unwrap();
                    let path = process_path(pid);
                    windows.push(Window {
                        hwnd,
                        pid,
                        title: title.trim().to_string(),
                        class: element.get_classname().unwrap_or_default(),
                        process_name: process_name(path.as_deref()),
                        process_path: path,
                        left: rect.get_left(),
                        top: rect.get_top(),
                        width: rect.get_width(),
                        height: rect.get_height(),
                        element: element.clone(),
                    });
                }
            }
            current = walker.get_next_sibling(&element).ok();
        }
        windows.sort_by(|left, right| {
            left.process_name
                .to_lowercase()
                .cmp(&right.process_name.to_lowercase())
                .then_with(|| left.title.cmp(&right.title))
        });
        windows.dedup_by_key(|window| window.hwnd);
        Ok(windows)
    }

    fn apps(&self) -> Result<Vec<App>> {
        let mut apps: Vec<App> = Vec::new();
        for window in self.windows()? {
            if let Some(app) = apps.iter_mut().find(|app| app.pid == window.pid) {
                app.windows.push(window);
            } else {
                apps.push(App {
                    pid: window.pid,
                    name: window.process_name.clone(),
                    process_path: window.process_path.clone(),
                    windows: vec![window],
                });
            }
        }
        Ok(apps)
    }

    fn resolve(&self, name: &str) -> Result<App> {
        if let Ok(pid) = name.parse::<u32>() {
            if let Some(app) = self.apps()?.into_iter().find(|app| app.pid == pid) {
                return Ok(app);
            }
        }
        let needle = name.to_lowercase();
        let exact_path = Path::new(name).exists();
        let found = self.apps()?.into_iter().find(|app| {
            app.name.to_lowercase() == needle
                || app
                    .process_path
                    .as_deref()
                    .is_some_and(|path| path.eq_ignore_ascii_case(name))
                || app.windows.iter().any(|window| {
                    window.title.to_lowercase().contains(&needle)
                        || window.class.to_lowercase().contains(&needle)
                })
        });
        if let Some(app) = found {
            return Ok(app);
        }
        if exact_path {
            std::process::Command::new(name)
                .spawn()
                .map_err(|error| anyhow!("launching {name}: {error}"))?;
            for _ in 0..100 {
                if let Some(app) = self.apps()?.into_iter().find(|app| {
                    app.process_path
                        .as_deref()
                        .is_some_and(|path| path.eq_ignore_ascii_case(name))
                }) {
                    return Ok(app);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let known = self
            .apps()
            .unwrap_or_default()
            .into_iter()
            .map(|app| app.name)
            .collect::<Vec<_>>();
        bail!(
            "no Windows app matches `{name}`; running apps: {}",
            known.join(", ")
        )
    }

    fn authorize(&self, app: &App) -> Result<()> {
        let identity = identity(app);
        match self.policy.check(&app.name, identity.as_ref())? {
            Access::Allowed => {}
            Access::OneTime => {
                return self.policy.authorize(&app.name, || identity.clone());
            }
            Access::Blocked(message) => bail!("{message}"),
        }
        if self.policy.terminals() == policy::Fallback::Deny {
            let name = app.name.to_lowercase();
            let terminal = [
                "windows terminal",
                "windowsterminal",
                "cmd",
                "powershell",
                "pwsh",
                "conhost",
                "wt",
            ]
            .iter()
            .any(|terminal| name == *terminal || name.contains(terminal));
            if terminal {
                bail!(
                    "`{}` is a terminal and terminal automation is refused; allow it with `oxide-computer-use apps terminals allow`",
                    app.name
                );
            }
        }
        if self.policy.self_automation() == policy::Fallback::Deny {
            let own = std::env::current_exe().ok();
            if own
                .as_deref()
                .zip(app.process_path.as_deref())
                .is_some_and(|(own, running)| own == Path::new(running))
            {
                bail!(
                    "`{}` is the process hosting this server and self-automation is refused",
                    app.name
                );
            }
        }
        Ok(())
    }

    fn window(&self, app: &App, selector: Option<&str>) -> Result<Window> {
        let Some(selector) = selector else {
            return app
                .windows
                .iter()
                .max_by_key(|window| window.width.saturating_mul(window.height))
                .cloned()
                .ok_or_else(|| anyhow!("{} has no usable window", app.name));
        };
        if let Ok(index) = selector.parse::<usize>() {
            return app
                .windows
                .get(index)
                .cloned()
                .ok_or_else(|| anyhow!("window {index} does not exist"));
        }
        app.windows
            .iter()
            .find(|window| {
                window
                    .title
                    .to_lowercase()
                    .contains(&selector.to_lowercase())
            })
            .cloned()
            .ok_or_else(|| anyhow!("no window title contains `{selector}`"))
    }

    fn snapshot(&self, app: &App, window: &Window, args: &Value) -> Result<Snapshot> {
        let automation = self.automation()?;
        let walker = automation
            .get_control_view_walker()
            .map_err(|error| anyhow!("reading the Windows control-view walker: {error}"))?;
        let max_nodes = usize_arg(args, "max_tree_nodes").unwrap_or(DEFAULT_MAX_TREE_NODES);
        let max_depth = usize_arg(args, "max_tree_depth").unwrap_or(DEFAULT_MAX_TREE_DEPTH);
        let limit = text_limit(args);
        let include_frames = bool_arg(args, "include_frames");
        let mut nodes = Vec::new();
        let context = Walk {
            walker: &walker,
            max_depth,
            max_nodes,
            text_limit: limit,
            include_frames,
        };
        let complete = walk(&context, &window.element, 0, &mut nodes);
        if nodes.is_empty() {
            bail!("could not read {}'s UI Automation tree", app.name);
        }
        Ok(Snapshot {
            nodes,
            truncated: !complete,
        })
    }

    fn settle(&mut self) {
        if let Some(action) = self.last_action.take() {
            let wait = ACTION_SETTLE.saturating_sub(action.elapsed());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
    }

    fn node(&self, index: i64, app: &str) -> Result<&Cached> {
        if self.cached_app.as_deref() != Some(app) {
            bail!("no snapshot for `{app}`; call get_app_state first");
        }
        let index = usize::try_from(index).map_err(|_| anyhow!("element_index must be >= 0"))?;
        let node = self
            .nodes
            .get(index)
            .ok_or_else(|| anyhow!("element_index {index} is out of range"))?;
        if node.element.get_runtime_id().ok().as_deref() != Some(node.runtime_id.as_slice()) {
            bail!("element {index} changed since the snapshot; call get_app_state again");
        }
        Ok(node)
    }

    pub fn list_apps(&self, _include_helpers: bool) -> Result<String> {
        self.guard()?;
        let apps = self
            .apps()?
            .into_iter()
            .map(|app| {
                let marker = self
                    .policy
                    .access(&app.name, identity(&app).as_ref())
                    .marker()
                    .trim();
                serde_json::json!({
                    "id": app.process_path.clone().unwrap_or_else(|| app.pid.to_string()),
                    "displayName": app.name,
                    "path": app.process_path,
                    "isRunning": true,
                    "windowTitles": app.windows.iter().map(|window| window.title.clone()).collect::<Vec<_>>(),
                    "policy": marker,
                    "pid": app.pid,
                })
            })
            .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&apps)?)
    }

    pub fn app_policy(&mut self, args: &Value) -> Result<String> {
        let action = string_arg(args, "action").unwrap_or_else(|| "list".to_string());
        let app_name = || app_arg(args).ok_or_else(|| anyhow!("`app` is required"));
        let identity_for = |app: &str| self.resolve(app).ok().and_then(|app| identity(&app));
        match action.as_str() {
            "list" => {}
            "allow" => {
                let app = app_name()?;
                self.policy.allow(&app, identity_for(&app))?;
            }
            "grant_once" | "grant-once" => {
                let app = app_name()?;
                self.policy.grant_once(&app, identity_for(&app))?;
            }
            "deny" => self.policy.deny(&app_name()?)?,
            "revoke" => self.policy.revoke(&app_name()?)?,
            "set_default" | "set-default" => {
                let mode = string_arg(args, "mode")
                    .and_then(|mode| policy::Fallback::parse(&mode))
                    .ok_or_else(|| anyhow!("`mode` must be allow or deny"))?;
                self.policy.set_fallback(mode)?;
            }
            "set_self_automation" | "set-self-automation" => {
                let mode = string_arg(args, "mode")
                    .and_then(|mode| policy::Fallback::parse(&mode))
                    .ok_or_else(|| anyhow!("`mode` must be allow or deny"))?;
                self.policy.set_self_automation(mode)?;
            }
            "set_terminals" | "set-terminals" => {
                let mode = string_arg(args, "mode")
                    .and_then(|mode| policy::Fallback::parse(&mode))
                    .ok_or_else(|| anyhow!("`mode` must be allow or deny"))?;
                self.policy.set_terminals(mode)?;
            }
            "reset" => self.policy.clear()?,
            other => bail!("unknown app policy action `{other}`"),
        }
        self.policy.reload();
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
            "apps": self.apps()?.into_iter().map(|app| serde_json::json!({
                "id": app.process_path.clone().unwrap_or_else(|| app.pid.to_string()),
                "displayName": app.name,
                "path": app.process_path,
                "isRunning": true,
                "policy": self.policy.access(&app.name, identity(&app).as_ref()).marker().trim(),
            })).collect::<Vec<_>>(),
        }))?)
    }

    pub fn get_app_state(&mut self, args: &Value) -> Result<State> {
        self.guard()?;
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, string_arg(args, "window").as_deref())?;
        self.settle();
        let snapshot = self.snapshot(&app, &window, args)?;
        let disable_diff = bool_arg(args, "disableDiff") || bool_arg(args, "disable_diff");
        let body = if !disable_diff
            && self.cached_app.as_deref() == Some(&app.name)
            && self.cached_window == Some(window.hwnd)
            && !self.nodes.is_empty()
        {
            diff(&self.nodes, &snapshot.nodes)
        } else {
            render(&snapshot.nodes)
        };
        let count = snapshot.nodes.len();
        self.cached_app = Some(app.name.clone());
        self.cached_window = Some(window.hwnd);
        self.nodes = snapshot.nodes;
        let mut text = format!(
            "App: {} (pid {})\nWindow: \"{}\" {}x{}\nElements: {}{}\n\n{}",
            app.name,
            app.pid,
            window.title,
            window.width,
            window.height,
            count,
            if snapshot.truncated {
                " (truncated; raise max_tree_nodes or max_tree_depth)"
            } else {
                ""
            },
            body
        );
        let limit = usize_arg(args, "max_output_chars").unwrap_or(DEFAULT_MAX_OUTPUT_CHARS);
        text = cap_output(text, limit);
        let mut image = None;
        if args
            .get("include_screenshot")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            match self.capture_window(&window) {
                Ok(bytes) => image = Some(bytes),
                Err(error) => text.push_str(&format!("\n\nScreenshot unavailable: {error:#}")),
            }
        }
        Ok(State { text, image })
    }

    pub fn query(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, string_arg(args, "window").as_deref())?;
        let snapshot = self.snapshot(&app, &window, args)?;
        let needle = string_arg(args, "text").map(|text| text.to_lowercase());
        let role = string_arg(args, "role").map(|role| role.to_lowercase());
        let limit = usize_arg(args, "max_results").unwrap_or(20);
        let matches: Vec<_> = snapshot
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                needle
                    .as_deref()
                    .map(|needle| node.text.to_lowercase().contains(needle))
                    .unwrap_or(true)
                    && role
                        .as_deref()
                        .map(|role| node.role.to_lowercase().contains(role))
                        .unwrap_or(true)
            })
            .take(limit)
            .map(|(index, node)| format!("{index:>4} {}", node.text))
            .collect();
        self.cached_app = Some(app.name.clone());
        self.cached_window = Some(window.hwnd);
        self.nodes = snapshot.nodes;
        if matches.is_empty() {
            Ok(format!("No element in {} matched.", app.name))
        } else {
            Ok(format!(
                "{} matches in {}:\n{}",
                matches.len(),
                app.name,
                matches.join("\n")
            ))
        }
    }

    fn point(&self, window: &Window, x: f64, y: f64) -> Point {
        Point::new(window.left + x as i32, window.top + y as i32)
    }

    pub fn click(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, None)?;
        let button = match string_arg(args, "mouse_button").as_deref() {
            Some("right" | "r") => MouseButton::RIGHT,
            Some("middle" | "m") => MouseButton::MIDDLE,
            _ => MouseButton::LEFT,
        };
        let count = usize_arg(args, "click_count").unwrap_or(1).max(1);
        let Some(index) = integer_arg(args, "element_index") else {
            let x = number_arg(args, "x").ok_or_else(|| anyhow!("pass element_index or x/y"))?;
            let y = number_arg(args, "y").ok_or_else(|| anyhow!("pass element_index or x/y"))?;
            let point = self.point(&window, x, y);
            let mouse = Mouse::new();
            mouse.move_to(&point).map_err(|error| anyhow!("{error}"))?;
            for _ in 0..count {
                mouse
                    .click_button(button)
                    .map_err(|error| anyhow!("{error}"))?;
            }
            self.last_action = Some(std::time::Instant::now());
            return Ok(format!("Clicked Windows coordinate ({x:.0},{y:.0})."));
        };
        let node = self.node(index, &app.name)?.clone();
        if count == 1 && button == MouseButton::LEFT {
            if let Ok(invoke) = node
                .element
                .get_pattern::<uiautomation::patterns::UIInvokePattern>()
            {
                invoke.invoke().map_err(|error| anyhow!("{error}"))?;
                self.last_action = Some(std::time::Instant::now());
                return Ok(format!(
                    "Clicked element {index} ({}) with Invoke.",
                    node.role
                ));
            }
        }
        let point = node
            .center
            .ok_or_else(|| anyhow!("element {index} has no clickable rectangle"))?;
        let mouse = Mouse::new();
        mouse.move_to(&point).map_err(|error| anyhow!("{error}"))?;
        for _ in 0..count {
            mouse
                .click_button(button)
                .map_err(|error| anyhow!("{error}"))?;
        }
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Clicked element {index} ({}) with a pointer event.",
            node.role
        ))
    }

    pub fn perform_action(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let index = integer_arg(args, "element_index")
            .ok_or_else(|| anyhow!("`element_index` is required"))?;
        let action = string_arg(args, "action").unwrap_or_else(|| "Invoke".to_string());
        let node = self.node(index, &app.name)?.clone();
        let lower = action.to_lowercase().replace(['_', ' '], "");
        match lower.as_str() {
            "invoke" | "press" | "axpress" => node
                .element
                .get_pattern::<uiautomation::patterns::UIInvokePattern>()
                .map_err(|error| anyhow!("{error}"))?
                .invoke()
                .map_err(|error| anyhow!("{error}"))?,
            "toggle" => node
                .element
                .get_pattern::<UITogglePattern>()
                .map_err(|error| anyhow!("{error}"))?
                .toggle()
                .map_err(|error| anyhow!("{error}"))?,
            "expand" => node
                .element
                .get_pattern::<uiautomation::patterns::UIExpandCollapsePattern>()
                .map_err(|error| anyhow!("{error}"))?
                .expand()
                .map_err(|error| anyhow!("{error}"))?,
            "collapse" => node
                .element
                .get_pattern::<uiautomation::patterns::UIExpandCollapsePattern>()
                .map_err(|error| anyhow!("{error}"))?
                .collapse()
                .map_err(|error| anyhow!("{error}"))?,
            "select" => node
                .element
                .get_pattern::<uiautomation::patterns::UISelectionItemPattern>()
                .map_err(|error| anyhow!("{error}"))?
                .select()
                .map_err(|error| anyhow!("{error}"))?,
            "focus" => node
                .element
                .set_focus()
                .map_err(|error| anyhow!("{error}"))?,
            "scrollintoview" => node
                .element
                .get_pattern::<uiautomation::patterns::UIScrollItemPattern>()
                .map_err(|error| anyhow!("{error}"))?
                .scroll_into_view()
                .map_err(|error| anyhow!("{error}"))?,
            _ => bail!("unsupported Windows accessibility action `{action}`"),
        }
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Performed {action} on element {index} ({}).",
            node.role
        ))
    }

    pub fn set_value(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let index = integer_arg(args, "element_index")
            .ok_or_else(|| anyhow!("`element_index` is required"))?;
        let value = args
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`value` is required"))?;
        let node = self.node(index, &app.name)?.clone();
        let pattern = node
            .element
            .get_pattern::<UIValuePattern>()
            .map_err(|error| anyhow!("element {index} has no Value pattern: {error}"))?;
        pattern
            .set_value(value)
            .map_err(|error| anyhow!("{error}"))?;
        let read_back = pattern.get_value().map_err(|error| anyhow!("{error}"))?;
        if read_back != value {
            bail!("element {index} still reads {read_back:?} after writing {value:?}");
        }
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Set element {index} value and read {value:?} back."
        ))
    }

    pub fn select_text(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let index = integer_arg(args, "element_index")
            .ok_or_else(|| anyhow!("`element_index` is required"))?;
        let needle = string_arg(args, "text").ok_or_else(|| anyhow!("`text` is required"))?;
        let selection = string_arg(args, "selection_type").unwrap_or_else(|| "text".to_string());
        let node = self.node(index, &app.name)?.clone();
        let pattern = node
            .element
            .get_pattern::<UITextPattern>()
            .map_err(|error| anyhow!("element {index} has no Text pattern: {error}"))?;
        let document = pattern
            .get_document_range()
            .map_err(|error| anyhow!("{error}"))?;
        let text = document.get_text(-1).map_err(|error| anyhow!("{error}"))?;
        let char_index = text
            .to_lowercase()
            .find(&needle.to_lowercase())
            .map(|byte| text[..byte].chars().count())
            .ok_or_else(|| anyhow!("`{needle}` is not in the element text"))?;
        let range = pattern
            .get_document_range()
            .map_err(|error| anyhow!("{error}"))?;
        let start = if selection == "cursor_after" {
            char_index + needle.chars().count()
        } else {
            char_index
        };
        let end = if selection == "text" {
            start + needle.chars().count()
        } else {
            start
        };
        use uiautomation::types::{TextPatternRangeEndpoint, TextUnit};
        range
            .move_endpoint_by_unit(
                TextPatternRangeEndpoint::Start,
                TextUnit::Character,
                start as i32,
            )
            .map_err(|error| anyhow!("{error}"))?;
        range
            .move_endpoint_by_unit(
                TextPatternRangeEndpoint::End,
                TextUnit::Character,
                end as i32,
            )
            .map_err(|error| anyhow!("{error}"))?;
        range.select().map_err(|error| anyhow!("{error}"))?;
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Selected `{needle}` in element {index} ({selection})."
        ))
    }

    pub fn type_text(&mut self, args: &Value) -> Result<String> {
        if bool_arg(args, "background") && app_arg(args).is_none() {
            bail!("`background` needs `app`: the input is addressed to that process");
        }
        let name = required_app(args)?;
        if bool_arg(args, "background") {
            bail!("background keyboard delivery is unavailable on Windows; Computer Use uses the foreground desktop");
        }
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, None)?;
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`text` is required"))?;
        window
            .element
            .set_focus()
            .map_err(|error| anyhow!("{error}"))?;
        let keyboard = Keyboard::new();
        for (index, segment) in text.split('\n').enumerate() {
            if index > 0 {
                keyboard
                    .send_keys("{enter}")
                    .map_err(|error| anyhow!("{error}"))?;
            }
            if !segment.is_empty() {
                keyboard
                    .send_text(segment)
                    .map_err(|error| anyhow!("{error}"))?;
            }
        }
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Typed {} characters into {} on the foreground Windows desktop.",
            text.chars().count(),
            app.name
        ))
    }

    pub fn press_key(&mut self, args: &Value) -> Result<String> {
        let key = string_arg(args, "key").ok_or_else(|| anyhow!("`key` is required"))?;
        let send_keys = windows_keys(&key)?;
        let name = required_app(args)?;
        if bool_arg(args, "background") {
            bail!("background keyboard delivery is unavailable on Windows; Computer Use uses the foreground desktop");
        }
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, None)?;
        window
            .element
            .set_focus()
            .map_err(|error| anyhow!("{error}"))?;
        Keyboard::new()
            .send_keys(&send_keys)
            .map_err(|error| anyhow!("{error}"))?;
        self.last_action = Some(std::time::Instant::now());
        Ok(format!("Pressed Windows key `{key}` in {}.", app.name))
    }

    pub fn scroll(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, None)?;
        let direction = string_arg(args, "direction").unwrap_or_else(|| "down".to_string());
        let pages = number_arg(args, "pages").unwrap_or(1.0).max(1.0).round() as usize;
        let element = if let Some(index) = integer_arg(args, "element_index") {
            self.node(index, &app.name)?.element.clone()
        } else if let (Some(x), Some(y)) = (number_arg(args, "x"), number_arg(args, "y")) {
            self.automation()?
                .element_from_point(self.point(&window, x, y))
                .map_err(|error| anyhow!("{error}"))?
        } else {
            window.element.clone()
        };
        let scroll = element
            .get_pattern::<UIScrollPattern>()
            .map_err(|error| anyhow!("the target has no Scroll pattern: {error}"))?;
        for _ in 0..pages {
            let (horizontal, vertical) = match direction.as_str() {
                "up" | "u" => (ScrollAmount::NoAmount, ScrollAmount::LargeDecrement),
                "down" | "d" => (ScrollAmount::NoAmount, ScrollAmount::LargeIncrement),
                "left" | "l" => (ScrollAmount::LargeDecrement, ScrollAmount::NoAmount),
                "right" | "r" => (ScrollAmount::LargeIncrement, ScrollAmount::NoAmount),
                _ => bail!("`direction` must be up, down, left or right"),
            };
            scroll
                .scroll(horizontal, vertical)
                .map_err(|error| anyhow!("{error}"))?;
        }
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Scrolled {direction} {pages} page(s) in {}.",
            app.name
        ))
    }

    pub fn drag(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, None)?;
        let from = self.point(
            &window,
            number_arg(args, "from_x").ok_or_else(|| anyhow!("`from_x` is required"))?,
            number_arg(args, "from_y").ok_or_else(|| anyhow!("`from_y` is required"))?,
        );
        let to = self.point(
            &window,
            number_arg(args, "to_x").ok_or_else(|| anyhow!("`to_x` is required"))?,
            number_arg(args, "to_y").ok_or_else(|| anyhow!("`to_y` is required"))?,
        );
        window
            .element
            .set_focus()
            .map_err(|error| anyhow!("{error}"))?;
        let mouse = Mouse::new().auto_move(false);
        Mouse::set_cursor_pos(&from).map_err(|error| anyhow!("{error}"))?;
        mouse
            .drag_to(MouseButton::LEFT, &to)
            .map_err(|error| anyhow!("{error}"))?;
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Dragged in {} from ({},{}) to ({},{}).",
            app.name,
            from.get_x(),
            from.get_y(),
            to.get_x(),
            to.get_y()
        ))
    }

    pub fn paste(&mut self, args: &Value) -> Result<String> {
        let name = required_app(args)?;
        let app = self.resolve(&name)?;
        self.authorize(&app)?;
        let window = self.window(&app, None)?;
        let format = string_arg(args, "format").unwrap_or_else(|| "text".to_string());
        if format == "html" {
            bail!("Windows paste currently supports text and md, not html");
        }
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("`text` is required"))?;
        window
            .element
            .send_text_by_clipboard(text)
            .map_err(|error| anyhow!("{error}"))?;
        self.last_action = Some(std::time::Instant::now());
        Ok(format!(
            "Pasted text into {} and restored the clipboard.",
            app.name
        ))
    }

    fn capture_window(&self, window: &Window) -> Result<Vec<u8>> {
        let screenshot = Screenshot::capture_rect(
            window
                .element
                .get_bounding_rectangle()
                .map_err(|error| anyhow!("{error}"))?,
        )
        .map_err(|error| anyhow!("{error}"))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "oxide-computer-use-windows-{}-{stamp}.png",
            std::process::id()
        ));
        screenshot
            .save_png(&path)
            .map_err(|error| anyhow!("{error}"))?;
        let bytes = std::fs::read(&path)?;
        let _ = std::fs::remove_file(path);
        Ok(bytes)
    }

    pub fn screenshot(&self, args: &Value) -> Result<Vec<u8>> {
        self.guard()?;
        if let Some(name) = app_arg(args) {
            let app = self.resolve(&name)?;
            self.authorize(&app)?;
            let window = self.window(&app, string_arg(args, "window").as_deref())?;
            self.capture_window(&window)
        } else {
            let screenshot = Screenshot::capture_desktop().map_err(|error| anyhow!("{error}"))?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "oxide-computer-use-windows-desktop-{}-{stamp}.png",
                std::process::id()
            ));
            screenshot
                .save_png(&path)
                .map_err(|error| anyhow!("{error}"))?;
            let bytes = std::fs::read(&path)?;
            let _ = std::fs::remove_file(path);
            Ok(bytes)
        }
    }

    pub fn guard(&self) -> Result<()> {
        self.automation().map(|_| ())
    }
}

fn windows_keys(spec: &str) -> Result<String> {
    let spec = spec.trim();
    if spec == "\\n" {
        return Ok("{enter}".to_string());
    }
    let (modifiers, key) = spec.rsplit_once('+').unwrap_or(("", spec));
    let mut output = String::new();
    for modifier in modifiers.split('+').filter(|part| !part.trim().is_empty()) {
        let modifier = match modifier.trim().to_lowercase().as_str() {
            "cmd" | "command" | "super" | "super_l" | "win" | "meta" => "{Win}",
            "ctrl" | "control" | "control_l" => "{Ctrl}",
            "shift" | "shift_l" => "{Shift}",
            "alt" | "option" | "alt_l" => "{Alt}",
            other => bail!("unknown Windows modifier `{other}`"),
        };
        output.push_str(modifier);
    }
    let key = match key.trim().to_lowercase().as_str() {
        "return" | "enter" => "{enter}".to_string(),
        "tab" => "{tab}".to_string(),
        "escape" | "esc" => "{esc}".to_string(),
        "delete" | "backspace" => "{backspace}".to_string(),
        "forward_delete" => "{delete}".to_string(),
        "up" => "{up}".to_string(),
        "down" => "{down}".to_string(),
        "left" => "{left}".to_string(),
        "right" => "{right}".to_string(),
        "home" => "{home}".to_string(),
        "end" => "{end}".to_string(),
        "page_up" | "prior" => "{pgup}".to_string(),
        "page_down" | "next" => "{pgdn}".to_string(),
        "space" => " ".to_string(),
        key if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => format!("{{{key}}}"),
        key if key.chars().count() == 1 => key.to_string(),
        other => bail!(
            "unknown key `{other}` for Windows; use Return, Tab, Escape, arrows, Page_Up/Page_Down, F1-F20 or a single character"
        ),
    };
    output.push_str(&key);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_chords_use_send_keys_names() {
        assert_eq!(windows_keys("super+c").unwrap(), "{Win}c");
        assert_eq!(windows_keys("Control_L+a").unwrap(), "{Ctrl}a");
        assert_eq!(windows_keys("Return").unwrap(), "{enter}");
        assert_eq!(windows_keys("\\n").unwrap(), "{enter}");
    }

    #[test]
    fn app_mentions_are_stripped() {
        assert_eq!(
            app_arg(&serde_json::json!({ "app": "@notepad" })),
            Some("notepad".to_string())
        );
    }
}
