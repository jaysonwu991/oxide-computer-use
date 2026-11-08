mod install;
#[cfg(target_os = "macos")]
mod macos;
mod mcp;
mod permissions;
mod policy;
mod runtime;
mod tools;

use std::path::PathBuf;
use std::process::Command as Process;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use install::Client;
use policy::Policy;
use runtime::Runtime;

#[derive(Parser)]
#[command(
    name = "oxide-computer-use",
    version,
    about = "Computer Use MCP server for any agent: Oxide, Claude Code, Codex or Cursor, driving macOS apps through native accessibility APIs"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the stdio MCP server that an agent connects to.
    Mcp,
    /// Call one Computer Use tool and print its MCP-style JSON result.
    Call {
        /// Tool name, for example list_apps or get_app_state.
        tool: Option<String>,
        /// Tool arguments as JSON.
        #[arg(long)]
        args: Option<String>,
        /// A JSON array of {"tool", "args"} calls sharing one process and one element-index state.
        #[arg(long)]
        calls: Option<String>,
        /// Read the call array from a file instead, resolved against the current directory.
        #[arg(long = "calls-file")]
        calls_file: Option<PathBuf>,
        /// Seconds to wait between calls in a sequence.
        #[arg(long, default_value_t = 1.0)]
        sleep: f64,
    },
    /// Print the tool names this build exposes.
    Tools,
    /// Check the permissions Computer Use needs.
    Doctor {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Ask macOS to show the permission prompts.
        #[arg(long)]
        prompt: bool,
    },
    /// Register this binary with an MCP client.
    #[command(alias = "install-oxide")]
    Install {
        /// oxide, claude, codex, cursor or desktop.
        #[arg(long, default_value = "oxide")]
        client: String,
        /// Write the project-scoped config instead of the user-scoped one.
        #[arg(long)]
        project: bool,
        /// Server name to register. Defaults to `oxide-computer-use`, or
        /// `oxide-computer-use-local` for the clients that ship their own Computer
        /// Use: Claude Code reserves the plain name, and Codex's bundled plugin
        /// owns it.
        #[arg(long)]
        name: Option<String>,
        /// Print the config entry instead of writing a file.
        #[arg(long)]
        print: bool,
    },
    /// Inspect or change which apps this server may touch.
    Apps {
        #[command(subcommand)]
        command: AppsCommand,
    },
}

#[derive(Subcommand)]
enum AppsCommand {
    /// Show the app policy and the running apps it marks.
    List,
    /// Allow an app; with no allow list yet, this starts one.
    Allow { app: String },
    /// Block an app, whatever the allow list says.
    Deny { app: String },
    /// Clear both lists and allow every app again.
    Reset,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("oxide-computer-use: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => {
            use clap::CommandFactory;
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
        Some(Command::Mcp) => mcp::serve(),
        Some(Command::Tools) => {
            for name in tools::names() {
                println!("{name}");
            }
            Ok(())
        }
        Some(Command::Call {
            tool,
            args,
            calls,
            calls_file,
            sleep,
        }) => call(tool, args, calls, calls_file, sleep),
        Some(Command::Doctor { json, prompt }) => doctor(json, prompt),
        Some(Command::Install {
            client,
            project,
            name,
            print,
        }) => install::install(Client::parse(&client)?, project, name.as_deref(), print),
        Some(Command::Apps { command }) => apps(command),
    }
}

fn parse_args(text: Option<&str>) -> Result<Value> {
    let Some(text) = text else {
        return Ok(json!({}));
    };
    let value: Value = serde_json::from_str(text).context("arguments must be a JSON object")?;
    if !value.is_object() {
        bail!("arguments must be a JSON object");
    }
    Ok(value)
}

