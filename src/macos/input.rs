//! Synthetic input: pointer events, keyboard events and scrolling.

use anyhow::{bail, Result};
use core_foundation_sys::base::{CFRelease, CFTypeRef};

use super::ax::CGPoint;

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

use std::time::Duration;

const SCROLL_UNIT_LINE: u32 = 1;
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

pub fn click(point: CGPoint, button: Button) -> Result<()> {
    let (down, up, index) = match button {
        Button::Left => (LEFT_MOUSE_DOWN, LEFT_MOUSE_UP, 0),
        Button::Right => (RIGHT_MOUSE_DOWN, RIGHT_MOUSE_UP, 1),
    };
    let source = Source::hid();
    unsafe {
        post(CGEventCreateMouseEvent(source.raw(), MOUSE_MOVED, point, 0))?;
        pace(UI_SETTLED);
        let down_event = Event(CGEventCreateMouseEvent(source.raw(), down, point, index));
        if down_event.0.is_null() {
            bail!("the window server refused to create the mouse event");
        }
        CGEventSetIntegerValueField(down_event.0, MOUSE_EVENT_CLICK_STATE, 1);
        CGEventPost(HID_EVENT_TAP, down_event.0);
        pace(UI_SETTLED);
        post(CGEventCreateMouseEvent(source.raw(), up, point, index))?;
    }
    Ok(())
}

pub enum Button {
    Left,
    Right,
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
        }

        pace(UI_SETTLED);
        post(CGEventCreateMouseEvent(source.raw(), LEFT_MOUSE_UP, to, 0))?;
    }
    Ok(())
}

pub fn scroll(point: Option<CGPoint>, lines: i32) -> Result<()> {
    unsafe {
        let event =
            CGEventCreateScrollWheelEvent2(std::ptr::null(), SCROLL_UNIT_LINE, 1, lines, 0, 0);
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

fn modifiers(spec: &str) -> u64 {
    let mut flags = 0;
    for part in spec.split(['+', ',', ' ']) {
        match part.trim().to_lowercase().as_str() {
            "cmd" | "command" | "meta" => flags |= FLAG_COMMAND,
            "shift" => flags |= FLAG_SHIFT,
            "alt" | "opt" | "option" => flags |= FLAG_OPTION,
            "ctrl" | "control" => flags |= FLAG_CONTROL,
            _ => {}
        }
    }
    flags
}

fn key_code(name: &str) -> Option<u16> {
    let name = name.trim();
    let lower = name.to_lowercase();
    let code = match lower.as_str() {
        "return" | "enter" => 36,
        "tab" => 48,
        "space" | " " => 49,
        "delete" | "backspace" => 51,
        "escape" | "esc" => 53,
        "forward_delete" | "forwarddelete" => 117,
        "home" => 115,
        "end" => 119,
        "page_up" | "pageup" => 116,
        "page_down" | "pagedown" => 121,
        "left" | "arrow_left" | "left_arrow" => 123,
        "right" | "arrow_right" | "right_arrow" => 124,
        "down" | "arrow_down" | "down_arrow" => 125,
        "up" | "arrow_up" | "up_arrow" => 126,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => {
            let mut chars = name.chars();
            let (first, rest) = (chars.next()?, chars.next());
            if rest.is_some() {
                return None;
            }
            match first {
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
            }
        }
    };
    Some(code)
}

pub fn press(key: &str, modifier_spec: &str) -> Result<()> {
    let spec = key.trim();
    let (modifier_part, key_part) = match spec.rsplit_once('+') {
        Some((modifiers, rest)) if !rest.is_empty() => (modifiers, rest),
        _ => (modifier_spec, spec),
    };
    let code = key_code(key_part)
        .ok_or_else(|| anyhow::anyhow!("unknown key `{key_part}`; use a key name such as Return, Tab, Escape, Left, Page_Down, or a single character"))?;
    let flags = modifiers(modifier_part);
    unsafe {
        let down = Event(CGEventCreateKeyboardEvent(std::ptr::null(), code, true));
        let up = Event(CGEventCreateKeyboardEvent(std::ptr::null(), code, false));
        if down.0.is_null() || up.0.is_null() {
            bail!("the window server refused to create the key event");
        }
        if flags != 0 {
            CGEventSetFlags(down.0, flags);
            CGEventSetFlags(up.0, flags);
        }
        CGEventPost(HID_EVENT_TAP, down.0);
        CGEventPost(HID_EVENT_TAP, up.0);
    }
    Ok(())
}

pub fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        bail!("text is empty");
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let down = Event(CGEventCreateKeyboardEvent(std::ptr::null(), 0, true));
        let up = Event(CGEventCreateKeyboardEvent(std::ptr::null(), 0, false));
        if down.0.is_null() || up.0.is_null() {
            bail!("the window server refused to create the key event");
        }
        CGEventKeyboardSetUnicodeString(down.0, units.len(), units.as_ptr());
        CGEventKeyboardSetUnicodeString(up.0, units.len(), units.as_ptr());
        CGEventPost(HID_EVENT_TAP, down.0);
        CGEventPost(HID_EVENT_TAP, up.0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_map_to_hid_key_codes() {
        assert_eq!(key_code("Return"), Some(36));
        assert_eq!(key_code("return"), Some(36));
        assert_eq!(key_code("Escape"), Some(53));
        assert_eq!(key_code("Page_Down"), Some(121));
        assert_eq!(key_code("a"), Some(0));
        assert_eq!(key_code("\""), None);
        assert_eq!(key_code("nonsense"), None);
    }

    #[test]
    fn modifiers_accept_aliases_and_are_order_independent() {
        assert_eq!(modifiers("cmd+shift"), FLAG_COMMAND | FLAG_SHIFT);
        assert_eq!(modifiers("shift+cmd"), FLAG_COMMAND | FLAG_SHIFT);
        assert_eq!(modifiers("alt"), FLAG_OPTION);
        assert_eq!(modifiers("control"), FLAG_CONTROL);
        assert_eq!(modifiers(""), 0);
    }

    #[test]
    fn an_unknown_key_is_rejected_with_a_hint() {
        let error = press("nonsense", "").expect_err("nonsense is not a key");
        assert!(error.to_string().contains("unknown key"));
    }
}
