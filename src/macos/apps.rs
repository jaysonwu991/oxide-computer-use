//! Running applications and their windows, via the window server.

use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{kCFAllocatorDefault, CFTypeRef};
use core_foundation_sys::bundle::{CFBundleCreate, CFBundleGetIdentifier, CFBundleRef};
use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation_sys::url::{CFURLCreateFromFileSystemRepresentation, CFURLRef};

use super::cf;
use crate::policy::Identity;

#[link(name = "proc")]
extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut u8, size: u32) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    static kCGWindowOwnerPID: CFTypeRef;
    static kCGWindowOwnerName: CFTypeRef;
    static kCGWindowName: CFTypeRef;
    static kCGWindowNumber: CFTypeRef;
    static kCGWindowLayer: CFTypeRef;
    static kCGWindowBounds: CFTypeRef;
}

const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const NULL_WINDOW: u32 = 0;

#[derive(Clone, Debug)]
pub struct Window {
    pub number: i64,
    pub title: String,
    pub layer: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug)]
pub struct App {
    pub pid: i32,
    pub name: String,
    pub windows: Vec<Window>,
}

/// A window a person could point at: not a menu-bar strip, not a 1-pixel
/// helper, and not one of the window-server agents that own surfaces without
/// being apps.
fn is_usable(owner: &str, window: &Window) -> bool {
    if window.layer != 0 || window.width < 120.0 || window.height < 80.0 {
        return false;
    }
    !is_helper(owner)
}

/// Window-server agents that own surfaces without being apps, such as the
/// menu-bar strips and the save-panel service. `list_apps` hides them by
/// default; `include_helpers` shows them, which is the only way to reach a
/// native file picker that is not part of its app.
fn is_helper(owner: &str) -> bool {
    matches!(
        owner,
        "Window Server"
            | "loginwindow"
            | "Dock"
            | "Control Center"
            | "Notification Center"
            | "通知中心"
            | "SystemUIServer"
            | "通用控制"
            | "自动填充"
    ) || owner.ends_with("Service")
}

pub fn screen_recording_allowed() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub fn request_screen_recording() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

fn dictionary_number(dict: CFDictionaryRef, key: CFTypeRef) -> Option<f64> {
    unsafe {
        let value = CFDictionaryGetValue(dict, key);
        cf::number(value as CFTypeRef).or_else(|| cf::integer(value as CFTypeRef).map(|n| n as f64))
    }
}

fn dictionary_string(dict: CFDictionaryRef, key: CFTypeRef) -> String {
    unsafe {
        let value = CFDictionaryGetValue(dict, key);
        cf::string_from(value as CFTypeRef).unwrap_or_default()
    }
}

fn bounds(dict: CFDictionaryRef) -> Option<(f64, f64, f64, f64)> {
    unsafe {
        let value = CFDictionaryGetValue(dict, kCGWindowBounds);
        if value.is_null() {
            return None;
        }
        let bounds = value as CFDictionaryRef;
        let x_key = cf::string("X")?;
        let y_key = cf::string("Y")?;
        let width_key = cf::string("Width")?;
        let height_key = cf::string("Height")?;
        Some((
            dictionary_number(bounds, x_key.raw())?,
            dictionary_number(bounds, y_key.raw())?,
            dictionary_number(bounds, width_key.raw())?,
            dictionary_number(bounds, height_key.raw())?,
        ))
    }
}

/// Every window the window server reports, grouped into applications.
///
/// Windows in another Space are included on purpose: accessibility actions
/// still reach those apps, and a full-screen app in its own Space must not make
/// the rest of the desktop look unavailable.
pub fn running_apps() -> Vec<App> {
    running_apps_with(false)
}

pub fn running_apps_with(include_helpers: bool) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    unsafe {
        let list = CGWindowListCopyWindowInfo(EXCLUDE_DESKTOP_ELEMENTS, NULL_WINDOW);
        let Some(owned) = cf::Owned::from_create(list as CFTypeRef) else {
            return apps;
        };
        let array = owned.raw() as CFArrayRef;
        for index in 0..CFArrayGetCount(array) {
            let entry = CFArrayGetValueAtIndex(array, index);
            if entry.is_null() {
                continue;
            }
            let entry = entry as CFDictionaryRef;
            let pid = match dictionary_number(entry, kCGWindowOwnerPID) {
                Some(pid) => pid as i32,
                None => continue,
            };
            let layer = dictionary_number(entry, kCGWindowLayer).unwrap_or(0.0) as i64;
            let name = dictionary_string(entry, kCGWindowOwnerName);
            let number = dictionary_number(entry, kCGWindowNumber).unwrap_or(0.0) as i64;
            let title = dictionary_string(entry, kCGWindowName);
            let (x, y, width, height) = bounds(entry).unwrap_or((0.0, 0.0, 0.0, 0.0));
            let window = Window {
                number,
                title,
                layer,
                x,
                y,
                width,
                height,
            };
            if !include_helpers && !is_usable(&name, &window) {
                continue;
            }
            match apps.iter_mut().find(|app| app.pid == pid) {
                Some(app) => app.windows.push(window),
                None => apps.push(App {
                    pid,
                    name: if name.is_empty() {
                        format!("pid {pid}")
                    } else {
                        name
                    },
                    windows: vec![window],
                }),
            }
        }
    }
    apps
}

