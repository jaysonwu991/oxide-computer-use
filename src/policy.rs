//! Which apps the agent may touch.
//!
//! macOS grants Accessibility per calling process, not per app, so a server with
//! the permission can drive anything. This policy keeps a session inside the
//! apps a person approved: an approval records the bundle identity it was given
//! for, a name whose identity changed is refused, one approval can be spent on a
//! single call, and every call re-reads the file so an approval or a revocation
//! reaches a session that is already running. Two rules sit outside the per-app
//! decisions because they are about loops rather than trust: the hosting bundle
//! and terminal emulators are refused by default.

// The enforcement half of this policy is reached from the macOS runtime; a build
// for another platform only edits the decisions.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub bundle_id: String,
    pub path: String,
}

impl Identity {
    pub fn new(bundle_id: impl Into<String>, path: impl Into<String>) -> Identity {
        Identity {
            bundle_id: bundle_id.into(),
            path: path.into(),
        }
    }
}

/// What an app with no decision of its own may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fallback {
    #[default]
    Allow,
    Deny,
}

impl Fallback {
    fn name(self) -> &'static str {
        match self {
            Fallback::Allow => "allow",
            Fallback::Deny => "deny",
        }
    }

    pub fn parse(text: &str) -> Option<Fallback> {
        match text.trim().to_lowercase().as_str() {
            "allow" | "allowed" => Some(Fallback::Allow),
            "deny" | "denied" => Some(Fallback::Deny),
            _ => None,
        }
    }
}

/// What one call may do with one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Allowed,
    /// An approval granted for one call and not spent yet.
    OneTime,
    Blocked(String),
}

impl Access {
    #[cfg(test)]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Access::Allowed | Access::OneTime)
    }

    /// The suffix `list_apps` prints beside an app.
    pub fn marker(&self) -> &'static str {
        match self {
            Access::Allowed => "",
            Access::OneTime => " [approved for one call]",
            Access::Blocked(_) => " [blocked by app policy]",
        }
    }
}

/// Bundles whose job is a shell prompt. Driving one is how an agent starts
/// another agent, which is the restriction Codex documents.
pub const TERMINAL_BUNDLES: [&str; 9] = [
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "com.mitchellh.ghostty",
    "dev.warp.Warp-Stable",
    "com.github.wez.wezterm",
    "io.alacritty",
    "net.kovidgoyal.kitty",
    "org.tabby",
    "co.zeit.hyper",
];

/// The same set by app name, for a build that cannot read a bundle identity.
pub const TERMINAL_NAMES: [&str; 9] = [
    "Terminal",
    "iTerm2",
    "Ghostty",
    "Warp",
    "WezTerm",
    "Alacritty",
    "kitty",
    "Tabby",
    "Hyper",
];

#[derive(Debug, Clone)]
struct Grant {
    app: String,
    identity: Option<Identity>,
}

#[derive(Debug, Default, Clone)]
struct Rules {
    /// `None` means the allow list decides: an app list with entries refuses
    /// everything it does not name, which is what the file meant before
    /// `app_policy` existed.
    fallback: Option<Fallback>,
    allowed: Vec<String>,
    denied: Vec<String>,
    identities: BTreeMap<String, Identity>,
    one_time: Option<Grant>,
    /// `None` means the default: the app hosting this server is refused.
    self_automation: Option<Fallback>,
    /// `None` means the default: terminals are refused.
    terminals: Option<Fallback>,
}

impl Rules {
    fn identity(&self, app: &str, live: Option<&Identity>) -> Option<&Identity> {
        self.identities
            .iter()
            .find(|(name, _)| matches(name, app, live))
            .map(|(_, identity)| identity)
    }

    fn fallback(&self) -> Fallback {
        match self.fallback {
            Some(fallback) => fallback,
            None if self.allowed.is_empty() => Fallback::Allow,
            None => Fallback::Deny,
        }
    }

    fn self_automation(&self) -> Fallback {
        self.self_automation.unwrap_or(Fallback::Deny)
    }

    fn terminals(&self) -> Fallback {
        self.terminals.unwrap_or(Fallback::Deny)
    }

