//! End-to-end checks of the binary: CLI surface, the stdio MCP handshake, the
//! client installers and the app policy.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_oxide-computer-use")
}

fn run(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(binary())
        .args(args)
        .output()
        .expect("the binary runs");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

fn run_with_home(home: &Path, args: &[&str]) -> (String, String, bool) {
    let output = Command::new(binary())
        .args(args)
        .env("HOME", home)
        .env_remove("OXIDE_COMPUTER_USE_CONFIG")
        .output()
        .expect("the binary runs");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

fn temporary_home(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "oxide-cu-cli-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&path).expect("a temporary home");
    path
}

#[test]
fn tools_lists_every_registered_tool() {
    let (stdout, _, ok) = run(&["tools"]);
    assert!(ok);
    let names: Vec<&str> = stdout.lines().collect();
    assert_eq!(names.len(), 13, "unexpected tool list: {stdout}");
    assert!(names.contains(&"list_apps"));
    assert!(names.contains(&"get_app_state"));
    assert!(names.contains(&"set_value"));
    assert!(names.contains(&"select_text"));
}

#[test]
fn the_mcp_server_answers_initialize_and_tools_list() {
    let mut child = Command::new(binary())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the MCP server starts");
    let mut stdin = child.stdin.take().expect("stdin");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    )
    .unwrap();
    drop(stdin);
    let output = child.wait_with_output().expect("the server exits");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let responses: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON packet"))
        .collect();
    assert_eq!(
        responses.len(),
        2,
        "notifications are not answered: {stdout}"
    );
    assert_eq!(responses[0]["id"], json!(1));
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        json!("2024-11-05"),
        "the client's revision is echoed when it is supported"
    );
    assert!(responses[0]["result"]["capabilities"]["tools"].is_object());
    assert_eq!(responses[1]["id"], json!(2));
    assert!(responses[1]["result"]["tools"]
        .as_array()
        .is_some_and(|tools| tools.len() == 13));
}

#[test]
fn install_prints_a_stdio_entry_for_each_client_shape() {
    let (stdout, _, ok) = run(&["install", "--client", "oxide", "--print"]);
    assert!(ok);
    let value: Value = serde_json::from_str(&stdout).expect("print emits JSON");
    let entry = &value["mcpServers"]["oxide-computer-use"];
    assert_eq!(entry["args"], json!(["mcp"]));
    assert!(entry["command"]
        .as_str()
        .is_some_and(|command| command.ends_with("oxide-computer-use")));

    let (stdout, _, ok) = run(&["install", "--client", "codex", "--print"]);
    assert!(ok);
    assert!(
        stdout.contains("[mcp_servers.oxide-computer-use-local]"),
        "Codex ships its own Computer Use, so this client gets a name it does not own: {stdout}"
    );
    assert!(stdout.contains("args = [\"mcp\"]"), "{stdout}");

    let (stdout, _, ok) = run(&["install", "--client", "claude", "--print"]);
    assert!(ok);
    let value: Value = serde_json::from_str(&stdout).expect("print emits JSON");
    assert!(
        value["mcpServers"]["oxide-computer-use-local"].is_object(),
        "Claude Code gets a name it does not reserve: {stdout}"
    );
}

#[test]
fn install_writes_each_clients_config_file() {
    let home = temporary_home("install");

    let (stdout, _, ok) = run_with_home(&home, &["install", "--client", "oxide"]);
    assert!(ok, "{stdout}");
    let oxide: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".oxide/mcp.json")).expect("oxide config"),
    )
    .unwrap();
    assert_eq!(
        oxide["mcpServers"]["oxide-computer-use"]["args"],
        json!(["mcp"])
    );

    let (_, _, ok) = run_with_home(&home, &["install", "--client", "claude"]);
    assert!(ok);
    let claude: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".claude.json")).expect("claude config"),
    )
    .unwrap();
    assert!(claude["mcpServers"]["oxide-computer-use-local"].is_object());
    assert!(claude["mcpServers"]["oxide-computer-use"].is_null());

    let (_, _, ok) = run_with_home(&home, &["install", "--client", "codex"]);
    assert!(ok);
    let codex = std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config");
    assert!(codex.contains("[mcp_servers.oxide-computer-use-local]"));

    let (_, _, ok) = run_with_home(&home, &["install", "--client", "cursor"]);
    assert!(ok);
    assert!(home.join(".cursor/mcp.json").exists());

    let (_, _, ok) = run_with_home(&home, &["install", "--client", "desktop"]);
    assert!(ok);
    assert!(home
        .join("Library/Application Support/Claude/claude_desktop_config.json")
        .exists());

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn an_install_keeps_settings_it_did_not_write() {
    let home = temporary_home("merge");
    let path = home.join(".oxide/mcp.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#,
    )
    .unwrap();
    let (_, _, ok) = run_with_home(&home, &["install", "--client", "oxide"]);
    assert!(ok);
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["theme"], json!("dark"));
    assert!(value["mcpServers"]["other"].is_object());
    assert!(value["mcpServers"]["oxide-computer-use"].is_object());
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn the_name_claude_code_reserves_is_refused() {
    let (_, stderr, ok) = run(&[
        "install",
        "--client",
        "claude",
        "--name",
        "computer-use",
        "--print",
    ]);
    assert!(!ok);
    assert!(stderr.contains("reserved"), "{stderr}");
}

