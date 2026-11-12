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
        "oxide-computer-use-cli-{label}-{}-{}",
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
    assert_eq!(names.len(), 15, "unexpected tool list: {stdout}");
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
        .is_some_and(|tools| tools.len() == 15));
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
        stdout.contains("[mcp_servers.oxide-computer-use]"),
        "Codex ships its own Computer Use, so this client gets a name it does not own: {stdout}"
    );
    assert!(stdout.contains("args = [\"mcp\"]"), "{stdout}");

    let (stdout, _, ok) = run(&["install", "--client", "claude", "--print"]);
    assert!(ok);
    let value: Value = serde_json::from_str(&stdout).expect("print emits JSON");
    assert!(
        value["mcpServers"]["oxide-computer-use"].is_object(),
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
    assert!(claude["mcpServers"]["oxide-computer-use"].is_object());
    assert!(claude["mcpServers"]["computer-use"].is_null());

    let (_, _, ok) = run_with_home(&home, &["install", "--client", "codex"]);
    assert!(ok);
    let codex = std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config");
    assert!(codex.contains("[mcp_servers.oxide-computer-use]"));

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
            stderr.contains("oxide-computer-use"),
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
    assert!(stdout.contains("oxide-computer-use"), "{stdout}");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# Codex's own settings live here too."),
        "{text}"
    );
    assert!(text.contains("[computer_use]"), "{text}");
    assert!(text.contains("default_app_access = \"deny\""), "{text}");
    assert!(text.contains("[mcp_servers.cua_repl]"), "{text}");
    assert!(text.contains("[mcp_servers.oxide-computer-use]"), "{text}");
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
    assert!(text.contains("[mcp_servers.oxide-computer-use]"), "{text}");
    std::fs::remove_dir_all(&home).ok();
}

