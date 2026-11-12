//! A listen-only event tap that tells a person's input apart from our own.
//!
//! Every event this runtime posts is stamped with its own process id, and a
//! person's hardware input is not, so one comparison between the tap callback
//! and our pid separates "the person is using this Mac" from "this is the event
//! we just posted". A paced gesture pumps the tap between steps and yields
//! instead of fighting the pointer. A tap that cannot be installed leaves the
//! gesture unarbitrated rather than failing it, and whether an installed tap
//! receives a person's events is only proven on a granted machine:
//! `oxide-computer-use verify` reports the install and whether the grant exists.

use std::cell::Cell;
use std::ffi::c_void;

use core_foundation_sys::base::CFRelease;
use core_foundation_sys::mach_port::{
    CFMachPortCreateRunLoopSource, CFMachPortInvalidate, CFMachPortRef,
};
use core_foundation_sys::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef,
    CFRunLoopRemoveSource, CFRunLoopRunInMode, CFRunLoopSourceRef,
};

type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const SOURCE_PROCESS_ID: u32 = 41;

const MOUSE: u32 = 1;
const KEYBOARD: u32 = 2;
const SCROLL: u32 = 4;

const INTERESTING: [u32; 14] = [1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 22, 25, 26, 27];

fn mask() -> u64 {
    INTERESTING.iter().fold(0, |mask, kind| mask | 1 << kind)
}

fn kind(event_type: u32) -> u32 {
    match event_type {
        10..=12 => KEYBOARD,
        22 => SCROLL,
        _ => MOUSE,
    }
}

fn describe(seen: u32) -> Option<&'static str> {
    if seen & KEYBOARD != 0 {
        return Some("a keystroke");
    }
    if seen & MOUSE != 0 {
        return Some("pointer movement");
    }
    if seen & SCROLL != 0 {
        return Some("a scroll");
    }
    None
}

struct Watch {
    ours: i32,
    seen: Cell<u32>,
}

/// A posted event carries this process as its source; anything else came from
/// the person, another tool, or the system.
fn is_person(source: i64, ours: i32) -> bool {
    source != ours as i64
}

extern "C" fn observe(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef {
    if user.is_null() || event.is_null() {
        return event;
    }
    let watch = unsafe { &*(user as *const Watch) };
    let source = unsafe { CGEventGetIntegerValueField(event, SOURCE_PROCESS_ID) };
    if is_person(source, watch.ours) {
        watch.seen.set(watch.seen.get() | kind(event_type));
    }
    event
}

pub struct Takeover {
    tap: CFMachPortRef,
    source: CFRunLoopSourceRef,
    run_loop: CFRunLoopRef,
    watch: *mut Watch,
}

impl Takeover {
    pub fn start() -> Option<Takeover> {
        let watch = Box::into_raw(Box::new(Watch {
            ours: std::process::id() as i32,
            seen: Cell::new(0),
        }));
        let tap = unsafe {
            CGEventTapCreate(
                SESSION_TAP,
                HEAD_INSERT,
                LISTEN_ONLY,
                mask(),
                observe,
                watch as *mut c_void,
            )
        };
        if tap.is_null() {
            unsafe { drop(Box::from_raw(watch)) };
            return None;
        }
        let source = unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };
        if source.is_null() {
            unsafe {
                CFMachPortInvalidate(tap);
                CFRelease(tap as *const c_void);
                drop(Box::from_raw(watch));
            }
            return None;
        }
        let run_loop = unsafe { CFRunLoopGetCurrent() };
        unsafe { CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode) };
        Some(Takeover {
            tap,
            source,
            run_loop,
            watch,
        })
    }

    /// Dispatch what the tap has collected and name the first kind of input a
    /// person produced since the gesture started.
    pub fn poll(&self) -> Option<&'static str> {
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.02, 0) };
        let seen = unsafe { (*self.watch).seen.get() };
        describe(seen)
    }
}

impl Drop for Takeover {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopDefaultMode);
            CFRelease(self.source as *const c_void);
            CFMachPortInvalidate(self.tap);
            CFRelease(self.tap as *const c_void);
            drop(Box::from_raw(self.watch));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_person_input_has_a_name() {
        assert_eq!(describe(MOUSE), Some("pointer movement"));
        assert_eq!(describe(KEYBOARD), Some("a keystroke"));
        assert_eq!(describe(SCROLL), Some("a scroll"));
        assert_eq!(describe(0), None);
        assert_eq!(describe(MOUSE | KEYBOARD), Some("a keystroke"));
    }

    #[test]
    fn the_mask_covers_keys_pointer_buttons_and_scrolling() {
        let mask = mask();
        for kind in [1, 2, 5, 6, 10, 12, 22] {
            assert_eq!(mask & (1 << kind), 1 << kind, "event type {kind}");
        }
    }

    #[test]
    fn only_another_source_counts_as_the_person() {
        assert!(!is_person(4242, 4242));
        assert!(is_person(0, 4242));
        assert!(is_person(17, 4242));
    }
}
