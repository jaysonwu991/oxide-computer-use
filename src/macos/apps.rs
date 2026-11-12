//! Running applications and their windows, via the window server.

use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{kCFAllocatorDefault, CFTypeRef};
use core_foundation_sys::bundle::{
    CFBundleCopyExecutableURL, CFBundleCreate, CFBundleGetIdentifier, CFBundleRef,
};
use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation_sys::url::{CFURLCreateFromFileSystemRepresentation, CFURLRef};
use std::path::{Path, PathBuf};
use std::process::Command;

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

#[derive(Clone, Debug)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub path: String,
    pub identity: Option<Identity>,
    pub pid: Option<i32>,
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
    let needle = app.trim().to_lowercase();
    if let Ok(pid) = app.trim().parse::<i32>() {
        if let Some(found) = apps.iter().find(|candidate| candidate.pid == pid) {
            return Some(found.clone());
        }
    }
    let normalized_path = app.trim().trim_end_matches('/');
    if normalized_path.ends_with(".app") {
        if let Some(found) = apps.iter().find(|candidate| {
            identity(candidate.pid)
                .map(|identity| identity.path == normalized_path)
                .unwrap_or(false)
        }) {
            return Some(found.clone());
        }
    }
    if app.contains('.') {
        if let Some(found) = apps.iter().find(|candidate| {
            identity(candidate.pid)
                .map(|identity| identity.bundle_id.eq_ignore_ascii_case(app.trim()))
                .unwrap_or(false)
        }) {
            return Some(found.clone());
        }
    }
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

fn scan_apps(directory: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("app") {
            out.push(path);
        } else if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            scan_apps(&path, depth - 1, out);
        }
    }
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Applications"));
    }
    dirs
}

/// Applications in the standard locations plus every running app bundle. This
/// is the launchable set behind `list_apps`; it deliberately does not traverse
/// inside a bundle once the `.app` boundary is found.
pub fn installed_apps() -> Vec<InstalledApp> {
    let running = running_apps_with(true);
    let mut paths = Vec::new();
    for directory in application_dirs() {
        scan_apps(&directory, 4, &mut paths);
    }
    for app in &running {
        if let Some(identity) = identity(app.pid) {
            paths.push(PathBuf::from(identity.path));
        }
    }
    paths.sort();
    paths.dedup();

    paths
        .into_iter()
        .filter_map(|path| {
            let path = path.to_string_lossy().to_string();
            if !bundle_has_executable(&path) {
                return None;
            }
            let bundle_id = bundle_identifier(&path);
            let display_name = Path::new(&path)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(&path)
                .to_string();
            let matching_pid = running
                .iter()
                .find(|app| {
                    identity(app.pid)
                        .map(|identity| identity.path == path)
                        .unwrap_or(false)
                })
                .map(|app| app.pid)
                .or_else(|| {
                    running
                        .iter()
                        .find(|app| app.name.eq_ignore_ascii_case(&display_name))
                        .map(|app| app.pid)
                });
            Some(InstalledApp {
                id: bundle_id.clone().unwrap_or_else(|| path.clone()),
                name: display_name,
                path: path.clone(),
                identity: bundle_id.map(|bundle_id| Identity::new(bundle_id, path)),
                pid: matching_pid,
            })
        })
        .collect()
}

pub fn resolve_installed(app: &str) -> Option<InstalledApp> {
    let installed = installed_apps();
    let needle = app.trim().to_lowercase();
    let direct = app.trim().trim_end_matches('/');
    installed
        .iter()
        .find(|candidate| {
            candidate.id.eq_ignore_ascii_case(app.trim())
                || candidate.path == direct
                || candidate.name.to_lowercase() == needle
        })
        .or_else(|| {
            installed.iter().find(|candidate| {
                candidate.name.to_lowercase().contains(&needle)
                    || candidate.id.to_lowercase().contains(&needle)
            })
        })
        .cloned()
}

pub fn launch_installed(app: &InstalledApp) -> anyhow::Result<App> {
    let status = Command::new("/usr/bin/open")
        .arg(&app.path)
        .status()
        .map_err(|error| anyhow::anyhow!("could not launch {}: {error}", app.path))?;
    if !status.success() {
        anyhow::bail!("opening {} failed with status {status}", app.path);
    }

    for _ in 0..100 {
        if let Some(running) = resolve(&app.path).or_else(|| resolve(&app.id)) {
            return Ok(running);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!(
        "{} launched, but no app with a window appeared within 10 seconds",
        app.name
    )
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
    let created = create_bundle(bundle)?;
    unsafe {
        cf::string_from(CFBundleGetIdentifier(created.raw() as CFBundleRef) as CFTypeRef)
            .filter(|identifier| !identifier.trim().is_empty())
    }
}

fn bundle_has_executable(bundle: &str) -> bool {
    let Some(created) = create_bundle(bundle) else {
        return false;
    };
    let executable = unsafe { CFBundleCopyExecutableURL(created.raw() as CFBundleRef) };
    !executable.is_null()
}

fn create_bundle(bundle: &str) -> Option<cf::Owned> {
    unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(
            kCFAllocatorDefault,
            bundle.as_ptr(),
            bundle.len() as isize,
            1,
        );
        let url = cf::Owned::from_create(url as CFTypeRef)?;
        let created = CFBundleCreate(kCFAllocatorDefault, url.raw() as CFURLRef);
        cf::Owned::from_create(created as CFTypeRef)
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
