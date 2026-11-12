//! Synthetic input: pointer events, keyboard events and scrolling.

use anyhow::{bail, Result};
use core_foundation_sys::base::{CFRelease, CFTypeRef};

use super::ax::CGPoint;
use super::takeover::Takeover;

type CGEventRef = CFTypeRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateMouseEvent(
        source: CFTypeRef,
        mouse_type: u32,
        position: CGPoint,
        mouse_button: u32,
    ) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(source: CFTypeRef, key: u16, down: bool) -> CGEventRef;
    fn CGEventKeyboardSetUnicodeString(event: CGEventRef, length: usize, string: *const u16);
    fn CGEventCreateScrollWheelEvent2(
        source: CFTypeRef,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventSetLocation(event: CGEventRef, location: CGPoint);
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGEventPostToPid(pid: i32, event: CGEventRef);
    fn CGEventSourceCreate(state_id: i32) -> CFTypeRef;
    fn CGEventSourceSetLocalEventsSuppressionInterval(source: CFTypeRef, seconds: f64);
}

const HID_SYSTEM_STATE: i32 = 0;
const MOUSE_EVENT_DELTA_X: u32 = 4;
const MOUSE_EVENT_DELTA_Y: u32 = 5;
const UI_SETTLED: Duration = Duration::from_millis(35);
const DRAG_STEP: Duration = Duration::from_millis(12);

const HID_EVENT_TAP: u32 = 0;

const LEFT_MOUSE_DOWN: u32 = 1;
const LEFT_MOUSE_UP: u32 = 2;
const RIGHT_MOUSE_DOWN: u32 = 3;
const RIGHT_MOUSE_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_MOUSE_DRAGGED: u32 = 6;
const OTHER_MOUSE_DOWN: u32 = 25;
const OTHER_MOUSE_UP: u32 = 26;

use std::time::Duration;

const SCROLL_UNIT_PIXEL: u32 = 0;
const MOUSE_EVENT_CLICK_STATE: u32 = 1;

const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_OPTION: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

struct Event(CGEventRef);

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

/// A private HID event source: without it the window server may coalesce or
/// suppress events that arrive faster than a person could produce them.
struct Source(CFTypeRef);

impl Source {
    fn hid() -> Source {
        unsafe {
            let raw = CGEventSourceCreate(HID_SYSTEM_STATE);
            if !raw.is_null() {
                CGEventSourceSetLocalEventsSuppressionInterval(raw, 0.0);
            }
            Source(raw)
        }
    }