fn call(
    tool: Option<String>,
    args: Option<String>,
    calls: Option<String>,
    calls_file: Option<PathBuf>,
    sleep: f64,
) -> Result<()> {
    let mut runtime = Runtime::new();
    let sequence = match (calls, calls_file) {
        (Some(text), _) => Some(text),
        (None, Some(path)) => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()?.join(&path)
            };
            Some(std::fs::read_to_string(&resolved).with_context(|| {
                format!(
                    "reading {} (resolved from `{}`)",
                    resolved.display(),
                    path.display()
                )
            })?)
        }
        (None, None) => None,
    };

    let Some(sequence) = sequence else {
        let tool = tool.context("pass a tool name, or --calls/--calls-file")?;
        let args = parse_args(args.as_deref())?;
        return report(&mut runtime, &tool, &args);
    };

    let parsed: Value = serde_json::from_str(&sequence).context("--calls must be a JSON array")?;
    let list = parsed
        .as_array()
        .context("--calls must be a JSON array of {\"tool\", \"args\"} objects")?;
    let mut failures = 0;
    for (index, entry) in list.iter().enumerate() {
        let name = entry
            .get("tool")
            .and_then(Value::as_str)
            .with_context(|| format!("call {index} is missing `tool`"))?;
        let arguments = entry.get("args").cloned().unwrap_or_else(|| json!({}));
        if report(&mut runtime, name, &arguments).is_err() {
            failures += 1;
        }
        if index + 1 < list.len() && sleep > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f64(sleep));
        }
    }
    if failures > 0 {
        bail!("{failures} of {} calls failed", list.len());
    }
    Ok(())
}

fn report(runtime: &mut Runtime, tool: &str, args: &Value) -> Result<()> {
    match tools::execute(runtime, tool, args) {
        Ok(outcome) => {
            println!("{}", serde_json::to_string(&tools::content_json(&outcome))?);
            Ok(())
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "content": [{ "type": "text", "text": format!("{error:#}") }],
                    "isError": true
                }))?
            );
            bail!("{error:#}")
        }
    }
}

fn apps(command: AppsCommand) -> Result<()> {
    let mut policy = Policy::load();
    match command {
        AppsCommand::List => {
            println!("{}", policy.describe());
            if let Some(path) = Policy::path() {
                println!("Config: {}", path.display());
            }
            println!("\nRunning apps:\n{}", Runtime::new().list_apps(false)?);
        }
        AppsCommand::Allow { app } => {
            policy.allow(&app)?;
            println!("Allowed `{app}`.");
            println!("{}", policy.describe());
        }
        AppsCommand::Deny { app } => {
            policy.deny(&app)?;
            println!("Denied `{app}`.");
            println!("{}", policy.describe());
        }
        AppsCommand::Reset => {
            policy.clear()?;
            println!(
                "Cleared the app policy; every app with Accessibility access is reachable again."
            );
        }
    }
    Ok(())
}

fn doctor(as_json: bool, prompt: bool) -> Result<()> {
    let version = macos_version();
    let accessibility = accessibility(prompt);
    let screen_recording = screen_recording(prompt);
    let policy = Policy::load();

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "version": env!("CARGO_PKG_VERSION"),
                "macos": version,
                "accessibility": accessibility,
                "screen_recording": screen_recording,
                "allowed_apps": policy.allowed(),
                "denied_apps": policy.denied(),
                "launching_app": permissions::launching_app(),
                "guidance": permissions::guidance(),
                "config": Policy::path().map(|path| path.display().to_string()),
            }))?
        );
    } else {
        println!("oxide-computer-use {}", env!("CARGO_PKG_VERSION"));
        println!(
            "macOS: {}",
            version.clone().unwrap_or_else(|| "unknown".to_string())
        );
        println!(
            "Accessibility: {}",
            if accessibility { "granted" } else { "missing" }
        );
        println!(
            "Screen Recording: {}",
            if screen_recording {
                "granted"
            } else {
                "missing"
            }
        );
        println!("{}", policy.describe());
        if !accessibility || !screen_recording {
            println!("\n{}", permissions::guidance());
        }
    }

    if accessibility && screen_recording {
        Ok(())
    } else {
        bail!("permissions are missing; Accessibility enables app control, Screen Recording enables screenshots")
    }
}

#[cfg(target_os = "macos")]
fn macos_version() -> Option<String> {
    let output = Process::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn macos_version() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn accessibility(prompt: bool) -> bool {
    if prompt {
        macos::ax::request_trust(true)
    } else {
        macos::ax::trusted()
    }
}

#[cfg(not(target_os = "macos"))]
fn accessibility(_prompt: bool) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn screen_recording(prompt: bool) -> bool {
    if prompt {
        macos::apps::request_screen_recording()
    } else {
        macos::apps::screen_recording_allowed()
    }
}

#[cfg(not(target_os = "macos"))]
fn screen_recording(_prompt: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_arguments_default_to_an_empty_object() {
        assert_eq!(parse_args(None).unwrap(), json!({}));
    }

    #[test]
    fn arguments_must_be_a_json_object() {
        assert!(parse_args(Some("{\"app\":\"Finder\"}")).is_ok());
        assert!(parse_args(Some("[1,2]")).is_err());
        assert!(parse_args(Some("not json")).is_err());
    }
}
