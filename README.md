# oxide-computer-use

Computer Use as a stdio MCP server for any agent on macOS — Oxide, Claude Code,
Codex CLI, Cursor, Claude Desktop. It lists the running apps, reads an app's
accessibility tree, and clicks, types, selects, scrolls and drags inside it.

The idea matches [open-computer-use](https://github.com/iFurySt/open-computer-use);
the core implementation is a single Rust binary with
[one tool list](src/tools.rs) and explicit action failures. Oxide also provides
an opt-in [native companion app](docs/oxide-helper.md) in its Desktop repository.

```text
agent (Oxide / Claude Code / Codex / Cursor) ──stdio JSON-RPC──▶ oxide-computer-use
                                                   ├─▶ Accessibility (AX) elements, actions, values
                                                   ├─▶ CoreGraphics pointer, keyboard, scroll
                                                   └─▶ window server + screencapture
```

## Requirements

- macOS 14 or later.
- Accessibility permission for the terminal or app that launches the server
  (System Settings → Privacy & Security → Accessibility).
- Screen Recording permission for `screenshot` and `include_screenshot`.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/jaysonwu991/oxide-computer-use/main/install.sh | sh
oxide-computer-use doctor
```

The installer picks the Apple Silicon or Intel archive from the
[latest release](https://github.com/jaysonwu991/oxide-computer-use/releases), checks
its SHA-256, and installs `oxide-computer-use` into `~/.local/bin`.
`OXIDE_COMPUTER_USE_INSTALL_DIR`, `OXIDE_COMPUTER_USE_VERSION` and `OXIDE_COMPUTER_USE_REPO`
override where it goes and what it installs. There is no Linux or Windows
archive: the tools drive macOS APIs, so another platform builds to explicit
"not implemented" errors — see [docs/install.md](docs/install.md).

## Build from source

```sh
cargo build --release
install -m 755 target/release/oxide-computer-use ~/.local/bin/oxide-computer-use
```

## Connect a client

```sh
oxide-computer-use install --client oxide     # ~/.oxide/mcp.json
oxide-computer-use install --client claude    # ~/.claude.json
oxide-computer-use install --client codex     # ~/.codex/config.toml
oxide-computer-use install --client cursor    # ~/.cursor/mcp.json
oxide-computer-use install --client desktop   # Claude Desktop
oxide-computer-use install --print            # the entry, for any other client
```

It registers as `oxide-computer-use`, except for the clients that already ship their
own Computer Use: Claude Code reserves `computer-use` as that name and skips a server registered
under it, and Codex's bundled Computer Use plugin owns it, so `--client claude`
and `--client codex` use `oxide-computer-use-local` and sit beside the built-in rather
than over it. `--name` picks another name where a client has none reserved.
Details and the equivalent per-client commands are in
[docs/clients.md](docs/clients.md).

## Check permissions and policy

```sh
oxide-computer-use doctor          # permissions, app policy, versions
oxide-computer-use doctor --prompt # ask macOS to show the permission prompts
oxide-computer-use apps deny "1Password"
oxide-computer-use apps list
```

## Tools

| Tool | What it does |
| --- | --- |
| `permissions` | Check permissions in the MCP host and identify the launching app; no prompts. |
| `list_apps` | Apps that own windows, with pid and window titles; `include_helpers` also shows window-server helpers such as a native file picker. |
| `get_app_state` | One app's accessibility tree as indexed lines; `window` binds it to a single window, `include_screenshot` attaches a PNG, `max_output_chars` caps the reply. |
| `query` | Search that tree by text and/or role without printing all of it. Also refreshes the indexes. |
| `click` | `AXPress` when the element has it, otherwise a pointer click at its center or at `x`/`y`; the reply says which ran. |
| `perform_secondary_action` | A named AX action such as `AXShowMenu`; an action the element does not support is refused with the list it does support. |
| `set_value` | Writes `AXValue` only when the element reports it settable, then reads it back. |
| `select_text` | Selects a span inside a text element, counted in UTF-16 units like Cocoa ranges. |
| `type_text` | Types into the focused control; activates the app first. |
| `press_key` | One key with optional `cmd`/`shift`/`alt`/`ctrl` modifiers. |
| `scroll` | Wheel events at the frontmost window, an element center, or `x`/`y`; needs no snapshot. |
| `drag` | Paced HID drag from an element center or `from_x`/`from_y` to `to_x`/`to_y`. |
| `screenshot` | A PNG of the screen or of one app's window. |

Element indexes are per-process state: `get_app_state` and `query` replace the
snapshot, each index is bound to the `AXWindow` it came from, and an index from
an old snapshot, the wrong app or the wrong window is refused by name instead of
being applied to whatever is there now.

## Call it without an agent

```sh
oxide-computer-use call list_apps
oxide-computer-use call get_app_state --args '{"app":"TextEdit","window":0}'
oxide-computer-use call click --args '{"app":"TextEdit","element_index":12}'
```

A sequence shares one process and one snapshot state:

```sh
oxide-computer-use call --calls '[
  {"tool":"get_app_state","args":{"app":"TextEdit"}},
  {"tool":"select_text","args":{"app":"TextEdit","element_index":9,"text":"Hello"}},
  {"tool":"press_key","args":{"app":"TextEdit","key":"c","modifiers":"cmd"}}
]'
```

Each call prints the MCP-shaped result, and the process exits nonzero when a
call fails.

## How it works

- **Observe** — `AXUIElementCreateApplication`, then a depth-limited walk of
  `AXChildren` rendering role, title, description, value, identifier, focus and
  disabled state. Truncation is reported, never silent.
- **Act** — accessibility actions and attribute writes first, because they reach
  a background app; synthetic `CGEvent` input only where AX has no equivalent,
  from a private HID event source and paced like a person, so drags and clicks
  land.
- **Refuse** — a non-settable value, an unsupported action, a stale index, a
  moved window or a denied app produces an error an agent can act on, not a
  success message.
- **Server** — newline-delimited JSON-RPC over stdio, serving revisions
  `2025-06-18`, `2025-03-26` and `2024-11-05` by echoing the client's, tools
  only. Tool failures are `isError` results, not protocol errors.

## Layout

```text
src/main.rs        CLI: mcp, call, tools, doctor, install, apps
src/mcp.rs         stdio JSON-RPC server and version negotiation
src/tools.rs       the tool schemas, dispatch and result shape
src/install.rs     per-client config writing (JSON and TOML) and reserved names
src/policy.rs      the app allow/deny policy
src/runtime.rs     platform selection; macOS implementation, explicit stub elsewhere
src/macos/ax.rs    accessibility elements, attributes, actions, ranges
src/macos/tree.rs  snapshot rendering and window binding
src/macos/apps.rs  running apps and windows via the window server
src/macos/input.rs synthetic pointer, keyboard and scroll events
src/macos/screen.rs screenshot capture
src/macos/cf.rs    Core Foundation bridging
tests/cli.rs       binary-level checks: CLI, MCP handshake, installers, policy, UTF-8
install.sh         the curl installer the releases serve
scripts/           set-version.sh, which a release tag stamps into the crate
.github/           CI, and the two release workflows: the drafter drafts the
                   next release, release.yml attaches its archives on publish