    fn raw(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn pace(duration: Duration) {
    std::thread::sleep(duration);
}

fn post(raw: CFTypeRef) -> Result<()> {
    if raw.is_null() {
        bail!("the window server refused to create the event");
    }
    let event = Event(raw);
    unsafe { CGEventPost(HID_EVENT_TAP, event.0) };
    Ok(())
}

fn mouse_event(source: &Source, kind: u32, point: CGPoint, index: u32) -> Event {
    unsafe { Event(CGEventCreateMouseEvent(source.raw(), kind, point, index)) }
}

/// A click sequence with the options Computer Use exposes: repeated click
/// count, a held mouse button, and modifier flags held across the sequence.
pub fn click_with(
    point: CGPoint,
    button: Button,
    click_count: usize,
    duration: Duration,
    flags: u64,
) -> Result<()> {
    let (down, up, index) = button.events();
    let source = Source::hid();
    unsafe {
        let moved = mouse_event(&source, MOUSE_MOVED, point, 0);
        if moved.0.is_null() {
            bail!("the window server refused to create the mouse event");
        }
        if flags != 0 {
            CGEventSetFlags(moved.0, flags);
        }
        CGEventPost(HID_EVENT_TAP, moved.0);
        pace(UI_SETTLED);
        for count in 1..=click_count {
            let down_event = mouse_event(&source, down, point, index);
            let up_event = mouse_event(&source, up, point, index);
            if down_event.0.is_null() || up_event.0.is_null() {
                bail!("the window server refused to create the mouse event");
            }
            if flags != 0 {
                CGEventSetFlags(down_event.0, flags);
                CGEventSetFlags(up_event.0, flags);
            }
            CGEventSetIntegerValueField(down_event.0, MOUSE_EVENT_CLICK_STATE, count as i64);
            CGEventSetIntegerValueField(up_event.0, MOUSE_EVENT_CLICK_STATE, count as i64);
            CGEventPost(HID_EVENT_TAP, down_event.0);
            pace(duration);
            CGEventPost(HID_EVENT_TAP, up_event.0);
            if count < click_count {
                pace(UI_SETTLED);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn events(self) -> (u32, u32, u32) {
        match self {
            Button::Left => (LEFT_MOUSE_DOWN, LEFT_MOUSE_UP, 0),
            Button::Right => (RIGHT_MOUSE_DOWN, RIGHT_MOUSE_UP, 1),
            Button::Middle => (OTHER_MOUSE_DOWN, OTHER_MOUSE_UP, 2),
        }
    }
}

/// A drag that a window manager and a text view both accept: the pointer
/// arrives, the button goes down after the app has seen the move, the steps are
/// paced, and the button comes up only after the last step landed.
pub fn drag(from: CGPoint, to: CGPoint) -> Result<()> {
    let source = Source::hid();
    unsafe {
        post(CGEventCreateMouseEvent(source.raw(), MOUSE_MOVED, from, 0))?;
        pace(UI_SETTLED);
        let down_event = Event(CGEventCreateMouseEvent(
            source.raw(),
            LEFT_MOUSE_DOWN,
            from,
            0,
        ));
        if down_event.0.is_null() {
            bail!("the window server refused to create the mouse event");
        }
        CGEventSetIntegerValueField(down_event.0, MOUSE_EVENT_CLICK_STATE, 1);
        CGEventPost(HID_EVENT_TAP, down_event.0);
        pace(UI_SETTLED);

        let watch = Takeover::start();
        let steps = 16;
        let mut previous = from;
        for step in 1..=steps {
            let ratio = step as f64 / steps as f64;
            let point = CGPoint {
                x: from.x + (to.x - from.x) * ratio,
                y: from.y + (to.y - from.y) * ratio,
            };
            let event = Event(CGEventCreateMouseEvent(
                source.raw(),
                LEFT_MOUSE_DRAGGED,
                point,
                0,
            ));
            if event.0.is_null() {
                bail!("the window server refused to create the drag event");
            }
            CGEventSetIntegerValueField(
                event.0,
                MOUSE_EVENT_DELTA_X,
                (point.x - previous.x) as i64,
            );
            CGEventSetIntegerValueField(
                event.0,
                MOUSE_EVENT_DELTA_Y,
                (point.y - previous.y) as i64,
            );
            CGEventPost(HID_EVENT_TAP, event.0);
            previous = point;
            pace(DRAG_STEP);
            if let Some(kind) = watch.as_ref().and_then(Takeover::poll) {
                post(CGEventCreateMouseEvent(
                    source.raw(),
                    LEFT_MOUSE_UP,
                    previous,
                    0,
                ))?;
                bail!(
                    "{kind} arrived from a person while the drag was running, so the drag stopped and the button was released. That is user takeover: leave the desktop to them, ask what is next, and re-read the app with get_app_state before acting again"
                );
            }
        }

        pace(UI_SETTLED);
        post(CGEventCreateMouseEvent(source.raw(), LEFT_MOUSE_UP, to, 0))?;
    }
    Ok(())
}

pub fn scroll(point: Option<CGPoint>, horizontal: i32, vertical: i32) -> Result<()> {
    unsafe {
        let event = CGEventCreateScrollWheelEvent2(
            std::ptr::null(),
            SCROLL_UNIT_PIXEL,
            2,
            vertical,
            horizontal,
            0,
        );
        let event = Event(event);
        if event.0.is_null() {
            bail!("the window server refused to create the scroll event");
        }
        if let Some(point) = point {
            CGEventSetLocation(event.0, point);
        }
        CGEventPost(HID_EVENT_TAP, event.0);
    }
    Ok(())
}

fn modifier_flag(part: &str) -> Option<u64> {
    let normalized = part.trim().to_lowercase();
    let part = normalized
        .strip_suffix("_l")
        .or_else(|| normalized.strip_suffix("_r"))
        .unwrap_or(&normalized);
    match part {
        "cmd" | "command" | "meta" | "super" => Some(FLAG_COMMAND),
        "shift" => Some(FLAG_SHIFT),
        "alt" | "opt" | "option" => Some(FLAG_OPTION),
        "ctrl" | "control" => Some(FLAG_CONTROL),
        _ => None,
    }
}

fn modifiers(spec: &str) -> u64 {
    let mut flags = 0;
    for part in spec.split(['+', ',', ' ']) {
        if let Some(flag) = modifier_flag(part) {
            flags |= flag;
        }
    }
    flags
}

/// Validate a chord used as held input during a pointer action. The pointer
/// API takes modifiers rather than printable keys, so an unsupported token is
/// refused instead of silently omitted.
pub fn click_flags(spec: &str) -> Result<u64> {
    let mut flags = 0;
    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let flag = modifier_flag(part).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown click key `{part}`; use a modifier chord such as Shift, Super, Control_L, or Super+Shift"
            )
        })?;
        flags |= flag;
    }
    Ok(flags)
}

/// A key name and the character a person would read for it, so a reply can say
/// which key was actually pressed when the caller spelled it another way.
struct Key {
    code: u16,
    name: String,
}

const NAMED: [(&str, u16, &str); 50] = [
    ("return", 36, "Return"),
    ("enter", 36, "Return"),
    ("tab", 48, "Tab"),
    ("space", 49, "Space"),
    (" ", 49, "Space"),
    ("delete", 51, "Delete"),
    ("backspace", 51, "Delete"),
    ("escape", 53, "Escape"),
    ("esc", 53, "Escape"),
    ("forward_delete", 117, "Forward_Delete"),
    ("forwarddelete", 117, "Forward_Delete"),
    ("home", 115, "Home"),
    ("end", 119, "End"),
    ("page_up", 116, "Page_Up"),
    ("pageup", 116, "Page_Up"),
    ("page_down", 121, "Page_Down"),
    ("pagedown", 121, "Page_Down"),
    ("left", 123, "Left"),
    ("arrow_left", 123, "Left"),
    ("left_arrow", 123, "Left"),
    ("right", 124, "Right"),
    ("arrow_right", 124, "Right"),
    ("right_arrow", 124, "Right"),
    ("down", 125, "Down"),
    ("arrow_down", 125, "Down"),
    ("down_arrow", 125, "Down"),
    ("up", 126, "Up"),
    ("arrow_up", 126, "Up"),
    ("up_arrow", 126, "Up"),
    ("f1", 122, "F1"),
    ("f2", 120, "F2"),
    ("f3", 99, "F3"),
    ("f4", 118, "F4"),
    ("f5", 96, "F5"),
    ("f6", 97, "F6"),
    ("f7", 98, "F7"),
    ("f8", 100, "F8"),
    ("f9", 101, "F9"),
    ("f10", 109, "F10"),
    ("f11", 103, "F11"),
    ("kp_0", 82, "KP_0"),
    ("kp_1", 83, "KP_1"),
    ("kp_2", 84, "KP_2"),
    ("kp_3", 85, "KP_3"),
    ("kp_4", 86, "KP_4"),
    ("kp_5", 87, "KP_5"),
    ("kp_6", 88, "KP_6"),
    ("kp_7", 89, "KP_7"),
    ("kp_8", 91, "KP_8"),
    ("kp_9", 92, "KP_9"),
];

/// Spelling a key as an escape is common enough in a model's tool call that the
/// runtime reads it rather than refusing: a literal `\n` is the Return key, not
/// two characters, when it appears in `key`.
fn escaped(spec: &str) -> Option<Key> {
    let lower = spec.to_lowercase();
    let name = match lower.trim() {
        "\\n" | "\\r" | "newline" | "linefeed" | "lf" | "cr" => "Return",
        "\\t" | "tab_char" => "Tab",
        _ => match lower.as_str() {
            "\n" | "\r" => "Return",
            "\t" => "Tab",
            _ => return None,
        },
    };
    Some(Key {
        code: if name == "Return" { 36 } else { 48 },
        name: name.to_string(),
    })
}

fn key(spec: &str) -> Option<Key> {
    let lower = spec.to_lowercase();
    if lower.trim() == "f12" {
        return Some(Key {
            code: 111,
            name: "F12".to_string(),
        });
    }
    if let Some((_, code, name)) = NAMED.iter().find(|(name, _, _)| *name == lower.trim()) {
        return Some(Key {
            code: *code,
            name: (*name).to_string(),
        });
    }
    if let Some(key) = escaped(&lower) {
        return Some(key);
    }
    let spec = spec.trim();
    let mut chars = spec.chars();
    let (first, rest) = (chars.next()?, chars.next());
    if rest.is_some() {
        return None;
    }
    let code = match first {
        'a' => 0,
        's' => 1,
        'd' => 2,
        'f' => 3,
        'h' => 4,
        'g' => 5,
        'z' => 6,
        'x' => 7,
        'c' => 8,
        'v' => 9,
        'b' => 11,
        'q' => 12,
        'w' => 13,
        'e' => 14,
        'r' => 15,
        'y' => 16,
        't' => 17,
        '1' => 18,
        '2' => 19,
        '3' => 20,
        '4' => 21,
        '6' => 22,
        '5' => 23,
        '=' => 24,
        '9' => 25,
        '7' => 26,
        '-' => 27,
        '8' => 28,
        '0' => 29,
        ']' => 30,
        'o' => 31,
        'u' => 32,
        '[' => 33,
        'i' => 34,
        'p' => 35,
        'l' => 37,
        'j' => 38,
        '\'' => 39,
        'k' => 40,
        ';' => 41,
        '\\' => 42,
        ',' => 43,
        '/' => 44,
        'n' => 45,
        'm' => 46,
        '.' => 47,
        '`' => 50,
        _ => return None,
    };
    Some(Key {
        code,
        name: first.to_string(),
    })
}

fn unknown_key(spec: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "unknown key `{spec}`; use a key name such as Return, Tab, Escape, Left or Page_Down, or a single character. `\\n` is a backslash and an `n`, not the Return key: write Return, and move between fields with Tab"
    )
}

