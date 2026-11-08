use std::process::Command;

use serde_json::{json, Value};

fn app_bundle(executable: &str) -> Option<String> {
    let end = executable.find(".app/Contents/")? + 4;
    Some(executable[..end].to_string())
}

pub fn launching_app() -> Option<String> {
    let mut pid = std::process::id();
    for _ in 0..32 {
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "ppid=", "-o", "comm="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let line = String::from_utf8(output.stdout).ok()?;
        let line = line.trim();
        let (parent, executable) = line.split_once(char::is_whitespace)?;
        if let Some(bundle) = app_bundle(executable.trim()) {
            return Some(bundle);
        }
        let parent = parent.parse().ok()?;
        if parent <= 1 || parent == pid {
            return None;
        }
        pid = parent;
    }
    None
}

pub fn guidance() -> String {
    let host = launching_app()
        .map(|path| format!("the launching app `{path}`"))
        .unwrap_or_else(|| {
            "the app launching this server (Oxide in Oxide Desktop, or your terminal for CLI use)"
                .into()
        });
    format!(
        "In System Settings → Privacy & Security, grant Accessibility to {host} to read and drive apps, \
         and Screen Recording to capture screenshots. If the app is absent, use + to add it. \
         The entry may be the hosting app rather than `computer-use`. Quit and reopen that app after \
         changing Screen Recording. Run `permissions` through this same MCP connection to recheck; \
         a different terminal can have different permissions. Only the user can approve these grants."
    )
}

pub fn status() -> Value {
    json!({
        "accessibility": crate::accessibility(false),
        "screen_recording": crate::screen_recording(false),
        "launching_app": launching_app(),
        "guidance": guidance(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_outer_app_bundle_in_helper_and_harness_paths() {
        assert_eq!(
            app_bundle("/Applications/Oxide.app/Contents/Resources/harness/oxide-desktop"),
            Some("/Applications/Oxide.app".into())
        );
        assert_eq!(app_bundle("/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper"), Some("/Applications/Visual Studio Code.app".into()));
        assert_eq!(app_bundle("/Users/me/.local/bin/computer-use"), None);
    }
}
