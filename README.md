# oxide-computer-use

Computer Use as a stdio MCP server for any agent on macOS — Oxide, Claude Code,
Codex CLI, Cursor, Claude Desktop. It lists launchable apps, reads an app's
accessibility tree, and clicks, types, pastes, selects, scrolls and drags inside
it. Installed apps are discoverable by bundle id, name or path and launch when
first addressed.

The name: reins steer without letting go. The agent drives the apps, while the
[app policy](docs/configuration.md) and the permission grants stay in your hands.

The idea matches [open-computer-use](https://github.com/iFurySt/open-computer-use);
the core implementation is a single Rust binary with
[one tool list](src/tools.rs) and explicit action failures. Oxide also provides
an opt-in [native companion app](docs/oxide-helper.md) in its Desktop repository.
[Use it with Oxide Desktop](#use-it-with-oxide-desktop) is the setup that matters
when Oxide is the host, and
[what is different from Codex Computer Use](#what-is-different-from-codex-computer-use)
says which half of that product this server is.

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
- Screen Recording permission for the app-state screenshot and `screenshot`.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/jaysonwu991/oxide-computer-use/main/install.sh | sh
oxide-computer-use doctor
```

The installer picks the Apple Silicon or Intel archive from the
[latest release](https://github.com/jaysonwu991/oxide-computer-use/releases), checks
its SHA-256, and installs `oxide-computer-use` into `~/.local/bin`. Beside the binary it
writes `.oxide-computer-use-install`, naming the release, the version, the platform and
the installed binary's SHA-256; `oxide-computer-use doctor` reports that marker and the
build it is running.
`OXIDE_COMPUTER_USE_INSTALL_DIR`, `OXIDE_COMPUTER_USE_VERSION` and `OXIDE_COMPUTER_USE_REPO`
override where it goes and what it installs. `sh install.sh --check` compares the
installed copy with the latest release: exit 0 current, 1 update available,
2 cannot tell. There is no Linux or Windows
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

It registers as `oxide-computer-use` for every supported client. That name stays
distinct from `computer-use`, which Claude Code reserves and Codex's bundled
Computer Use plugin owns, so Oxide, Codex and Claude Code can all load this
server alongside their built-ins. `--name` picks another unreserved name.
Details and the equivalent per-client commands are in
[docs/clients.md](docs/clients.md).

## Use it with Oxide Desktop

Oxide hosts this server under its own bundle, so the permission grant, the
approval prompt and the preview each belong to a different party than they do in
a terminal setup. From a terminal on the same machine:

```sh
oxide-computer-use install --client oxide      # writes ~/.oxide/mcp.json as `oxide-computer-use`
oxide mcp list                          # Oxide should show `oxide-computer-use` connected
```

Then, in this order:

1. **Grant to Oxide, not to the binary.** macOS records Accessibility and Screen
   Recording against the app that launches the server — `/Applications/Oxide.app`
   when Oxide hosts it. Quit and reopen Oxide after changing Screen Recording.
2. **Check from the connection that is failing.** The `permissions` tool runs
   inside the process Oxide hosts and reports the launching app it observes;
   `oxide-computer-use doctor` in a terminal measures that terminal instead, and the two
   can disagree in both directions.
3. **Two gates, and only one of them is per app.** Oxide's own approvals decide
   whether a tool call may run; the [app policy](#check-permissions-and-policy)
   below decides which apps this server will drive, and is re-read on every call.
4. **Oxide's own window is refused as a target**, and so are terminal emulators,
   until the user allows it (`apps self-automation allow`, `apps terminals allow`).
   Those are the user's decisions, not a step to work around.
5. **Preview and stop belong to Oxide.** Newer builds show the last image this
   server returned in the details dock, a mouse glyph while a call runs, and
   **Stop task**. The image comes from the default `get_app_state` screenshot,
   `screenshot`, or a read of `computer-use://screen` — on request, never on a
   timer — and stopping a turn does not undo a click or a write that already ran.
6. **A separate app identity is available.** The opt-in
   [native helper](docs/oxide-helper.md) hosts this same runtime with its own
   permission entry and a menu-bar stop; `install --client oxide` registers the
   standalone runtime and does not switch to it.

## Check permissions and policy

```sh
oxide-computer-use doctor          # permissions, app policy, the build it runs
oxide-computer-use doctor --prompt # ask macOS to show the permission prompts
oxide-computer-use apps list
oxide-computer-use apps deny "1Password"
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
| `list_apps` | Launchable apps as JSON, addressed by bundle id, name or path; `isRunning` and the policy marker show the current state. |
| `app_policy` | Read or change Always allow, one-call approval, deny/revoke and fallback app access for settings UIs. |
| `get_app_state` | Launches the app if needed, waits briefly after actions, then returns indexed accessibility state, optional first-use app guidance, a window screenshot by default, and a diff unless `disableDiff` is true. |
| `query` | Search that tree by text and/or role without printing all of it. Also refreshes the indexes. |
| `click` | `AXPress` for a plain left click when available; otherwise a pointer click with middle/right buttons, click count, held modifier, duration, or app-window `x`/`y`. |
| `perform_secondary_action` | A named AX action such as `AXShowMenu`; an action the element does not support is refused with the list it does support. |
| `set_value` | Writes `AXValue` only when the element reports it settable, then reads it back. |
| `select_text` | Selects text with prefix/suffix disambiguation, or places the cursor before/after it, using UTF-16 Cocoa ranges. |
| `type_text` | Types into the focused control, a newline as `Return`; activates the app first, or `background: true` to one process. |
| `press_key` | A broader xdotool-style key/chord surface, including modifiers, keypad, navigation and punctuation names, with optional hold duration and background delivery. |
| `scroll` | Up/down/left/right wheel scrolling by `pages`, at an element or app-window coordinate. |
| `drag` | Paced HID drag between app-window screenshot coordinates, stopped by a person's input. |
| `paste` | Pastes text, Markdown or HTML and restores the previous clipboard contents. |
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
src/macos/apps.rs  launchable/running apps, bundle ids and windows
src/macos/input.rs synthetic pointer, keyboard and scroll events
src/macos/instructions.rs first-use guidance for selected accessibility-heavy apps
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

## What is different from Codex Computer Use

Codex Computer Use is a product with a client and a private runtime; this is a
stdio server with no window of its own. Part of the comparison is therefore about
which side a feature belongs to rather than whether it exists. The per-item
record — with commands, evidence and what is still unverified — is
[docs/parity.md](docs/parity.md); [docs/comparison.md](docs/comparison.md) covers
`open-computer-use` as well.

**Brought level in this runtime**

- **Per-app approvals** — allow, deny, one call, revoke, and a default for an app
  nobody decided about, pinned to the bundle identity, enforced before any
  accessibility call and re-read on every call. The hosting app and terminal
  emulators are refused unless a rule allows them.
- **Background keyboard input** — `type_text` and `press_key` with
  `background: true` post to one process without activating it, and the reply asks
  for verification, because an app may ignore delivered keys. Pointer events still
  move the real cursor: CoreGraphics has no per-process pointer path.
- **Cursor takeover** — a paced drag or long `type_text` stops when input arrives
  from anyone but this process and answers `user takeover`.
- **A screen preview** — `computer-use://screen` captures on read, and a
  subscriber is told after any call that could have changed it.
- **Bounded, honest failure** — a read an app never answers fails within two
  seconds and leaves the previous snapshot addressable, and a call that breaks
  internally is an `isError` result rather than the end of the session.

**Client windows, so not here**

- The system-wide activity overlay, the settings pane, the built-in browser and
  native `@App` selection are windows in Oxide or Codex. This server exposes the
  surface such a pane would call — `apps list|allow|deny|revoke|grant-once|policy`,
  `doctor --json`, `permissions` — and drives a browser through accessibility like
  any other app.

**Absent, with the reason it stays absent**

- **Unlock, covering the display and relocking** — privileged session APIs and a
  password this runtime will not ask for or store.
- **A signed, notarized download** — needs a Developer ID certificate. Releases
  are unsigned archives with a SHA-256, a manifest, an install marker and
  `install.sh --check`.
- **A Windows backend** — a second implementation, not a build flag. Other
  platforms build the same tool list over a stub whose every tool returns an
  explicit error.
- **Pushed frames** — a stream would be bytes no client in this setup reads. The
  preview is a resource read plus an update notification.

## Limits

- macOS only: another platform builds the same tool list over a stub that returns
  an explicit error. There is no Windows backend, and no desktop of its own — the
  real pointer moves during a drag, pointer input activates the target app, and a
  killed server is a new server on the client's next start.
- Pointer input needs the target app in front. `set_value` and `AXPress` work in
  the background, and an app that keeps its own key focus may ignore background
  keys, so the reply asks for verification.
- `oxide-computer-use verify` proves what the machine can do (tree, capture, identity,
  read bound, takeover tap, background typing) and exits nonzero when a path is
  missing; it does not grant a permission.
- The preview is a resource, not a stream: `resources/read` on
  `computer-use://screen` captures on demand, and a subscriber is told when a
  call may have changed the screen. Nothing is pushed on a timer.
- Native `<input type="date">` segments and web content behind a cross-origin
  picker are not reachable by AX alone.
- No audit log or automatic screenshot after every action. `get_app_state`
  captures the current app window by default; action replies stay text-only.

## License

MIT.