fn parse(spec: &str, modifier_spec: &str) -> Result<(Key, u64)> {
    let spec = spec.trim();
    let (modifier_part, key_part) = match spec.rsplit_once('+') {
        Some((modifiers, rest)) if !rest.is_empty() => (modifiers, rest),
        _ => (modifier_spec, spec),
    };
    let key = key(key_part).ok_or_else(|| unknown_key(key_part))?;
    Ok((key, modifiers(modifier_part)))
}

/// Resolve a key spelling without posting anything, so a malformed key is
/// refused before an activation or a permission is asked for.
pub fn check(key_spec: &str, modifier_spec: &str) -> Result<String> {
    parse(key_spec, modifier_spec).map(|(key, _)| key.name)
}

/// Where a posted event goes. `Process` is ``CGEventPostToPid``: the app
/// receives it without being activated, so the person's desktop does not change
/// hands. Apps that route key handling through their own window/focus state can
/// ignore it, which is why the reply says the delivery path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Desktop,
    Process(i32),
}

fn deliver(target: Target, event: CGEventRef) {
    match target {
        Target::Desktop => unsafe { CGEventPost(HID_EVENT_TAP, event) },
        Target::Process(pid) => unsafe { CGEventPostToPid(pid, event) },
    }
}

fn press_with(
    target: Target,
    key_spec: &str,
    modifier_spec: &str,
    duration: Duration,
) -> Result<String> {
    let (key, flags) = parse(key_spec, modifier_spec)?;
    unsafe {
        let down = Event(CGEventCreateKeyboardEvent(std::ptr::null(), key.code, true));
        let up = Event(CGEventCreateKeyboardEvent(
            std::ptr::null(),
            key.code,
            false,
        ));
        if down.0.is_null() || up.0.is_null() {
            bail!("the window server refused to create the key event");
        }
        if flags != 0 {
            CGEventSetFlags(down.0, flags);
            CGEventSetFlags(up.0, flags);
        }
        deliver(target, down.0);
        pace(duration);
        deliver(target, up.0);
    }
    Ok(key.name)
}

