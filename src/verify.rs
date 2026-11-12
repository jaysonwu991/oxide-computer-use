//! What this machine can actually do, one native path at a time.
//!
//! The tool list is compiled and unit-tested on any machine, but several of its
//! paths only prove themselves against a granted GUI: the accessibility tree, a
//! capture, a bundle identity, the takeover tap, background key delivery. This
//! command exercises each one, prints the evidence, and exits nonzero when
//! something is missing, so "unverified" becomes one command rather than a note
//! in a document.

use anyhow::Result;
use serde_json::{json, Value};

use crate::policy::{Fallback, Policy};
use crate::runtime::Runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Passed,
    Missing,
    /// Nothing was attempted, and the reason is in the detail.
    Skipped,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Passed => "ok",
            State::Missing => "missing",
            State::Skipped => "skipped",
        }
    }

    fn mark(self) -> &'static str {
        match self {
            State::Passed => "ok",
            State::Missing => "MISSING",
            State::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub state: State,
    pub detail: String,
}

impl Check {
    fn passed(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            state: State::Passed,
            detail: detail.into(),
        }
    }

    fn missing(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            state: State::Missing,
            detail: detail.into(),
        }
    }

    fn skipped(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            state: State::Skipped,
            detail: detail.into(),
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn from_result(name: &'static str, result: Result<String>) -> Check {
        match result {
            Ok(detail) => Check::passed(name, detail),
            Err(error) => Check::missing(name, format!("{error:#}")),
        }
    }
}

pub fn run(runtime: &mut Runtime, app: Option<&str>, input: bool) -> Vec<Check> {
    let mut checks = vec![
        granted(
            "accessibility",
            crate::accessibility(false),
            "Accessibility is granted to this process",
        ),
        granted(
            "screen recording",
            crate::screen_recording(false),
            "Screen Recording is granted to this process",
        ),
    ];
    checks.extend(native(runtime, app, input));
    checks.push(policy_check());
    checks
}

fn granted(name: &'static str, ok: bool, detail: &str) -> Check {
    if ok {
        Check::passed(name, detail)
    } else {
        Check::missing(
            name,
            "not granted to the app that launched this server; grant it in System Settings → Privacy & Security, or run `oxide-computer-use doctor --prompt` there",
        )
    }
}

fn policy_check() -> Check {
    let policy = Policy::load();
    let fallback = match policy.fallback() {
        Fallback::Allow => "any app not denied",
        Fallback::Deny => "only approved apps",
    };
    let terminals = match policy.terminals() {
        Fallback::Allow => "reachable",
        Fallback::Deny => "refused",
    };
    let automation = match policy.self_automation() {
        Fallback::Allow => "the hosting app is reachable",
        Fallback::Deny => "the hosting app is refused",
    };
    let detail = format!(
        "{fallback}; terminals {terminals}; {automation}; {} allowed, {} denied",
        policy.allowed().len(),
        policy.denied().len()
    );
    match policy.error() {
        Some(error) => Check::missing("app policy", error),
        None => Check::passed("app policy", detail),
    }
}

