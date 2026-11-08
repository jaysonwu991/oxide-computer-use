# Configuration

There is no config file for behaviour: limits are per-call arguments, so a
session can tighten or loosen them without restarting anything. What *is*
configured on disk is which apps the server may touch, and which MCP client
launches it.

## Files

| File | Written by | Read by | Purpose |
| --- | --- | --- | --- |
| `~/.config/oxide-computer-use/config.json` | `oxide-computer-use apps allow\|deny\|reset` | the server at start | App allow/deny policy. |
| `$OXIDE_COMPUTER_USE_CONFIG` | you | the server at start | Overrides the policy path (a file, not a directory). |
| `~/.oxide/mcp.json` | `install --client oxide` | Oxide | Global MCP servers. |
| `./.oxide/mcp.json` | `install --client oxide --project` | Oxide | Project MCP servers. |
| `~/.claude.json` | `install --client claude` | Claude Code | User-scope MCP servers. |
| `./.mcp.json` | `install --client claude --project` | Claude Code | Project-scope MCP servers. |
| `~/.codex/config.toml` | `install --client codex` | Codex CLI | `[mcp_servers.<name>]`. |
| `./.codex/config.toml` | `install --client codex --project` | Codex CLI in that repo | Project servers. |
| `~/.cursor/mcp.json` | `install --client cursor` | Cursor | User servers. |
| `./.cursor/mcp.json` | `install --client cursor --project` | Cursor in that repo | Project servers. |
| `~/Library/Application Support/Claude/claude_desktop_config.json` | `install --client desktop` | Claude Desktop | Its only scope. |

`install` merges into the existing file — other servers, other settings and, for
Codex, comments and ordering survive — and writes through a temporary file plus a
rename, so an interrupted write cannot truncate a config. In
`~/.codex/config.toml` it adds one `[mcp_servers.<name>]` table and leaves
Codex's `[computer_use]` app-access policy, its bundled plugin entries and the
`~/.codex/oxide-computer-use/` component directory untouched.

`HOME` decides where all of the above lives. `OXIDE_COMPUTER_USE_CONFIG` is the only
other environment variable the server reads.

### Server name

The entry is registered as `oxide-computer-use`, except for the two clients that ship
their own Computer Use. Claude Code reserves `computer-use` as that exact server name and
would skip the server; Codex's bundled Computer Use plugin already answers to it, and
`node_repl`, `cua_repl` and `codex_apps` are names Codex gives its own MCP
servers. `--client claude` and `--client codex` therefore register
`oxide-computer-use-local`, beside a built-in server rather than over it. `--name`
overrides the default where a client has nothing reserved, and a name a client
does reserve is refused with that explanation. Names may contain letters, digits,
`-` and `_` only, so every client accepts them.

## App policy

```json
{
  "allowed_apps": ["Finder", "TextEdit", "Google Chrome"],
  "denied_apps": ["1Password", "Keychain Access"]
}
```

Semantics:

- **Deny wins.** A name in `denied_apps` is refused even if the allow list also
  matches it.
- **An empty allow list means "everything not denied".** Once `allowed_apps` has
  one entry, every app not on it is refused.
- Matching is case-insensitive and exact on the app name as macOS reports it —
  the same string `list_apps` prints. `apps deny 1password` blocks `1Password`.
- The check happens after `app` is resolved and before any accessibility call, so
  a denial reads as a policy decision, not as a permissions problem.
- `list_apps` still shows a blocked app, marked `[blocked by app policy]`, so an
  agent can tell "refused" from "not running".

Commands:

```sh
oxide-computer-use apps list              # policy + running apps it marks
oxide-computer-use apps allow "Finder"    # starts or extends the allow list
oxide-computer-use apps deny "1Password"  # always wins
oxide-computer-use apps reset             # clears both lists
```

The policy is read **when a process starts**. A running `oxide-computer-use mcp`
server keeps the policy it started with; change it, then restart the session (or
let the client reconnect) for it to take effect. `oxide-computer-use call` reads it per
invocation.

## Defaults

| Setting | Default | Overridden by |
| --- | --- | --- |
| Tree nodes per snapshot | 1200 | `get_app_state.max_tree_nodes`, `query.max_tree_nodes` |
| Tree depth | 64 | `get_app_state.max_tree_depth` |
| Characters per text value | 200 | `get_app_state.text_limit` (`"max"` = no limit) |
| Reply text | 20000 characters | `max_output_chars` on `get_app_state` and `query` |
| `query` matches | 20 | `query.max_results` |
| Scroll amount | 3 lines | `scroll.lines` |
| Scroll direction | `down` | `scroll.direction` |
| Delay between `--calls` calls | 1 s | `call --sleep` |
| Screenshot | off | `get_app_state.include_screenshot`, `screenshot` |
| On-screen frames in the tree | off | `get_app_state.include_frames` |

Exceeding a budget is always reported: the reply says how much was cut and which
argument raises the limit.

## What a window must look like to be listed

`list_apps` hides surfaces that are not usable windows:

- layer 0 only, so menu-bar strips, panels and overlays are not apps;
- at least 120×80 points, so 1×1 and 10×10 helpers disappear;
- owners that are window-server agents are hidden: `Window Server`,
  `loginwindow`, `Dock`, `Control Center`, `Notification Center`, `通知中心`,
  `SystemUIServer`, `通用控制`, `自动填充`, and any name ending in `Service`.

Windows in another Space are kept, because accessibility still reaches those
apps. `list_apps {"include_helpers": true}` shows the hidden surfaces — the way to
find a native file picker that belongs to no app. Resolution of `app` searches
those helpers too, so a pid works even when the name is not listed.

## Permissions

| Permission | Needed by | Granted to |
| --- | --- | --- |
| Accessibility | app inspection/control; not `permissions`, `list_apps` or `screenshot` | the responsible hosting app; normally your terminal or direct-launch client, or the optional native helper |
| Screen Recording | `screenshot`, `include_screenshot` | the same process |

macOS attributes grants to a responsible app/process. Direct stdio launches
normally use the terminal or client app; the optional Oxide helper is launched
through LaunchServices as a separate app. Query `permissions` from the actual MCP
connection and verify the recipient in System Settings. Existing grants to
Oxide.app do not grant the helper access. Moving/rebuilding a development bundle
can affect attribution or leave a stale entry; recheck grants after reconnecting.

`oxide-computer-use doctor` reports both, plus the policy and its path.
`oxide-computer-use doctor --prompt` asks macOS to show the prompts. Neither can grant
on your behalf; only System Settings can.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command ran. A tool may still report `isError: true` — see below. |
| 1 | The command failed: bad arguments, unknown tool, client config it could not parse, or `doctor` finding a missing permission. |

`oxide-computer-use call` prints the MCP-shaped result for every call, including
failures, and exits 1 if any call in the batch failed. That makes it usable in a
shell pipeline while keeping the agent-facing message intact.


## Oxide native helper policy context

The optional [Oxide helper](oxide-helper.md) starts through LaunchServices, which
does not inherit the MCP caller's environment/cwd as a normal child process does.
Its proxy explicitly transfers `HOME`, `OXIDE_COMPUTER_USE_CONFIG` and the working
directory through a private context file before starting the bundled server.
This preserves a relative custom policy path and its allow/deny decisions. Other
caller environment variables are not forwarded. The helper smoke check includes
a custom relative deny-policy regression. Client tool approvals and macOS grants
remain separate; installing the helper does not migrate or reset either.