/// Press one key on the real desktop and report the name of the key that was
/// pressed.
pub fn press(key_spec: &str, modifier_spec: &str) -> Result<String> {
    press_with(Target::Desktop, key_spec, modifier_spec, Duration::ZERO)
}

pub fn press_for(key_spec: &str, modifier_spec: &str, duration: Duration) -> Result<String> {
    press_with(Target::Desktop, key_spec, modifier_spec, duration)
}

pub fn press_in_for(
    pid: i32,
    key_spec: &str,
    modifier_spec: &str,
    duration: Duration,
) -> Result<String> {
    press_with(Target::Process(pid), key_spec, modifier_spec, duration)
}

fn type_units(target: Target, text: &str) -> Result<()> {
    let units: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let down = Event(CGEventCreateKeyboardEvent(std::ptr::null(), 0, true));
        let up = Event(CGEventCreateKeyboardEvent(std::ptr::null(), 0, false));
        if down.0.is_null() || up.0.is_null() {
            bail!("the window server refused to create the key event");
        }
        CGEventKeyboardSetUnicodeString(down.0, units.len(), units.as_ptr());
        CGEventKeyboardSetUnicodeString(up.0, units.len(), units.as_ptr());
        deliver(target, down.0);
        deliver(target, up.0);
    }
    Ok(())
}