    fn is_default(&self) -> bool {
        self.fallback.is_none()
            && self.allowed.is_empty()
            && self.denied.is_empty()
            && self.one_time.is_none()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Policy {
    path: Option<PathBuf>,
    rules: RefCell<Rules>,
    error: RefCell<Option<String>>,
}

fn path_from(home: Option<&Path>) -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("OXIDE_COMPUTER_USE_CONFIG") {
        return Some(PathBuf::from(custom));
    }
    home.map(|home| {
        home.join(".config")
            .join("oxide-computer-use")
            .join("config.json")
    })
}

fn same(rule: &str, app: &str) -> bool {
    rule.trim().eq_ignore_ascii_case(app.trim())
}

fn matches(rule: &str, app: &str, identity: Option<&Identity>) -> bool {
    same(rule, app)
        || identity
            .is_some_and(|identity| same(rule, &identity.bundle_id) || same(rule, &identity.path))
}

fn cleaned(app: &str) -> Result<&str> {
    let app = app.trim();
    if app.is_empty() {
        bail!("an app name is required");
    }
    Ok(app)
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn identity_from(value: &Value) -> Option<Identity> {
    let bundle_id = value.get("bundle_id").and_then(Value::as_str)?.trim();
    if bundle_id.is_empty() {
        return None;
    }
    Some(Identity::new(
        bundle_id,
        value.get("path").and_then(Value::as_str).unwrap_or(""),
    ))
}

fn rules_from(value: &Value) -> Rules {
    let identities = value
        .get("app_identities")
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(app, entry)| {
                    identity_from(entry).map(|identity| (app.trim().to_string(), identity))
                })
                .collect()
        })
        .unwrap_or_default();
    let one_time = value.get("one_time_grant").and_then(|entry| {
        let app = entry.get("app").and_then(Value::as_str)?.trim();
        if app.is_empty() {
            return None;
        }
        Some(Grant {
            app: app.to_string(),
            identity: identity_from(entry),
        })
    });
    Rules {
        fallback: value
            .get("app_policy")
            .and_then(Value::as_str)
            .and_then(Fallback::parse),
        allowed: strings(value, "allowed_apps"),
        denied: strings(value, "denied_apps"),
        identities,
        one_time,
        self_automation: value
            .get("self_automation")
            .and_then(Value::as_str)
            .and_then(Fallback::parse),
        terminals: value
            .get("terminals")
            .and_then(Value::as_str)
            .and_then(Fallback::parse),
    }
}

