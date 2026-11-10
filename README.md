# oxide-reins

Computer Use as a stdio MCP server for any agent on macOS — Oxide, Claude Code,
Codex CLI, Cursor, Claude Desktop. It lists the running apps, reads an app's
accessibility tree, and clicks, types, selects, scrolls and drags inside it.

The name: reins steer without letting go. The agent drives the apps, while the
[app policy](docs/configuration.md) and the permission grants stay in your hands.

The idea matches [open-computer-use](https://github.com/iFurySt/open-computer-use);
the core implementation is a single Rust binary with
[one tool list](src/tools.rs) and explicit action failures. Oxide also provides
an opt-in [native companion app](docs/oxide-helper.md) in its Desktop repository.

```text
agent (Oxide / Claude Code / Codex / Cursor) ──stdio JSON-RPC──▶ oxide-reins
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
curl -fsSL https://raw.githubusercontent.com/jaysonwu991/oxide-reins/main/install.sh | sh
oxide-reins doctor
```

The installer picks the Apple Silicon or Intel archive from the
[latest release](https://github.com/jaysonwu991/oxide-reins/releases), checks
its SHA-256, and installs `oxide-reins` into `~/.local/bin`. Beside the binary it
writes `.oxide-reins-install`, naming the release, the version, the platform and
the installed binary's SHA-256; `oxide-reins doctor` reports that marker and the
build it is running.
`OXIDE_REINS_INSTALL_DIR`, `OXIDE_REINS_VERSION` and `OXIDE_REINS_REPO`
override where it goes and what it installs. `sh install.sh --check` compares the
installed copy with the latest release: exit 0 current, 1 update available,
2 cannot tell. There is no Linux or Windows
archive: the tools drive macOS APIs, so another platform builds to explicit
"not implemented" errors — see [docs/install.md](docs/install.md).

## Build from source

```sh
cargo build --release
install -m 755 target/release/oxide-reins ~/.local/bin/oxide-reins
```

## Connect a client

```sh
oxide-reins install --client oxide     # ~/.oxide/mcp.json
oxide-reins install --client claude    # ~/.claude.json
oxide-reins install --client codex     # ~/.codex/config.toml
oxide-reins install --client cursor    # ~/.cursor/mcp.json
oxide-reins install --client desktop   # Claude Desktop
oxide-reins install --print            # the entry, for any other client
```

It registers as `oxide-reins`, except for the clients that already ship their
own Computer Use: Claude Code reserves `computer-use` as that name and skips a server registered
under it, and Codex's bundled Computer Use plugin owns it, so `--client claude`
and `--client codex` use `oxide-reins-local` and sit beside the built-in rather
than over it. `--name` picks another name where a client has none reserved.
Details and the equivalent per-client commands are in
[docs/clients.md](docs/clients.md).

## Check permissions and policy

```sh
oxide-reins doctor          # permissions, app policy, the build it runs
oxide-reins doctor --prompt # ask macOS to show the permission prompts
oxide-reins apps list
oxide-reins apps deny "1Password"
```

An approval is per app and can name the bundle identity it was given for, so a
different app answering to the same name is refused; the app hosting this server
and terminal emulators are refused unless `apps self-automation allow` or
`apps terminals allow` says otherwise. Each decision is re-read on
every call: `apps allow` approves from now on, `apps grant-once` approves one
call, `apps revoke` withdraws every decision about one app, and `apps policy deny`
refuses an app nobody has decided about. See
[docs/configuration.md](docs/configuration.md#app-policy).

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
| `type_text` | Types into the focused control, a newline as `Return`; activates the app first, or `background: true` to one process. |
| `press_key` | One key with optional `cmd`/`shift`/`alt`/`ctrl` modifiers, foreground or `background`; `\n` and `\t` are read as `Return` and `Tab`. |
| `scroll` | Wheel events at the frontmost window, an element center, or `x`/`y`; needs no snapshot. |
| `drag` | Paced HID drag from an element center or `from_x`/`from_y` to `to_x`/`to_y`, stopped by a person's input. |
| `screenshot` | A PNG of the screen or of one app's window. |

Element indexes are per-process state: `get_app_state` and `query` replace the
snapshot, each index is bound to the `AXWindow` it came from, and an index from
an old snapshot, the wrong app or the wrong window is refused by name instead of
being applied to whatever is there now.

## Call it without an agent

```sh
oxide-reins call list_apps
oxide-reins call get_app_state --args '{"app":"TextEdit","window":0}'
oxide-reins call click --args '{"app":"TextEdit","element_index":12}'
```

A sequence shares one process and one snapshot state:

```sh
oxide-reins call --calls '[
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
  `2025-06-18`, `2025-03-26` and `2024-11-05` by echoing the client's: tools, and
  one resource (`computer-use://screen`) whose reads capture on demand and whose
  subscribers are told when a call may have changed the screen. Tool failures are
  `isError` results, not protocol errors.

## Layout

```text
src/main.rs        CLI: mcp, call, tools, doctor, install, apps
src/mcp.rs         stdio JSON-RPC server and version negotiation
src/tools.rs       the tool schemas, dispatch and result shape
src/install.rs     per-client config writing (JSON and TOML) and reserved names
src/policy.rs      per-app approvals, bundle identities and the default policy
src/runtime.rs     platform selection; macOS implementation, explicit stub elsewhere
src/verify.rs      the verify command: one native check per path, with evidence
src/macos/ax.rs    accessibility elements, attributes, actions, ranges
src/macos/tree.rs  snapshot rendering and window binding
src/macos/apps.rs  running apps and windows via the window server
src/macos/input.rs synthetic pointer, keyboard and scroll events
src/macos/takeover.rs a listen-only event tap that yields to a person's input
src/macos/screen.rs screenshot capture
src/macos/cf.rs    Core Foundation bridging
tests/cli.rs       binary-level checks: CLI, MCP handshake, installers, policy, resources, UTF-8
tests/install.sh   installer update check against a staged release directory
install.sh         the curl installer the releases serve
scripts/           set-version.sh, which a release tag stamps into the crate
.github/           CI, and the two release workflows: the drafter drafts the
                   next release, release.yml attaches its archives on publish
docs/              install, clients, comparison, parity, issue audit
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
- [docs/parity.md](docs/parity.md) — the Codex Computer Use parity follow-ups
  from the Oxide Desktop review: what this runtime now does, what stays in the
  client, and what is still unverified.

## Limits

- macOS only. Other platforms compile to a build whose tools return an explicit
  "not implemented here" error. There is no Windows backend.
- Pointer events need the target app in front. `set_value` and `AXPress` work in
  the background, and `type_text`/`press_key` with `background: true` deliver keys
  to one process without activating it — an app that keeps its own key focus may
  ignore them, so the reply asks for verification.
- `oxide-reins verify` proves what the machine can do (tree, capture, identity,
  takeover tap, background typing) and exits nonzero when a path is missing; it
  does not grant a permission.
- The preview is a resource, not a stream: `resources/read` on
  `computer-use://screen` captures on demand, and a subscriber is told when a
  call may have changed the screen. Nothing is pushed on a timer.
- Native `<input type="date">` segments and web content behind a cross-origin
  picker are not reachable by AX alone.
- No audit log or automatic screenshot after every action. Request
  `screenshot` or `include_screenshot` for visual verification. Oxide can keep
  the last returned image visible; the runtime does not stream the screen.
- The app policy and its approval commands are the server-side half of Codex's
  per-app Allow / Always allow / revoke model, and the hosting app is refused
  unless `apps self-automation allow` says otherwise. A client-side settings UI,
  the built-in browser, native @App selection, and automatic unlock/display
  covering are not part of this repository.
- The optional helper adds no isolated background input, streaming preview or
  system-wide overlay. See the [parity record](docs/parity.md) and the
  [integration limits](docs/oxide-helper.md).

## License

MIT.