#[cfg(target_os = "macos")]
fn native(runtime: &mut Runtime, app: Option<&str>, input: bool) -> Vec<Check> {
    use crate::macos::{apps, ax, takeover};

    let mut checks = Vec::new();
    let running = apps::running_apps();
    checks.push(if running.is_empty() {
        Check::missing(
            "window server",
            "no app with a window is running, so the rest of the native checks have nothing to read",
        )
    } else {
        Check::passed(
            "window server",
            format!("{} app(s) with a usable window", running.len()),
        )
    });

    let target = app
        .map(str::to_string)
        .or_else(|| running.first().map(|app| app.name.clone()));
    let Some(target) = target else {
        checks.push(Check::skipped(
            "accessibility tree",
            "no app to read; pass --app",
        ));
        checks.push(Check::skipped(
            "bundle identity",
            "no app to read; pass --app",
        ));
        checks.push(Check::skipped("read bound", "no app to read; pass --app"));
        checks.push(Check::skipped("screenshot", "no app to read; pass --app"));
        checks.push(Check::skipped(
            "takeover tap",
            "not attempted without an app to drive",
        ));
        checks.push(Check::skipped(
            "background input",
            "no app to type into; pass --app and --input",
        ));
        return checks;
    };

    let app_name = target.clone();
    checks.push(Check::from_result(
        "accessibility tree",
        runtime
            .get_app_state(&json!({ "app": app_name }))
            .map(|state| {
                let lines = state.text.lines().count();
                format!(
                    "read {lines} line(s) of {}'s tree, {} characters",
                    app_name,
                    state.text.chars().count()
                )
            }),
    ));

    checks.push(Check::from_result(
        "bundle identity",
        apps::resolve(&app_name)
            .and_then(|app| apps::identity(app.pid))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{app_name} has no readable `.app` bundle identity; an approval cannot be pinned to it"
                )
            })
            .map(|identity| format!("{} at {}", identity.bundle_id, identity.path)),
    ));

    checks.push(Check::from_result(
        "read bound",
        apps::resolve(&app_name)
            .ok_or_else(|| anyhow::anyhow!("{app_name} is no longer running"))
            .and_then(|app| ax::read_bound(app.pid)),
    ));

    checks.push(Check::from_result(
        "screenshot",
        runtime
            .screenshot(&json!({}))
            .map(|bytes| format!("captured {} KB PNG", bytes.len() / 1024)),
    ));

    let tap = takeover::Takeover::start();
    checks.push(match &tap {
        Some(_) => Check::passed(
            "takeover tap",
            "installed a listen-only event tap and removed it; that it receives a person's input needs the grant above",
        ),
        None => Check::missing(
            "takeover tap",
            "the window server refused a listen-only event tap, so a drag runs without takeover arbitration",
        ),
    });
    drop(tap);

    checks.push(if !input {
        Check::skipped(
            "background input",
            "pass --input to type into the app; it writes text to it",
        )
    } else {
        Check::from_result(
            "background input",
            runtime
                .type_text(&json!({
                    "app": app_name,
                    "text": "oxide-computer-use verify",
                    "background": true
                }))
                .map(|reply| reply.replace('\n', " ")),
        )
    });

    checks
}

#[cfg(not(target_os = "macos"))]
fn native(_runtime: &mut Runtime, _app: Option<&str>, _input: bool) -> Vec<Check> {
    vec![Check::skipped(
        "native backend",
        "computer use is macOS only; this build returns an error from every tool",
    )]
}

pub fn report(checks: &[Check], as_json: bool) -> Result<bool> {
    let missing = checks
        .iter()
        .filter(|check| check.state == State::Missing)
        .count();
    if as_json {
        let checks: Vec<Value> = checks
            .iter()
            .map(|check| {
                json!({
                    "name": check.name,
                    "state": check.state.name(),
                    "detail": check.detail
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "checks": checks, "missing": missing }))?
        );
    } else {
        for check in checks {
            println!(
                "{:<8} {:<20} {}",
                check.state.mark(),
                check.name,
                check.detail
            );
        }
        if missing > 0 {
            println!(
                "\n{missing} of {} checks failed. A missing permission is granted to the app that launched this server; a skipped check needs an argument.",
                checks.len()
            );
        } else {
            println!("\nAll {} checks passed.", checks.len());
        }
    }
    Ok(missing == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_missing_check_fails_the_run() {
        let checks = [
            Check::passed("accessibility", "granted"),
            Check::skipped("background input", "pass --input"),
        ];
        assert!(report(&checks, true).unwrap());

        let checks = [
            Check::passed("accessibility", "granted"),
            Check::missing("screenshot", "not granted"),
        ];
        assert!(!report(&checks, false).unwrap());
    }

    #[test]
    fn every_state_has_a_name() {
        assert_eq!(State::Passed.name(), "ok");
        assert_eq!(State::Missing.name(), "missing");
        assert_eq!(State::Skipped.name(), "skipped");
    }

    #[test]
    fn a_permission_check_says_where_to_grant_it() {
        let check = granted("accessibility", false, "granted");
        let check2 = granted("accessibility", true, "granted");
        assert_eq!(check2.state, State::Passed);
        assert_eq!(check.state, State::Missing);
        assert!(
            check.detail.contains("Privacy & Security"),
            "{}",
            check.detail
        );
    }
}