fn read_rules(path: &Path) -> Result<Rules> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Rules::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", path.display()));
        }
    };
    if text.trim().is_empty() {
        return Ok(Rules::default());
    }
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    if !value.is_object() {
        bail!("{} must hold a JSON object", path.display());
    }
    Ok(rules_from(&value))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = format!("{}\n", serde_json::to_string_pretty(value)?);
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
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
        let policy = Policy {
            path,
            rules: RefCell::new(Rules::default()),
            error: RefCell::new(None),
        };
        policy.reload();
        policy
    }

    /// Re-read the file. A file that does not parse keeps refusing every app
    /// instead of falling back to "unlisted, so allowed".
    pub fn reload(&self) {
        let Some(path) = &self.path else {
            return;
        };
        match read_rules(path) {
            Ok(rules) => {
                *self.rules.borrow_mut() = rules;
                *self.error.borrow_mut() = None;
            }
            Err(error) => {
                *self.error.borrow_mut() = Some(format!(
                    "{error:#}; every app is refused until the file parses. Fix it, or run `oxide-computer-use apps reset`"
                ));
            }
        }
    }

    pub fn refresh(&self) -> Result<()> {
        self.reload();
        match self.error.borrow().as_ref() {
            Some(message) => bail!("{message}"),
            None => Ok(()),
        }
    }

    pub fn fallback(&self) -> Fallback {
        self.rules.borrow().fallback()
    }

    pub fn allowed(&self) -> Vec<String> {
        self.rules.borrow().allowed.clone()
    }

    pub fn denied(&self) -> Vec<String> {
        self.rules.borrow().denied.clone()
    }

    pub fn identities(&self) -> BTreeMap<String, String> {
        self.rules
            .borrow()
            .identities
            .iter()
            .map(|(app, identity)| (app.clone(), identity.bundle_id.clone()))
            .collect()
    }

    pub fn self_automation(&self) -> Fallback {
        self.rules.borrow().self_automation()
    }

    pub fn terminals(&self) -> Fallback {
        self.rules.borrow().terminals()
    }

    /// A bundle identity decides when it is known; the name list is the fallback
    /// for a build that cannot read one, and never overrides a known identity.
    pub fn is_terminal(&self, app: &str, identity: Option<&Identity>) -> bool {
        match identity {
            Some(identity) => TERMINAL_BUNDLES
                .iter()
                .any(|bundle| bundle.eq_ignore_ascii_case(identity.bundle_id.trim())),
            None => TERMINAL_NAMES
                .iter()
                .any(|name| name.eq_ignore_ascii_case(app.trim())),
        }
    }

    pub fn one_time(&self) -> Option<String> {
        self.rules
            .borrow()
            .one_time
            .as_ref()
            .map(|grant| grant.app.clone())
    }

    pub fn error(&self) -> Option<String> {
        self.error.borrow().clone()
    }

    /// Whether a decision for this app is pinned to a bundle identity, which is
    /// what makes reading the running app's identity worth a call.
    pub fn needs_identity(&self, app: &str) -> bool {
        let rules = self.rules.borrow();
        rules.identities.keys().any(|name| same(name, app))
            || rules.one_time.as_ref().is_some_and(|grant| {
                same(&grant.app, app) || grant.app.contains('.') || grant.app.contains('/')
            })
            || rules
                .allowed
                .iter()
                .chain(rules.denied.iter())
                .any(|rule| rule.contains('.') || rule.contains('/'))
    }

    pub fn access(&self, app: &str, identity: Option<&Identity>) -> Access {
        let rules = self.rules.borrow();
        if rules.denied.iter().any(|rule| matches(rule, app, identity)) {
            return Access::Blocked(format!(
                "`{app}` is on the deny list; lift it with `oxide-computer-use apps revoke \"{app}\"` if that is not what you want"
            ));
        }
        if rules
            .allowed
            .iter()
            .any(|rule| matches(rule, app, identity))
        {
            let Some(recorded) = rules.identity(app, identity) else {
                return Access::Allowed;
            };
            return match identity {
                Some(live) if live.bundle_id == recorded.bundle_id => Access::Allowed,
                Some(live) => Access::Blocked(format!(
                    "`{app}` was approved as {} at {}, but this process is {} at {}; a different app now answers to that name. Re-approve it with `oxide-computer-use apps allow \"{app}\"` if that is intended",
                    recorded.bundle_id, recorded.path, live.bundle_id, live.path
                )),
                None => Access::Blocked(format!(
                    "`{app}` was approved as {}, but this runtime cannot read the running app's bundle identifier, so it does not trust the name alone. Re-approve it with `oxide-computer-use apps allow \"{app}\"`",
                    recorded.bundle_id
                )),
            };
        }
        if let Some(grant) = &rules.one_time {
            if matches(&grant.app, app, identity) {
                return match (&grant.identity, identity) {
                    (Some(recorded), Some(live)) if recorded.bundle_id != live.bundle_id => {
                        Access::Blocked(format!(
                            "the one-time approval for `{app}` names {}, not the running {} at {}; approve it again with `oxide-computer-use apps grant-once \"{app}\"`",
                            recorded.bundle_id, live.bundle_id, live.path
                        ))
                    }
                    _ => Access::OneTime,
                };
            }
        }
        match rules.fallback() {
            Fallback::Allow => Access::Allowed,
            Fallback::Deny => Access::Blocked(format!(
                "`{app}` is not approved. Approve one call with `oxide-computer-use apps grant-once \"{app}\"`, or every call from now on with `oxide-computer-use apps allow \"{app}\"`"
            )),
        }
    }

    /// Check a decision without spending a one-time approval. The launch path
    /// uses this before starting an app, then performs the normal spending
    /// authorization after the process and its window exist.
    pub fn check(&self, app: &str, identity: Option<&Identity>) -> Result<Access> {
        self.refresh()?;
        Ok(self.access(app, identity))
    }

    /// The one gate every tool goes through, after `app` has been resolved.
    ///
    /// The identity is read on demand: whether it is worth a system call is a
    /// decision that depends on the freshly reloaded file, not on the copy the
    /// process started with.
    pub fn authorize<F>(&self, app: &str, identity: F) -> Result<()>
    where
        F: FnOnce() -> Option<Identity>,
    {
        self.refresh()?;
        let identity = self.needs_identity(app).then(identity).flatten();
        match self.access(app, identity.as_ref()) {
            Access::Allowed => Ok(()),
            Access::OneTime => {
                self.rules.borrow_mut().one_time = None;
                // A spent approval that could not be written back still applies
                // to this call: refusing it would be the worse failure.
                let _ = self.save();
                Ok(())
            }
            Access::Blocked(message) => bail!("{message}"),
        }
    }

    pub fn allow(&mut self, app: &str, identity: Option<Identity>) -> Result<()> {
        let app = cleaned(app)?.to_string();
        {
            let rules = self.rules.get_mut();
            rules.denied.retain(|rule| !same(rule, &app));
            if !rules.allowed.iter().any(|rule| same(rule, &app)) {
                rules.allowed.push(app.clone());
            }
            if let Some(identity) = identity {
                rules.identities.retain(|name, _| !same(name, &app));
                rules.identities.insert(app.clone(), identity);
            }
        }
        self.save()
    }

    pub fn deny(&mut self, app: &str) -> Result<()> {
        let app = cleaned(app)?.to_string();
        {
            let rules = self.rules.get_mut();
            rules.allowed.retain(|rule| !same(rule, &app));
            rules.identities.retain(|name, _| !same(name, &app));
            if rules
                .one_time
                .as_ref()
                .is_some_and(|grant| same(&grant.app, &app))
            {
                rules.one_time = None;
            }
            if !rules.denied.iter().any(|rule| same(rule, &app)) {
                rules.denied.push(app);
            }
        }
        self.save()
    }

    /// Withdraw every decision about one app, including a deny.
    pub fn revoke(&mut self, app: &str) -> Result<()> {
        let app = cleaned(app)?;
        {
            let rules = self.rules.get_mut();
            rules.allowed.retain(|rule| !same(rule, app));
            rules.denied.retain(|rule| !same(rule, app));
            rules.identities.retain(|name, _| !same(name, app));
            if rules
                .one_time
                .as_ref()
                .is_some_and(|grant| same(&grant.app, app))
            {
                rules.one_time = None;
            }
        }
        self.save()
    }

    /// Approve exactly one call.
    pub fn grant_once(&mut self, app: &str, identity: Option<Identity>) -> Result<()> {
        let app = cleaned(app)?.to_string();
        self.rules.get_mut().one_time = Some(Grant { app, identity });
        self.save()
    }

    pub fn set_self_automation(&mut self, fallback: Fallback) -> Result<()> {
        self.rules.get_mut().self_automation = Some(fallback);
        self.save()
    }

    pub fn set_terminals(&mut self, fallback: Fallback) -> Result<()> {
        self.rules.get_mut().terminals = Some(fallback);
        self.save()
    }

    pub fn set_fallback(&mut self, fallback: Fallback) -> Result<()> {
        self.rules.get_mut().fallback = Some(fallback);
        self.save()
    }

    pub fn clear(&mut self) -> Result<()> {
        *self.rules.get_mut() = Rules::default();
        self.save()
    }

    fn to_json(&self) -> Value {
        let rules = self.rules.borrow();
        let identities: serde_json::Map<String, Value> = rules
            .identities
            .iter()
            .map(|(app, identity)| {
                (
                    app.clone(),
                    json!({ "bundle_id": identity.bundle_id, "path": identity.path }),
                )
            })
            .collect();
        let mut value = json!({
            "allowed_apps": rules.allowed,
            "denied_apps": rules.denied,
            "app_identities": Value::Object(identities),
        });
        if let Some(fallback) = rules.fallback {
            value["app_policy"] = json!(fallback.name());
        }
        if let Some(fallback) = rules.self_automation {
            value["self_automation"] = json!(fallback.name());
        }
        if let Some(fallback) = rules.terminals {
            value["terminals"] = json!(fallback.name());
        }
        if let Some(grant) = &rules.one_time {
            let mut entry = json!({ "app": grant.app });
            if let Some(identity) = &grant.identity {
                entry["bundle_id"] = json!(identity.bundle_id);
                entry["path"] = json!(identity.path);
            }
            value["one_time_grant"] = entry;
        }
        value
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            bail!(
                "no config path; set HOME, or OXIDE_COMPUTER_USE_CONFIG, to store an app decision"
            );
        };
        write_json(path, &self.to_json())
    }

    pub fn describe(&self) -> String {
        let mut out = self.describe_rules();
        if let Some(error) = self.error.borrow().as_ref() {
            out.push_str(&format!("\nUnreadable policy: {error}"));
        }
        out
    }

    fn describe_rules(&self) -> String {
        let rules = self.rules.borrow();
        let automation = match rules.self_automation() {
            Fallback::Deny => {
                "\nSelf-automation: refused for the app hosting this server (`apps self-automation allow` changes it)."
            }
            Fallback::Allow => "\nSelf-automation: the hosting app is reachable too.",
        };
        let terminals = match rules.terminals() {
            Fallback::Deny => "\nTerminals: refused (`apps terminals allow` changes it).",
            Fallback::Allow => "\nTerminals: reachable.",
        };
        if rules.is_default() {
            return format!(
                "No app policy: every app with Accessibility access is reachable.{automation}{terminals}"
            );
        }
        let mut out = match rules.fallback() {
            Fallback::Allow => {
                "App policy: every app except the deny list is reachable (default: allow)."
                    .to_string()
            }
            Fallback::Deny => {
                "App policy: only approved apps are reachable (default: deny).".to_string()
            }
        };
        let describe = |apps: &[String]| -> String {
            apps.iter()
                .map(|app| match rules.identity(app, None) {
                    Some(identity) => format!("{app} ({})", identity.bundle_id),
                    None => app.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "\nAllowed: {}",
            match rules.allowed.len() {
                0 => "none".to_string(),
                _ => describe(&rules.allowed),
            }
        ));
        if !rules.denied.is_empty() {
            out.push_str(&format!("\nDenied: {}", describe(&rules.denied)));
        }
        out.push_str(automation);
        out.push_str(terminals);
        if let Some(grant) = &rules.one_time {
            out.push_str(&format!(
                "\nOne-time approval: {} ({})",
                grant.app,
                match &grant.identity {
                    Some(identity) => identity.bundle_id.clone(),
                    None => "by name".to_string(),
                }
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "oxide-computer-use-policy-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_nanos())
                    .unwrap_or(0)
            ))
            .join("config.json")
    }

    fn policy_with(path: &Path, rules: Rules) -> Policy {
        let policy = Policy::load_from(Some(path.to_path_buf()));
        *policy.rules.borrow_mut() = rules;
        policy
    }

    fn finder() -> Identity {
        Identity::new(
            "com.apple.finder",
            "/System/Library/CoreServices/Finder.app",
        )
    }

    #[test]
    fn an_empty_policy_allows_everything() {
        let policy = Policy::default();
        assert!(policy.access("Google Chrome", None).is_allowed());
        assert!(policy.authorize("Google Chrome", || None).is_ok());
        assert!(policy.describe().contains("No app policy"));
    }

    #[test]
    fn a_deny_list_wins_over_an_allow_list() {
        let path = scratch("deny");
        let policy = policy_with(
            &path,
            Rules {
                allowed: vec!["Finder".into(), "1Password".into()],
                denied: vec!["1password".into()],
                ..Rules::default()
            },
        );
        assert!(policy.access("finder", None).is_allowed());
        let error = policy.access("1Password", None);
        let Access::Blocked(message) = error else {
            panic!("the deny list has to win");
        };
        assert!(message.contains("deny list"), "{message}");
        assert!(message.contains("apps revoke"), "{message}");
    }

    #[test]
    fn a_non_empty_allow_list_denies_everything_else() {
        let path = scratch("allow");
        let policy = policy_with(
            &path,
            Rules {
                allowed: vec!["Finder".into()],
                ..Rules::default()
            },
        );
        assert!(policy.access("Finder", None).is_allowed());
        let Access::Blocked(message) = policy.access("Terminal", None) else {
            panic!("the allow list has to win");
        };
        assert!(message.contains("is not approved"), "{message}");
        assert!(message.contains("apps allow"), "{message}");
    }

    #[test]
    fn an_approval_pinned_to_an_identity_refuses_a_different_bundle() {
        let path = scratch("identity");
        let policy = policy_with(
            &path,
            Rules {
                allowed: vec!["Finder".into()],
                identities: BTreeMap::from([("Finder".to_string(), finder())]),
                ..Rules::default()
            },
        );
        assert!(policy.access("Finder", Some(&finder())).is_allowed());
        let impostor = Identity::new("com.example.finder", "/Applications/Finder.app");
        let Access::Blocked(message) = policy.access("Finder", Some(&impostor)) else {
            panic!("a changed identity has to be refused");
        };
        assert!(message.contains("com.apple.finder"), "{message}");
        assert!(message.contains("com.example.finder"), "{message}");
        let Access::Blocked(message) = policy.access("Finder", None) else {
            panic!("an unreadable identity has to be refused too");
        };
        assert!(message.contains("bundle identifier"), "{message}");
    }

    #[test]
    fn policy_rules_match_bundle_ids_and_paths_as_well_as_names() {
        let path = scratch("identity-address");
        let policy = policy_with(
            &path,
            Rules {
                allowed: vec!["com.apple.finder".into()],
                ..Rules::default()
            },
        );
        assert!(policy.access("Finder", Some(&finder())).is_allowed());

        let policy = policy_with(
            &path,
            Rules {
                denied: vec!["/System/Library/CoreServices/Finder.app".into()],
                ..Rules::default()
            },
        );
        let Access::Blocked(message) = policy.access("Finder", Some(&finder())) else {
            panic!("a path rule has to match the running bundle path");
        };
        assert!(message.contains("deny list"), "{message}");
    }

    #[test]
    fn a_one_time_approval_is_spent_by_the_call_that_uses_it() {
        let path = scratch("once");
        let mut policy = policy_with(&path, Rules::default());
        policy.set_fallback(Fallback::Deny).unwrap();
        policy.grant_once("TextEdit", Some(finder())).unwrap();
        assert_eq!(policy.access("TextEdit", Some(&finder())), Access::OneTime);
        policy.authorize("TextEdit", || Some(finder())).unwrap();
        assert!(!policy.access("TextEdit", Some(&finder())).is_allowed());
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(!stored.contains("one_time_grant"), "{stored}");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_one_time_approval_for_another_bundle_is_refused_and_kept() {
        let path = scratch("once-identity");
        let mut policy = policy_with(&path, Rules::default());
        policy.set_fallback(Fallback::Deny).unwrap();
        policy.grant_once("TextEdit", Some(finder())).unwrap();
        let impostor = Identity::new("com.example.textedit", "/Applications/TextEdit.app");
        assert!(!policy.access("TextEdit", Some(&impostor)).is_allowed());
        assert_eq!(policy.one_time().as_deref(), Some("TextEdit"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_default_deny_policy_names_both_approval_commands() {
        let path = scratch("fallback");
        let policy = policy_with(
            &path,
            Rules {
                fallback: Some(Fallback::Deny),
                allowed: vec!["Finder".into()],
                ..Rules::default()
            },
        );
        assert!(policy.access("Finder", None).is_allowed());
        let Access::Blocked(message) = policy.access("Terminal", None) else {
            panic!("an unapproved app has to be refused");
        };
        assert!(message.contains("grant-once"), "{message}");
        assert!(message.contains("apps allow"), "{message}");
        assert!(policy.describe().contains("default: deny"));
    }

    #[test]
    fn revoking_an_app_lifts_a_deny_and_forgets_its_identity() {
        let path = scratch("revoke");
        let mut policy = policy_with(
            &path,
            Rules {
                allowed: vec!["Finder".into()],
                denied: vec!["1Password".into()],
                identities: BTreeMap::from([("Finder".to_string(), finder())]),
                one_time: Some(Grant {
                    app: "Finder".into(),
                    identity: Some(finder()),
                }),
                ..Rules::default()
            },
        );
        policy.revoke("finder").unwrap();
        policy.revoke("1password").unwrap();
        assert!(policy.allowed().is_empty());
        assert!(policy.denied().is_empty());
        assert!(policy.identities().is_empty());
        assert!(policy.one_time().is_none());
        assert!(policy.access("1Password", None).is_allowed());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn terminals_are_refused_by_default_and_by_bundle_or_name() {
        let policy = Policy::default();
        assert_eq!(policy.terminals(), Fallback::Deny);
        let term = Identity::new(
            "com.apple.Terminal",
            "/System/Applications/Utilities/Terminal.app",
        );
        assert!(policy.is_terminal("Terminal", Some(&term)));
        assert!(policy.is_terminal(
            "Terminal",
            Some(&Identity::new(
                "com.googlecode.iterm2",
                "/Applications/iTerm.app"
            ))
        ));
        assert!(
            policy.is_terminal("Terminal", None),
            "the name is the fallback"
        );
        assert!(!policy.is_terminal("TextEdit", None));
        assert!(
            !policy.is_terminal(
                "Terminal",
                Some(&Identity::new(
                    "com.example.terminal",
                    "/Applications/Terminal.app"
                ))
            ),
            "a known identity decides, so a lookalike name is not a terminal"
        );
        assert!(policy.describe().contains("Terminals: refused"));
    }

    #[test]
    fn a_deny_forgets_an_identity_whatever_its_case() {
        let path = scratch("deny-identity");
        let mut policy = policy_with(
            &path,
            Rules {
                allowed: vec!["Finder".into()],
                identities: BTreeMap::from([("Finder".to_string(), finder())]),
                ..Rules::default()
            },
        );
        policy.deny("finder").unwrap();
        assert!(policy.identities().is_empty());
        assert_eq!(policy.denied(), ["finder"]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn saving_and_reloading_round_trips_every_decision() {
        let path = scratch("round-trip");
        let mut policy = Policy::load_from(Some(path.clone()));
        policy.allow("Finder", Some(finder())).unwrap();
        policy.deny("1Password").unwrap();
        policy.set_fallback(Fallback::Deny).unwrap();
        policy.grant_once("TextEdit", Some(finder())).unwrap();

        let reloaded = Policy::load_from(Some(path.clone()));
        assert_eq!(reloaded.allowed(), ["Finder"]);
        assert_eq!(reloaded.denied(), ["1Password"]);
        assert_eq!(reloaded.fallback(), Fallback::Deny);
        assert_eq!(
            reloaded.identities().get("Finder").map(String::as_str),
            Some("com.apple.finder")
        );
        assert_eq!(reloaded.one_time().as_deref(), Some("TextEdit"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_decision_written_by_another_process_reaches_a_running_policy() {
        let path = scratch("reload");
        let mut writer = Policy::load_from(Some(path.clone()));
        writer.set_fallback(Fallback::Deny).unwrap();
        writer.allow("Finder", Some(finder())).unwrap();

        let reader = Policy::load_from(Some(path.clone()));
        assert!(reader.authorize("Finder", || Some(finder())).is_ok());
        assert!(reader.authorize("Terminal", || None).is_err());

        writer.grant_once("Terminal", None).unwrap();
        assert!(reader.authorize("Terminal", || None).is_ok());
        assert!(reader.authorize("Terminal", || None).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn an_unreadable_policy_refuses_instead_of_allowing_everything() {
        let path = scratch("broken");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let policy = Policy::load_from(Some(path.clone()));
        let error = policy.authorize("Finder", || None).unwrap_err().to_string();
        assert!(error.contains("not valid JSON"), "{error}");
        assert!(error.contains("apps reset"), "{error}");
        assert!(policy.describe().contains("Unreadable policy"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_missing_file_is_an_empty_policy() {
        let path = scratch("missing");
        let policy = Policy::load_from(Some(path));
        assert!(policy.error().is_none());
        assert!(policy.access("Finder", None).is_allowed());
    }

    #[test]
    fn the_fallback_spellings_are_read_leniently() {
        assert_eq!(Fallback::parse(" Deny "), Some(Fallback::Deny));
        assert_eq!(Fallback::parse("allow"), Some(Fallback::Allow));
        assert_eq!(Fallback::parse("maybe"), None);
    }
}