docs/              install, clients, comparison, issue audit
```

## Documentation

- [docs/install.md](docs/install.md) — the installer, the release assets, the
  environment overrides, and building from source.
- [docs/tools.md](docs/tools.md) — every tool: parameters, defaults, reply
  shape, failure modes.
- [docs/architecture.md](docs/architecture.md) — modules, request flow, snapshot
  and index lifecycle, FFI ownership, MCP behaviour.
- [docs/configuration.md](docs/configuration.md) — policy file, per-client config
  paths, defaults, permissions, exit codes.
- [docs/clients.md](docs/clients.md) — install and use it from each MCP client.
- [docs/oxide-helper.md](docs/oxide-helper.md) — optional Oxide native app,
  separate permission setup, activity/preview controls and parity limits.
- [docs/troubleshooting.md](docs/troubleshooting.md) — symptoms, causes and
  fixes, including the accessibility error codes.
- [docs/development.md](docs/development.md) — tests, adding a tool, a client or
  a platform backend, and the manual smoke checklist.
- [docs/comparison.md](docs/comparison.md) — how it differs from
  `open-computer-use` and Codex Computer Use, including where the others win.
- [docs/issues.md](docs/issues.md) — an audit against the
  `open-computer-use` issue tracker: what is designed out, what is mitigated,
  what is still open.

## Limits

- macOS only. Other platforms compile to a build whose tools return an explicit
  "not implemented here" error.
- Typing and pointer events need the target app in front; `set_value` and
  `AXPress` are the paths that work in the background.
- Native `<input type="date">` segments and web content behind a cross-origin
  picker are not reachable by AX alone.
- No audit log or automatic screenshot after every action. Request
  `screenshot` or `include_screenshot` for visual verification. Oxide can keep
  the last returned image visible; it does not stream the screen.
- The optional helper does not add isolated background input, Codex-style
  per-app approval/settings UX, or automatic unlock. See the
  [integration and parity limits](docs/oxide-helper.md).

## License

MIT.
