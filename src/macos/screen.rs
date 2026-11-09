//! Screen capture through the platform `screencapture` tool.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

pub fn capture(window: Option<i64>) -> Result<Vec<u8>> {
    if !super::apps::screen_recording_allowed() {
        bail!(
            "Screen Recording permission is missing. {}",
            crate::permissions::guidance()
        );
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("oxide-reins-{}-{stamp}.png", std::process::id()));
    let mut command = Command::new("screencapture");
    command.arg("-x").arg("-o");
    if let Some(number) = window {
        command.arg(format!("-l{number}"));
    }
    command.arg(&path);
    let output = command.output().context("running screencapture")?;
    let result = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "screencapture failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }
    let bytes = result.context("reading the captured image")?;
    if bytes.is_empty() {
        bail!("screencapture produced an empty image; grant Screen Recording to the app hosting this server");
    }
    Ok(bytes)
}
