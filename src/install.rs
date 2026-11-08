//! Registering the server with the MCP clients people actually run.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use toml_edit::{value, Array, DocumentMut, Item, Table};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    Oxide,
    Claude,
    Codex,
    Cursor,
    Desktop,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Json,
    Toml,
}

impl Client {
    pub fn parse(text: &str) -> Result<Client> {
        match text.trim().to_lowercase().as_str() {
            "oxide" => Ok(Client::Oxide),
            "claude" | "claude-code" => Ok(Client::Claude),
            "codex" => Ok(Client::Codex),
            "cursor" => Ok(Client::Cursor),
            "desktop" | "claude-desktop" => Ok(Client::Desktop),
            other => bail!(
                "unknown client `{other}`; choose oxide, claude, codex, cursor or desktop (or use --print)"
            ),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Client::Oxide => "Oxide",
            Client::Claude => "Claude Code",
            Client::Codex => "Codex CLI",
            Client::Cursor => "Cursor",
            Client::Desktop => "Claude Desktop",
        }
    }

    fn format(self) -> Format {
        match self {
            Client::Codex => Format::Toml,
            _ => Format::Json,
        }
    }

    /// Whether the client has a project-scoped config file of this shape.
    pub fn supports_project(self) -> bool {
        !matches!(self, Client::Desktop)
    }

    fn target(self, project: bool) -> Result<PathBuf> {
        let home = home()?;
        let path = match (self, project) {
            (Client::Oxide, false) => home.join(".oxide").join("mcp.json"),
            (Client::Oxide, true) => std::env::current_dir()?.join(".oxide").join("mcp.json"),
            (Client::Claude, false) => home.join(".claude.json"),
            (Client::Claude, true) => std::env::current_dir()?.join(".mcp.json"),
            (Client::Codex, false) => home.join(".codex").join("config.toml"),
            (Client::Codex, true) => std::env::current_dir()?.join(".codex").join("config.toml"),
            (Client::Cursor, false) => home.join(".cursor").join("mcp.json"),
            (Client::Cursor, true) => std::env::current_dir()?.join(".cursor").join("mcp.json"),
            (Client::Desktop, false) => home
                .join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json"),
            (Client::Desktop, true) => {
                bail!("Claude Desktop has no project-scoped config; drop --project")
            }
        };
        Ok(path)
    }
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn entry(executable: &Path) -> Value {
    json!({ "command": executable.to_string_lossy(), "args": ["mcp"] })
}

pub fn snippet(client: Client, executable: &Path, name: &str) -> String {
    match client.format() {
        Format::Json => serde_json::to_string_pretty(&json!({
            "mcpServers": { name: entry(executable) }
        }))
        .unwrap_or_default(),
        Format::Toml => format!(
            "[mcp_servers.{name}]\ncommand = {}\nargs = [\"mcp\"]\n",
            toml_string(&executable.to_string_lossy())
        ),
    }
}

fn toml_string(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The name a client gets when `--name` is not given. A client that ships its
/// own Computer Use already answers to `computer-use`: Claude Code skips a
/// server registered under that name, and Codex's bundled Computer Use plugin
/// owns it, so both clients get a suffixed name by default.
pub fn default_name(client: Client) -> &'static str {
    match client {
        Client::Claude | Client::Codex => "computer-use-local",
        _ => "computer-use",
    }
}

/// The name a client has already given to something else. Registering over one
/// of these would shadow a built-in server or be skipped by the client, so
/// `--name` is refused instead.
fn reserved_name(client: Client, name: &str) -> Option<&'static str> {
    match (client, name) {
        (Client::Claude, "computer-use") => {
            Some("Claude Code reserves it and skips a server registered under that name")
        }
        (Client::Codex, "computer-use" | "computer_use") => Some(
            "Codex's bundled Computer Use plugin owns that name and the `computer_use` namespace",
        ),
        (Client::Codex, "node_repl" | "cua_repl" | "codex_apps") => {
            Some("Codex's own plugins point their MCP servers at that name")
        }
        _ => None,
    }
}

pub fn install(client: Client, project: bool, name: Option<&str>, print: bool) -> Result<()> {
    let name = match name {
        Some(name) => {
            validate_name(client, name)?;
            name.to_string()
        }
        None => default_name(client).to_string(),
    };
    let executable = std::env::current_exe().context("resolving this executable")?;
    let executable = executable.canonicalize().unwrap_or(executable);

    if print {
        println!("{}", snippet(client, &executable, &name));
        return Ok(());
    }
    if project && !client.supports_project() {
        bail!(
            "{} has no project-scoped config; drop --project",
            client.label()
        );
    }

    let path = client.target(project)?;
    match client.format() {
        Format::Json => write_json(&path, &name, &executable)?,
        Format::Toml => write_toml(&path, &name, &executable)?,
    }
    println!(
        "Registered `{name}` for {} in {}",
        client.label(),
        path.display()
    );
    match client {
        Client::Oxide => println!("Run `oxide mcp list` to confirm it is visible."),
        Client::Claude => println!(
            "Run `claude mcp list` to confirm it is visible. Claude Code reserves the name `computer-use`, so this client registers `{name}` by default; --name picks another."
        ),
        Client::Codex => println!(
            "Run `codex mcp list` to confirm it is visible. Codex ships Computer Use as a bundled plugin, so this client registers `{name}` beside it rather than over it; restart Codex if it is already running."
        ),
        Client::Cursor => println!("Restart Cursor, then check Settings → MCP for `{name}`."),
        Client::Desktop => println!("Restart Claude Desktop to load `{name}`."),
    }
    Ok(())
}