#[test]
fn the_names_codex_computer_use_owns_are_refused() {
    for name in ["computer-use", "computer_use", "cua_repl"] {
        let (_, stderr, ok) = run(&["install", "--client", "codex", "--name", name, "--print"]);
        assert!(!ok, "`{name}` was accepted for Codex");
        assert!(stderr.contains("reserved"), "{stderr}");
        assert!(
            stderr.contains("oxide-computer-use-local"),
            "the error names the default to use instead: {stderr}"
        );
    }
}

#[test]
fn a_codex_install_leaves_codex_computer_use_settings_alone() {
    let home = temporary_home("codex-oxide-computer-use");
    let path = home.join(".codex/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        concat!(
            "# Codex's own settings live here too.\n",
            "[computer_use]\n",
            "default_app_access = \"deny\"\n",
            "\n",
            "[mcp_servers.cua_repl]\n",
            "command = \"/Applications/ChatGPT.app/Contents/MacOS/cua_repl\"\n",
        ),
    )
    .unwrap();

    let (stdout, stderr, ok) = run_with_home(&home, &["install", "--client", "codex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("oxide-computer-use-local"), "{stdout}");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# Codex's own settings live here too."),
        "{text}"
    );
    assert!(text.contains("[computer_use]"), "{text}");
    assert!(text.contains("default_app_access = \"deny\""), "{text}");
    assert!(text.contains("[mcp_servers.cua_repl]"), "{text}");
    assert!(
        text.contains("[mcp_servers.oxide-computer-use-local]"),
        "{text}"
    );
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_codex_install_warns_about_an_entry_under_codex_computer_use_name() {
    let home = temporary_home("codex-stale");
    let path = home.join(".codex/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "[mcp_servers.computer-use]\ncommand = \"/old/computer-use\"\nargs = [\"mcp\"]\n",
    )
    .unwrap();

    let (_, stderr, ok) = run_with_home(&home, &["install", "--client", "codex"]);
    assert!(ok, "{stderr}");
    assert!(
        stderr.contains("mcp_servers.computer-use"),
        "the warning names the stale table: {stderr}"
    );

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("[mcp_servers.computer-use]"), "{text}");
    assert!(
        text.contains("[mcp_servers.oxide-computer-use-local]"),
        "{text}"
    );
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn the_app_policy_round_trips_through_the_cli() {
    let home = temporary_home("policy");
    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("No app policy"), "{stdout}");

    let (stdout, _, ok) = run_with_home(&home, &["apps", "deny", "1Password"]);
    assert!(ok);
    assert!(stdout.contains("1Password"), "{stdout}");

    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok);
    assert!(stdout.contains("Denied apps: 1Password"), "{stdout}");

    let (_, _, ok) = run_with_home(&home, &["apps", "reset"]);
    assert!(ok);
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".config/oxide-computer-use/config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["denied_apps"], json!([]));

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn an_unknown_tool_is_reported_as_an_error_result() {
    let (stdout, stderr, ok) = run(&["call", "no_such_tool"]);
    assert!(!ok, "a failed call exits nonzero");
    assert!(stderr.contains("unknown tool"), "{stderr}");
    assert!(stdout.contains("\"isError\":true"), "{stdout}");
}

#[test]
fn a_missing_calls_file_names_the_resolved_path() {
    let (_, stderr, ok) = run(&["call", "--calls-file", "definitely-missing.json"]);
    assert!(!ok);
    assert!(stderr.contains("definitely-missing.json"), "{stderr}");
    assert!(
        stderr.contains("resolved from"),
        "the error should name the path it tried: {stderr}"
    );
}

#[test]
fn non_ascii_text_survives_the_stdio_transport() {
    // The reply is produced by this server alone: no app, permission or policy
    // is involved, so the check holds on any machine.
    let mut child = Command::new(binary())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the MCP server starts");
    let mut stdin = child.stdin.take().expect("stdin");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "工具", "arguments": {}}})
    )
    .unwrap();
    drop(stdin);
    let output = child.wait_with_output().expect("the server exits");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let response: Value = serde_json::from_str(stdout.trim()).expect("one JSON packet");
    assert_eq!(response["result"]["isError"], json!(true));
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains("工具"), "the name was mangled: {text}");
    assert!(text.contains("unknown tool"), "{text}");
}

#[test]
fn permissions_can_be_checked_in_the_calling_process_without_prompting() {
    let (stdout, stderr, ok) = run(&["call", "permissions"]);
    assert!(ok, "{stderr}");
    let result: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["isError"], false);
    let status: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(status["accessibility"].is_boolean());
    assert!(status["screen_recording"].is_boolean());
    assert!(status["guidance"].as_str().unwrap().contains("hosting app"));
}