/// Usable windows owned by one process.
pub fn windows_for(pid: i32) -> Vec<Window> {
    running_apps()
        .into_iter()
        .filter(|app| app.pid == pid)
        .flat_map(|app| app.windows)
        .filter(|window| window.layer == 0)
        .collect()
}

/// Resolves `app` against the running applications: exact name first, then a
/// case-insensitive substring, then the process id when the value is numeric.
pub fn resolve(app: &str) -> Option<App> {
    let apps = running_apps_with(true);
    if let Ok(pid) = app.trim().parse::<i32>() {
        if let Some(found) = apps.iter().find(|candidate| candidate.pid == pid) {
            return Some(found.clone());
        }
    }
    let needle = app.trim().to_lowercase();
    apps.iter()
        .find(|candidate| candidate.name.to_lowercase() == needle)
        .or_else(|| {
            apps.iter()
                .find(|candidate| candidate.name.to_lowercase().contains(&needle))
        })
        .cloned()
}

pub fn app_names() -> Vec<String> {
    running_apps().into_iter().map(|app| app.name).collect()
}

/// The outermost `.app` a binary belongs to, so a helper inside another app
/// reports the app a person sees.
fn bundle_root(executable: &str) -> Option<&str> {
    if let Some(index) = executable.find(".app/") {
        return Some(&executable[..index + ".app".len()]);
    }
    executable.ends_with(".app").then_some(executable)
}

fn executable_path(pid: i32) -> Option<String> {
    let mut buffer = vec![0u8; 4096];
    let written = unsafe { proc_pidpath(pid, buffer.as_mut_ptr(), buffer.len() as u32) };
    if written <= 0 {
        return None;
    }
    buffer.truncate(written as usize);
    String::from_utf8(buffer).ok()
}

fn bundle_identifier(bundle: &str) -> Option<String> {
    unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(
            kCFAllocatorDefault,
            bundle.as_ptr(),
            bundle.len() as isize,
            1,
        );
        let url = cf::Owned::from_create(url as CFTypeRef)?;
        let created = CFBundleCreate(kCFAllocatorDefault, url.raw() as CFURLRef);
        let created = cf::Owned::from_create(created as CFTypeRef)?;
        cf::string_from(CFBundleGetIdentifier(created.raw() as CFBundleRef) as CFTypeRef)
            .filter(|identifier| !identifier.trim().is_empty())
    }
}

/// The bundle identity of one running process: the identifier macOS would show,
/// plus the bundle path it was read from.
pub fn identity(pid: i32) -> Option<Identity> {
    let executable = executable_path(pid)?;
    let bundle = bundle_root(&executable)?;
    let identifier = bundle_identifier(bundle)?;
    Some(Identity::new(identifier, bundle))
}

pub fn identity_by_name(app: &str) -> Option<Identity> {
    identity(resolve(app)?.pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outermost_bundle_is_the_identity_a_person_sees() {
        assert_eq!(
            bundle_root("/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper"),
            Some("/Applications/Visual Studio Code.app")
        );
        assert_eq!(
            bundle_root("/System/Applications/TextEdit.app/Contents/MacOS/TextEdit"),
            Some("/System/Applications/TextEdit.app")
        );
        assert_eq!(bundle_root("/usr/bin/say"), None);
        assert_eq!(
            bundle_root("/Applications/TextEdit.app"),
            Some("/Applications/TextEdit.app")
        );
    }

    #[test]
    fn the_test_binary_has_no_bundle_identity() {
        assert!(identity(std::process::id() as i32).is_none());
    }

    #[test]
    fn an_unreadable_bundle_has_no_identifier() {
        assert!(bundle_identifier("/Applications/No Such App.app").is_none());
    }
}