fn config_of(home: &Path) -> Value {
    let text =
        std::fs::read_to_string(home.join(".config/oxide-computer-use/config.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_app_policy_round_trips_through_the_cli() {
    let home = temporary_home("policy");
    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("No app policy"), "{stdout}");

    let (stdout, _, ok) = run_with_home(&home, &["apps", "deny", "1Password"]);
    assert!(ok, "{stdout}");
    assert_eq!(config_of(&home)["denied_apps"], json!(["1Password"]));

    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("Denied: 1Password"), "{stdout}");

    let (_, stderr, ok) = run_with_home(&home, &["apps", "policy", "sometimes"]);
    assert!(!ok, "an unknown policy is refused");
    assert!(stderr.contains("allow"), "{stderr}");

    let (stdout, _, ok) = run_with_home(&home, &["apps", "policy", "deny"]);
    assert!(ok, "{stdout}");
    assert_eq!(config_of(&home)["app_policy"], json!("deny"));
    assert!(stdout.contains("default: deny"), "{stdout}");

    let (stdout, _, ok) = run_with_home(&home, &["apps", "grant-once", "TextEdit"]);
    assert!(ok, "{stdout}");
    assert_eq!(config_of(&home)["one_time_grant"]["app"], json!("TextEdit"));

    let (stdout, _, ok) = run_with_home(&home, &["apps", "revoke", "TextEdit"]);
    assert!(ok, "{stdout}");
    assert!(config_of(&home).get("one_time_grant").is_none(), "{stdout}");

    let (stdout, _, ok) = run_with_home(&home, &["apps", "revoke", "1Password"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("default policy again"), "{stdout}");
    assert_eq!(config_of(&home)["denied_apps"], json!([]));

    let (_, _, ok) = run_with_home(&home, &["apps", "reset"]);
    assert!(ok);
    let config = config_of(&home);
    assert_eq!(config["denied_apps"], json!([]));
    assert!(config.get("app_policy").is_none(), "{config}");

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn app_policy_tool_backs_the_client_settings_ui() {
    let home = temporary_home("app-policy-tool");
    let (stdout, stderr, ok) = run_with_home(
        &home,
        &[
            "call",
            "app_policy",
            "--args",
            "{\"action\":\"deny\",\"app\":\"1Password\"}",
        ],
    );
    assert!(ok, "{stderr}");
    let result: Value = serde_json::from_str(&stdout).unwrap();
    let state: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(state["denied"], json!(["1Password"]));
    assert_eq!(config_of(&home)["denied_apps"], json!(["1Password"]));

    let (stdout, stderr, ok) = run_with_home(
        &home,
        &[
            "call",
            "app_policy",
            "--args",
            "{\"action\":\"revoke\",\"app\":\"1Password\"}",
        ],
    );
    assert!(ok, "{stderr}");
    let result: Value = serde_json::from_str(&stdout).unwrap();
    let state: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(state["denied"], json!([]));

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_policy_file_that_does_not_parse_refuses_every_app() {
    let home = temporary_home("policy-broken");
    let path = home.join(".config/oxide-computer-use/config.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();

    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("Unreadable policy"), "{stdout}");

    // `doctor` exits nonzero while a grant is missing; the JSON is still printed.
    let (stdout, _, _) = run_with_home(&home, &["doctor", "--json"]);
    let status: Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        status["policy_error"]
            .as_str()
            .unwrap_or_default()
            .contains("apps reset"),
        "{status}"
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn doctor_reports_the_build_it_runs_and_the_installer_marker() {
    let home = temporary_home("provenance");
    let marker = home.join("bin/.oxide-computer-use-install");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(
        &marker,
        "source jaysonwu991/oxide-computer-use\nversion 0.1.0\narch darwin-arm64\n",
    )
    .unwrap();

    let (stdout, _, _) = run_with_home(&home, &["doctor", "--json"]);
    let status: Value = serde_json::from_str(&stdout).unwrap();
    let build = &status["provenance"];
    assert!(build["path"]
        .as_str()
        .unwrap_or_default()
        .ends_with("oxide-computer-use"));
    assert!(build["target"].as_str().unwrap_or_default().contains('-'));
    assert!(matches!(
        build["profile"].as_str(),
        Some("debug") | Some("release")
    ));
    assert!(
        build["install"].is_null(),
        "the marker is read beside the running binary, not in HOME: {build}"
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn tools_json_prints_the_surface_an_agent_searches() {
    let (stdout, stderr, ok) = run(&["tools", "--json"]);
    assert!(ok, "{stderr}");
    let specs: Value = serde_json::from_str(&stdout).unwrap();
    let specs = specs.as_array().expect("tools --json prints an array");
    assert_eq!(specs.len(), 15);
    for tool in specs {
        assert!(tool["name"].as_str().is_some_and(|name| !name.is_empty()));
        let description = tool["description"].as_str().unwrap_or_default();
        assert!(
            description.to_lowercase().contains("gui")
                || description.to_lowercase().contains("app")
                || description.to_lowercase().contains("element"),
            "{} has no app-facing words: {description}",
            tool["name"]
        );
    }
}

#[test]
fn a_subscribed_client_hears_that_the_screen_may_have_changed() {
    let mut child = Command::new(binary())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the MCP server starts");
    let mut stdin = child.stdin.take().expect("stdin");
    for request in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "resources/subscribe", "params": {"uri": "computer-use://screen"}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "permissions", "arguments": {}}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "click", "arguments": {"app": "__missing_computer_use_app__"}}}),
    ] {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().expect("the server exits");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let packets: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON packet"))
        .collect();

    let resources = packets
        .iter()
        .find(|packet| packet["id"] == json!(2))
        .expect("resources/list is answered");
    assert_eq!(
        resources["result"]["resources"][0]["uri"],
        json!("computer-use://screen")
    );

    let updates: Vec<&Value> = packets
        .iter()
        .filter(|packet| packet["method"] == json!("notifications/resources/updated"))
        .collect();
    assert_eq!(
        updates.len(),
        2,
        "subscribing reports the first view, and only the action reports another: {stdout}"
    );
    assert_eq!(updates[0]["params"]["uri"], json!("computer-use://screen"));
    assert!(
        updates[0].get("id").is_none(),
        "a notification carries no id: {stdout}"
    );

    let permissions = packets
        .iter()
        .position(|packet| packet["id"] == json!(4))
        .expect("permissions is answered");
    let action = packets
        .iter()
        .position(|packet| packet["id"] == json!(5))
        .expect("the action is answered");
    let reported = packets
        .iter()
        .rposition(|packet| packet["method"] == json!("notifications/resources/updated"))
        .expect("the action reported a change");
    assert!(
        permissions < reported && action < reported,
        "reading permissions reports nothing, and the action's notification follows it: {stdout}"
    );
    assert_eq!(
        reported,
        packets.len() - 1,
        "the answer comes before its notification: {stdout}"
    );
}

#[test]
fn verify_reports_every_native_path_and_exits_on_a_missing_one() {
    let (stdout, stderr, ok) = run(&["verify", "--json"]);
    assert!(
        !stdout.is_empty(),
        "verify prints its report even when a check fails: {stderr}"
    );
    let report: Value = serde_json::from_str(&stdout).unwrap();
    let checks = report["checks"].as_array().expect("checks is an array");
    let names: Vec<&str> = checks
        .iter()
        .filter_map(|check| check["name"].as_str())
        .collect();
    for expected in [
        "accessibility",
        "screen recording",
        "window server",
        "accessibility tree",
        "bundle identity",
        "screenshot",
        "takeover tap",
        "background input",
        "app policy",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} is missing: {names:?}"
        );
    }
    for check in checks {
        assert!(
            matches!(
                check["state"].as_str(),
                Some("ok") | Some("missing") | Some("skipped")
            ),
            "{check}"
        );
    }
    let missing = report["missing"].as_u64().unwrap_or(0);
    assert_eq!(
        ok,
        missing == 0,
        "the exit status follows the missing count, whatever this machine grants: {stdout}"
    );
}