pub struct Typed {
    pub characters: usize,
    pub line_breaks: usize,
    pub takeover: Option<&'static str>,
}

/// Type text as Unicode key events. A newline inside `text` is the Return key,
/// because an app that receives a newline inside one keyboard event usually
/// inserts nothing; the count is reported so the caller can see what landed.
pub fn type_text(text: &str) -> Result<Typed> {
    type_with(Target::Desktop, text)
}

pub fn type_text_in(pid: i32, text: &str) -> Result<Typed> {
    type_with(Target::Process(pid), text)
}

fn type_with(target: Target, text: &str) -> Result<Typed> {
    if text.is_empty() {
        bail!("text is empty");
    }
    // A person taking over matters when the real desktop is being driven; input
    // aimed at one process never takes the pointer or the focus from them.
    let watch = match target {
        Target::Desktop => Takeover::start(),
        Target::Process(_) => None,
    };
    let mut typed = Typed {
        characters: 0,
        line_breaks: 0,
        takeover: None,
    };
    for (index, segment) in text.split('\n').enumerate() {
        if index > 0 {
            press_with(target, "Return", "", Duration::ZERO)?;
            typed.line_breaks += 1;
            if let Some(kind) = watch.as_ref().and_then(Takeover::poll) {
                typed.takeover = Some(kind);
                return Ok(typed);
            }
        }
        let segment = segment.strip_suffix('\r').unwrap_or(segment);
        if segment.is_empty() {
            continue;
        }
        type_units(target, segment)?;
        typed.characters += segment.chars().count();
    }
    Ok(typed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(spec: &str) -> Option<u16> {
        key(spec).map(|key| key.code)
    }

    #[test]
    fn named_keys_map_to_hid_key_codes() {
        assert_eq!(code("Return"), Some(36));
        assert_eq!(code("return"), Some(36));
        assert_eq!(code("Escape"), Some(53));
        assert_eq!(code("Page_Down"), Some(121));
        assert_eq!(code("F12"), Some(111));
        assert_eq!(code("a"), Some(0));
        assert_eq!(code("\""), None);
        assert_eq!(code("nonsense"), None);
    }

    #[test]
    fn an_escaped_newline_is_the_return_key() {
        assert_eq!(code(r"\n"), Some(36));
        assert_eq!(code("\n"), Some(36));
        assert_eq!(code(r"\r"), Some(36));
        assert_eq!(code("newline"), Some(36));
        assert_eq!(code(r"\t"), Some(48));
        assert_eq!(code("\t"), Some(48));
    }

    #[test]
    fn a_pressed_key_is_reported_by_the_name_that_was_used() {
        assert_eq!(
            key("Return").map(|key| key.name),
            Some("Return".to_string())
        );
        assert_eq!(key(r"\n").map(|key| key.name), Some("Return".to_string()));
        assert_eq!(key("a").map(|key| key.name), Some("a".to_string()));
    }

    #[test]
    fn modifiers_accept_aliases_and_are_order_independent() {
        assert_eq!(modifiers("cmd+shift"), FLAG_COMMAND | FLAG_SHIFT);
        assert_eq!(modifiers("shift+cmd"), FLAG_COMMAND | FLAG_SHIFT);
        assert_eq!(modifiers("alt"), FLAG_OPTION);
        assert_eq!(modifiers("control"), FLAG_CONTROL);
        assert_eq!(modifiers("Super_L"), FLAG_COMMAND);
        assert_eq!(modifiers("Control_L+Shift_R"), FLAG_CONTROL | FLAG_SHIFT);
        assert_eq!(modifiers(""), 0);
    }

    #[test]
    fn click_modifiers_are_validated_and_xdotool_aliases_work() {
        assert_eq!(
            click_flags("Super_L+Shift").unwrap(),
            FLAG_COMMAND | FLAG_SHIFT
        );
        assert_eq!(click_flags("Control").unwrap(), FLAG_CONTROL);
        assert!(click_flags("a").is_err());
    }

    #[test]
    fn keypad_keys_are_available() {
        assert_eq!(code("KP_0"), Some(82));
        assert_eq!(code("kp_9"), Some(92));
    }

    #[test]
    fn an_unknown_key_is_rejected_with_a_hint_about_escapes() {
        assert!(key("nonsense").is_none());
        let error = unknown_key("nonsense").to_string();
        assert!(error.contains("unknown key"), "{error}");
        assert!(error.contains("Return"), "{error}");
        assert!(error.contains("Tab"), "{error}");
    }
}
