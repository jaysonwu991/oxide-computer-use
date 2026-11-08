//! Which apps the agent may touch.
//!
//! macOS grants Accessibility per calling process, not per app, so a server
//! with the permission can drive anything. This list keeps a session inside the
//! apps a person approved, and refuses the rest with a message that names the
//! command which changes it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

#[derive(Debug, Default, Clone)]
pub struct Policy {
    allowed: Vec<String>,
    denied: Vec<String>,
}

fn path_from(home: Option<&Path>) -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("COMPUTER_USE_CONFIG") {
        return Some(PathBuf::from(custom));
    }
    home.map(|home| {
        home.join(".config")
            .join("computer-use")
            .join("config.json")
    })
}

impl Policy {
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        path_from(home.as_deref())
    }

    pub fn load() -> Policy {
        Policy::load_from(Self::path())
    }

    fn load_from(path: Option<PathBuf>) -> Policy {
        let Some(path) = path else {
            return Policy::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Policy::default();
        };
        let value: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
        Policy {
            allowed: strings(&value, "allowed_apps"),
            denied: strings(&value, "denied_apps"),
        }
    }

    pub fn is_allowed(&self, app: &str) -> bool {
        if self.denied.iter().any(|rule| same(rule, app)) {
            return false;
        }
        self.allowed.is_empty() || self.allowed.iter().any(|rule| same(rule, app))
    }

    pub fn check(&self, app: &str) -> Result<()> {
        if self.is_allowed(app) {
            return Ok(());
        }
        if self.denied.iter().any(|rule| same(rule, app)) {
            bail!(
                "`{app}` is on the deny list; allow it with `computer-use apps allow \"{app}\"` if that is intended"
            );
        }
        bail!(
            "`{app}` is not on the allow list, which currently permits {}; add it with `computer-use apps allow \"{app}\"`",
            self.allowed.join(", ")
        )
    }

    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    pub fn denied(&self) -> &[String] {
        &self.denied
    }

    pub fn describe(&self) -> String {
        if self.allowed.is_empty() && self.denied.is_empty() {
            return "No app policy: every app with Accessibility access is reachable.".to_string();
        }
        let mut lines = Vec::new();
        lines.push(if self.allowed.is_empty() {
            "Allowed apps: all, except the deny list.".to_string()
        } else {
            format!("Allowed apps: {}", self.allowed.join(", "))
        });
        if !self.denied.is_empty() {
            lines.push(format!("Denied apps: {}", self.denied.join(", ")));
        }
        lines.join("\n")
    }

    pub fn allow(&mut self, app: &str) -> Result<()> {
        self.denied.retain(|rule| !same(rule, app));
        push_unique(&mut self.allowed, app);
        self.save()
    }

    pub fn deny(&mut self, app: &str) -> Result<()> {
        self.allowed.retain(|rule| !same(rule, app));
        push_unique(&mut self.denied, app);
        self.save()
    }

    pub fn clear(&mut self) -> Result<()> {
        self.allowed.clear();
        self.denied.clear();
        self.save()
    }

    fn save(&self) -> Result<()> {
        let path =
            Self::path().context("no config path: HOME and COMPUTER_USE_CONFIG are unset")?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let value = json!({
            "allowed_apps": self.allowed,
            "denied_apps": self.denied,
        });
        std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(&value)?))
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn same(rule: &str, app: &str) -> bool {
    rule.eq_ignore_ascii_case(app)
}

fn push_unique(list: &mut Vec<String>, app: &str) {
    if !list.iter().any(|item| same(item, app)) {
        list.push(app.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allowed: &[&str], denied: &[&str]) -> Policy {
        Policy {
            allowed: allowed.iter().map(|item| item.to_string()).collect(),
            denied: denied.iter().map(|item| item.to_string()).collect(),
        }
    }

    #[test]
    fn an_empty_policy_allows_everything() {
        let policy = policy(&[], &[]);
        assert!(policy.check("Google Chrome").is_ok());
        assert!(policy.describe().contains("No app policy"));
    }

    #[test]
    fn a_deny_list_wins_over_an_allow_list() {
        let policy = policy(&["Finder", "1Password"], &["1password"]);
        assert!(policy.check("Finder").is_ok());
        let error = policy.check("1Password").unwrap_err().to_string();
        assert!(error.contains("deny list"), "{error}");
        assert!(error.contains("apps allow"), "{error}");
    }

    #[test]
    fn a_non_empty_allow_list_denies_everything_else() {
        let policy = policy(&["Finder"], &[]);
        assert!(policy.check("finder").is_ok());
        let error = policy.check("Terminal").unwrap_err().to_string();
        assert!(error.contains("allow list"), "{error}");
        assert!(error.contains("Finder"), "{error}");
    }

    #[test]
    fn saving_and_reloading_round_trips_through_the_config_file() {
        let directory = std::env::temp_dir().join(format!(
            "oxide-cu-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let path = directory.join("config.json");
        let mut policy = Policy::default();
        policy.allowed.push("Finder".to_string());
        policy.denied.push("1Password".to_string());
        policy.save_to(&path).unwrap();
        let reloaded = Policy::load_from(Some(path));
        assert!(reloaded.check("Finder").is_ok());
        assert!(reloaded.check("1Password").is_err());
        assert_eq!(reloaded.allowed(), ["Finder"]);
        assert_eq!(reloaded.denied(), ["1Password"]);
        std::fs::remove_dir_all(&directory).ok();
    }
}