fn validate_name(client: Client, name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("the server name cannot be empty");
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("`{name}` may only contain letters, digits, `-` and `_`, so every client accepts it");
    }
    if let Some(reason) = reserved_name(client, name) {
        bail!(
            "`{name}` is reserved: {reason}; pass --name {}, or omit --name and this client uses that default",
            default_name(client)
        );
    }
    Ok(())
}

fn write_json(path: &Path, name: &str, executable: &Path) -> Result<()> {
    let mut root: Value = if path.exists() {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text).with_context(|| {
                format!(
                    "parsing {}: fix the JSON, then rerun, or register with another client",
                    path.display()
                )
            })?
        }
    } else {
        json!({})
    };
    if !root.is_object() {
        root = json!({});
    }
    let object = root.as_object_mut().expect("object");
    if !object.get("mcpServers").is_some_and(Value::is_object) {
        object.insert("mcpServers".to_string(), json!({}));
    }
    object
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .expect("mcpServers object")
        .insert(name.to_string(), entry(executable));
    write_atomically(path, &format!("{}\n", serde_json::to_string_pretty(&root)?))
}

fn write_toml(path: &Path, name: &str, executable: &Path) -> Result<()> {
    let text = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    let mut document: DocumentMut = text.parse().with_context(|| {
        format!(
            "parsing {}: fix the TOML, then rerun, or register with another client",
            path.display()
        )
    })?;
    if !document.get("mcp_servers").is_some_and(Item::is_table) {
        document["mcp_servers"] = Item::Table(Table::new());
    }
    let mut table = Table::new();
    table["command"] = value(executable.to_string_lossy().to_string());
    let mut args = Array::new();
    args.push("mcp");
    table["args"] = value(args);

    // An entry left under one of Codex's own Computer Use names would sit in the
    // slot its bundled plugin owns, so say so instead of leaving two silently.
    if let Some(servers) = document.get("mcp_servers").and_then(Item::as_table) {
        for stale in ["computer-use", "computer_use"] {
            if stale != name && servers.contains_key(stale) {
                eprintln!(
                    "{}: `mcp_servers.{stale}` is already there, under a name Codex's own Computer Use uses; `{name}` is registered beside it, so delete that table if it is an older entry from this server",
                    path.display()
                );
            }
        }
    }

    document["mcp_servers"][name] = Item::Table(table);
    write_atomically(path, &document.to_string())
}

fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let temporary = path.with_extension("computer-use-tmp");
    std::fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_names_parse_and_aliases_work() {
        assert_eq!(Client::parse("oxide").unwrap(), Client::Oxide);
        assert_eq!(Client::parse("Claude-Code").unwrap(), Client::Claude);
        assert_eq!(Client::parse("desktop").unwrap(), Client::Desktop);
        assert!(Client::parse("vscode").is_err());
    }

    #[test]
    fn reserved_and_odd_names_are_rejected() {
        assert!(validate_name(Client::Claude, "computer-use").is_err());
        assert!(validate_name(Client::Oxide, "computer-use").is_ok());
        assert!(validate_name(Client::Claude, "computer-use-local").is_ok());
        assert!(validate_name(Client::Oxide, "my server").is_err());
        assert!(validate_name(Client::Oxide, "computer-use").is_ok());
    }

    #[test]
    fn codex_keeps_the_names_it_ships_for_computer_use() {
        for name in ["computer-use", "computer_use", "node_repl", "cua_repl"] {
            assert!(
                validate_name(Client::Codex, name).is_err(),
                "`{name}` would shadow a server Codex ships"
            );
        }
        assert!(validate_name(Client::Codex, "computer-use-local").is_ok());
        assert!(validate_name(Client::Cursor, "computer-use").is_ok());
    }

    #[test]
    fn a_client_that_ships_computer_use_gets_a_name_it_does_not_reserve() {
        assert_eq!(default_name(Client::Oxide), "computer-use");
        assert_eq!(default_name(Client::Cursor), "computer-use");
        assert_eq!(default_name(Client::Codex), "computer-use-local");
        assert_eq!(default_name(Client::Claude), "computer-use-local");
    }

    #[test]
    fn a_toml_snippet_uses_the_codex_table_shape() {
        let text = snippet(Client::Codex, Path::new("/bin/computer-use"), "cu");
        assert!(text.contains("[mcp_servers.cu]"));
        assert!(text.contains("command = \"/bin/computer-use\""));
        assert!(text.contains("args = [\"mcp\"]"));
    }

    #[test]
    fn a_json_snippet_uses_the_mcp_servers_wrapper() {
        let value: Value =
            serde_json::from_str(&snippet(Client::Claude, Path::new("/bin/x"), "cu")).unwrap();
        assert_eq!(value["mcpServers"]["cu"]["args"], json!(["mcp"]));
        assert_eq!(value["mcpServers"]["cu"]["command"], json!("/bin/x"));
    }
}
