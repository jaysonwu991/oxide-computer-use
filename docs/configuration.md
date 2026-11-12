# Configuration

There is no config file for behaviour: limits are per-call arguments, so a
session can tighten or loosen them without restarting anything. What *is*
configured on disk is which apps the server may touch, and which MCP client
launches it.

## Files

| File | Written by | Read by | Purpose |
| --- | --- | --- | --- |
| `~/.config/oxide-computer-use/config.json` | `oxide-computer-use apps allow\|deny\|revoke\|grant-once\|policy\|self-automation\|reset` | every tool call | Per-app approvals, their bundle identities, the default policy and the self-automation rule. |
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
`~/.codex/computer-use/` component directory untouched.

`HOME` decides where all of the above lives. `OXIDE_COMPUTER_USE_CONFIG` is the only
other environment variable the server reads.

### Server name

The entry is registered as `oxide-computer-use` for every client. Claude Code
reserves `computer-use` as an exact server name, and Codex's bundled Computer Use
plugin already owns `computer-use`, `computer_use`, `node_repl`, `cua_repl` and
`codex_apps`; `oxide-computer-use` avoids all of those slots. `--name` picks
another unreserved name, and a reserved name is refused with that explanation.
Names may contain letters, digits, `-` and `_` only, so every client accepts them.

## App policy

```json
{
  "app_policy": "deny",
  "allowed_apps": ["Finder", "TextEdit", "Google Chrome"],
  "denied_apps": ["1Password", "Keychain Access"],
  "app_identities": {
    "Finder": {
      "bundle_id": "com.apple.finder",
      "path": "/System/Library/CoreServices/Finder.app"
    }
  },
  "one_time_grant": { "app": "Preview" },
  "self_automation": "allow",
  "terminals": "allow"
}
```

Semantics:

- **Deny wins.** A name in `denied_apps` is refused even if the allow list also
  matches it.
- **The allow list decides when `app_policy` is absent.** An empty list means
  "everything not denied"; once `allowed_apps` has one entry, every app not on it
  is refused. `app_policy` makes that choice explicit: `"allow"` or `"deny"` for
  an app with no decision of its own.
- **An approval can be pinned to a bundle identity.** `apps allow` records the
  bundle identifier and path of the running or installed app, and every later
  call reads the identity of the process it resolved. A different app answering
  to the same name is refused, naming both identities. If no bundle can be
  identified, the approval falls back to its name.
- **One approval can be spent on one call.** `apps grant-once` writes
  `one_time_grant`, which is consumed by the next call that uses it and removed
  from the file. It is refused, and kept, when the running app's identity differs
  from the one recorded.
- Matching is case-insensitive against the display name, bundle identifier or
  bundle path. `apps deny 1password`, `apps deny com.example.App`, and a path
  rule all reach the same app.
- The check happens after `app` is resolved and before any accessibility call, so
  a denial reads as a policy decision, not as a permissions problem.
- `list_apps` still shows a blocked app in its JSON `policy` field, so an agent
  can tell "refused" from "not running".
- **The file is re-read on every call.** An approval, a revocation or a one-time
  grant written by `apps allow`/`revoke`/`grant-once` in one terminal reaches an
  `oxide-computer-use mcp` server that is already running; nothing has to be restarted.
- **A file that does not parse refuses every app**, with a message naming the
  path and `apps reset`, instead of falling back to "unlisted, so allowed".
- **Terminals are refused by default.** A terminal is a way to start another
  agent, so an app whose bundle is one of the known shells (`Terminal`, `iTerm2`,
  `Ghostty`, `Warp`, `WezTerm`, `Alacritty`, `kitty`, `Tabby`, `Hyper`) is
  refused before any accessibility call. The bundle identity decides; the name is
  the fallback only when no identity can be read, so a lookalike name is not
  caught. `"terminals": "allow"` (or `apps terminals allow`) lifts it, and a
  single terminal can be exempted without lifting the rest by name only if the
  identity is unreadable — prefer the policy switch. The refusal names the bundle
  and the command.
- **Self-automation is refused by default.** The app bundle that launched the
  server — the parent app the MCP host runs under, found by walking up to a
  `.app` — may not be driven, because an agent clicking its own window is a loop.
  `"self_automation": "allow"` (or `apps self-automation allow`) lifts it. The
  check compares bundle paths, so it never matches a different app, and a server
  started from a plain shell has no host to refuse. This is narrower than Codex's
  documented terminal and self-automation restriction: shells stay reachable
  unless one is the host.

Commands:

```sh
oxide-computer-use apps list                   # policy, config path and launchable apps it marks
oxide-computer-use apps allow "Finder"         # approve from now on, pinned to its identity
oxide-computer-use apps grant-once "Preview"   # approve exactly one call
oxide-computer-use apps deny "1Password"       # always wins
oxide-computer-use apps revoke "Finder"        # withdraw every decision about one app, a deny included
oxide-computer-use apps policy deny            # an app with no decision is refused
oxide-computer-use apps self-automation allow  # let the hosting app be driven too
oxide-computer-use apps terminals allow       # let terminal emulators be driven
oxide-computer-use apps reset                  # clear everything and allow every app again
```

An app that is refused is told exactly what to ask for: the refusal message names
`apps grant-once` for one call and `apps allow` for every call, and a deny names
`apps revoke`. That message is the whole approval surface for a stdio MCP server:
the decision belongs to the person, and the client's own tool approval is a
separate gate.

## Defaults

| Setting | Default | Overridden by |
| --- | --- | --- |
| Tree nodes per snapshot | 1200 | `get_app_state.max_tree_nodes`, `query.max_tree_nodes` |
| Tree depth | 64 | `get_app_state.max_tree_depth` |
| Characters per text value | 200 | `get_app_state.text_limit` (`"max"` = no limit) |
| Reply text | 20000 characters | `max_output_chars` on `get_app_state` and `query` |
| `query` matches | 20 | `query.max_results` |
| Scroll amount | 1 page | `scroll.pages` |
| Scroll direction | required by the schema | `scroll.direction` |
| Delay between `--calls` calls | 1 s | `call --sleep` |
| App-state screenshot | on | `get_app_state.include_screenshot: false` |
| State text | diff after the first read | `get_app_state.disableDiff` |
| On-screen frames in the tree | off | `get_app_state.include_frames` |

Exceeding a budget is always reported: the reply says how much was cut and which
argument raises the limit.

## What a window must look like to count as running

Installed `.app` bundles are listed from the standard application directories
when they have an executable. Running bundles elsewhere are added too. Usable
windows obey these rules:

- layer 0 only, so menu-bar strips, panels and overlays are not apps;
- at least 120×80 points, so 1×1 and 10×10 helpers disappear;
- owners that are window-server agents are hidden: `Window Server`,
  `loginwindow`, `Dock`, `Control Center`, `Notification Center`, `通知中心`,
  `SystemUIServer`, `通用控制`, `自动填充`, and any name ending in `Service`.

Windows in another Space are kept, because accessibility still reaches those
apps. `list_apps {"include_helpers": true}` adds running hidden surfaces — the
way to find a native file picker that belongs to no app. Resolution of `app`
searches those helpers too, so a pid works even when the name is not listed.

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

`oxide-computer-use doctor` reports both, plus the policy, the self-automation and
terminal rules, its path and the build it runs (see
[provenance](development.md#provenance)). `oxide-computer-use verify` exercises each
native path and prints the evidence.
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