#[test]
fn the_terminal_rule_round_trips_through_the_cli() {
    let home = temporary_home("terminals");
    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("Terminals: refused"), "{stdout}");

    let (stdout, _, _) = run_with_home(&home, &["doctor", "--json"]);
    let status: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(status["terminals"], json!("deny"));

    let (stdout, _, ok) = run_with_home(&home, &["apps", "terminals", "allow"]);
    assert!(ok, "{stdout}");
    assert_eq!(config_of(&home)["terminals"], json!("allow"));
    assert!(stdout.contains("Terminals: reachable"), "{stdout}");

    let (_, stderr, ok) = run_with_home(&home, &["apps", "terminals", "sometimes"]);
    assert!(!ok);
    assert!(stderr.contains("allow"), "{stderr}");

    let (_, _, ok) = run_with_home(&home, &["apps", "reset"]);
    assert!(ok);
    assert!(config_of(&home).get("terminals").is_none());

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_key_spelling_is_checked_before_a_permission_is_asked_for() {
    let (stdout, _, ok) = run(&["call", "press_key", "--args", "{\"key\":\"NoSuchKeyName\"}"]);
    assert!(!ok, "a key that does not exist is refused");
    assert!(stdout.contains("unknown key"), "{stdout}");
    assert!(
        stdout.contains("Return"),
        "the refusal should name a key that works: {stdout}"
    );

    // A literal backslash-n is the Return key, so it gets past the key check. What
    // happens next depends on the machine: a trusted process presses it, an
    // untrusted one stops at the permission gate. Neither is a refusal.
    let (stdout, _, _) = run(&["call", "press_key", "--args", "{\"key\":\"\\\\n\"}"]);
    assert!(
        !stdout.contains("unknown key"),
        "a literal backslash-n is a key, not a spelling mistake: {stdout}"
    );
    assert!(
        stdout.contains("Pressed `Return`") || stdout.contains("Accessibility"),
        "the escape has to reach the key path: {stdout}"
    );
}

#[test]
fn background_input_without_an_app_is_refused_before_a_permission() {
    let (stdout, _, ok) = run(&[
        "call",
        "type_text",
        "--args",
        "{\"text\":\"hi\",\"background\":true}",
    ]);
    assert!(!ok);
    assert!(stdout.contains("`background` needs `app`"), "{stdout}");
    assert!(!stdout.contains("Accessibility"), "{stdout}");
}

#[test]
fn the_self_automation_rule_round_trips_through_the_cli() {
    let home = temporary_home("self-automation");
    let (stdout, _, ok) = run_with_home(&home, &["apps", "list"]);
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("Self-automation: refused for the app hosting this server"),
        "{stdout}"
    );

    let (stdout, _, ok) = run_with_home(&home, &["apps", "self-automation", "allow"]);
    assert!(ok, "{stdout}");
    assert_eq!(config_of(&home)["self_automation"], json!("allow"));
    assert!(stdout.contains("hosting app is reachable"), "{stdout}");

    let (_, stderr, ok) = run_with_home(&home, &["apps", "self-automation", "sometimes"]);
    assert!(!ok);
    assert!(stderr.contains("allow"), "{stderr}");

    let (_, _, ok) = run_with_home(&home, &["apps", "reset"]);
    assert!(ok);
    assert!(config_of(&home).get("self_automation").is_none());

    let (stdout, _, _) = run_with_home(&home, &["doctor", "--json"]);
    let status: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(status["self_automation"], json!("deny"));

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
