# Troubleshooting

Start with three commands. They separate setup, discovery and reading, which is
where almost every problem lives.

```sh
oxide-computer-use doctor                     # permissions, policy, versions
oxide-computer-use verify                     # each native path, with the evidence
oxide-computer-use call list_apps             # what the window server reports
oxide-computer-use call get_app_state --args '{"app":"TextEdit","max_tree_nodes":200}'
```

`verify` is the newest of these and the most specific: one line per path — the
accessibility tree, a capture, a bundle identity, the read bound, the takeover
tap, background typing — with `ok`, `MISSING` or `skipped` and the reason.
`--json` is the same report for a script, `--app` chooses what to read, and
`--input` additionally types into it (it writes text, so point it at a scratch
document). A `MISSING` permission is granted to the app that launched the server,
never by the server.

`doctor --json` gives the same information for a script:

```json
{
  "version": "0.1.0",
  "macos": "15.3",
  "accessibility": true,
  "screen_recording": true,
  "allowed_apps": [],
  "denied_apps": ["1Password"],
  "config": "/Users/you/.config/oxide-computer-use/config.json"
}
```

## Permissions

### A client says "Connected" but every tool fails

The MCP handshake needs no permission; the first accessibility call does. Run
the `permissions` MCP tool from the failing client. It checks the same process
and reports the observed launching app and Settings guidance. An `oxide-computer-use
doctor` run in a separate terminal measures a different host.

Grant it to the **responsible app**, not to this binary: your terminal if you run
the server from a shell, or the app that launches it. macOS records the grant
against that app's bundle identity.

### System Settings shows Accessibility on, but `doctor` says missing

The grant belongs to a different process, or to a stale copy:

- launched from a different terminal than the one you granted;
- launched by an IDE's integrated terminal (grant the IDE);
- launched by `launchd`, `cron`, a remote `ssh` session or a daemon, where no
  app is responsible and the prompt can never appear — run it from a logged-in
  GUI session instead;
- the binary moved or was rebuilt after the entry was added: remove the entry and
  add it again.

`ps -o comm= -p $PPID` shows the parent that needs the grant.

### The permission was granted after the server started

Accessibility is checked on every tool call, so it takes effect immediately.
Screen Recording is cached by macOS per process: restart the server, and if the
capture still fails, restart the app that owns it.

### `doctor --prompt` shows no prompt

macOS shows a given app's prompt once. If it was dismissed, open System Settings
→ Privacy & Security yourself; the prompt is a convenience, not a control.

## Finding the app

| Message | Cause | What to do |
| --- | --- | --- |
| ``no app matches `X` `` | Name/bundle/path typo, no launchable app, or it is a hidden helper | `list_apps`; use its `id` or `path`; `list_apps {"include_helpers":true}` |
| The app is running but not listed | Window smaller than 120×80, layer above 0, minimized, or a window-server agent | `include_helpers`, or target it by pid |
| ``window N does not exist`` / ``no window title contains …`` | Wrong index or title | Read the header of `get_app_state`: it lists the exact `window` values |
| Clicks land in the wrong window | The app has several windows and no `window` was given | Pass `window` (index or title substring); indexes are then bound to it |

## Element indexes

| Message | Cause | What to do |
| --- | --- | --- |
| ``element_index N is out of range`` | The snapshot was replaced, or the index came from an earlier read | Call `get_app_state` or `query` again and use its numbers |
| ``the cached elements belong to `A`, not `B` `` | Acting on one app with another app's indexes | Read the state of the app you are acting on |
| ``is no longer in the window the snapshot bound it to`` | The element moved, a window closed, or a tab/window switched | Read again; do not retry the same index |
| ``could not read X's accessibility tree`` | The app stopped answering within the two-second bound: it is busy, showing a modal sheet, or hung | The failed read did not replace the newest snapshot, so indexes from the reply before it still address that app. Retry once the app answers; do not renumber by guessing |
| ``the element changed since the snapshot`` | The app rebuilt its tree between read and act | Same: read, then act |
| ``has no on-screen frame`` | The element reports no position (some groups and virtual nodes) | Use `include_frames` with a neighbouring element, or `click` with `x`/`y` |

Indexes never survive a snapshot: the number is a position in that reply, not a
stable identifier.

## Actions that do not do what you expect

| Symptom or message | Cause | What to do |
| --- | --- | --- |
| ``reports AXValue as not settable`` | The control is read-only or text is edited through another attribute | `click` it, then `type_text`; or `press_key` per segment (date fields) |
| ``still reads … after writing`` | The control accepted the write and reverted it, or needs a commit action | `type_text` instead of `set_value`, then `press_key` Return or Tab |
| ``does not support AX…`` | The named action is not on that element | The message lists what it does support; `click` is usually `AXPress` |
| `click` succeeds but nothing changes | The element is `· disabled`, or its press action is a no-op | `include_frames`, then click its coordinates; or use `press_key` |
| Typing goes nowhere | The target is not the focused control, or another app is frontmost | Pass `app` (and `element_index` when known) so it is activated and focused |
| A drag moves nothing | Start point is not on a draggable area, or the window is not active | `include_frames` to pick a title-bar point; activate the app first with any action that takes `app` |
| Scrolling does nothing | Pointer is over a non-scrollable area | Pass `element_index` or `x`/`y` over the scrollable view |
| `unknown key` | Key name not recognised | Use the names from `docs/tools.md` (`Return`, `Tab`, `Page_Down`, `F5`, a single character) or `cmd+s` inline modifiers |

## macOS accessibility errors

The server prefixes these with the call that failed. A timed-out read reads
`reading AXChildren failed: kAXErrorCannotComplete (-25204)` followed by the
reason: the app did not answer within two seconds, so it is busy, showing a modal
sheet, or unresponsive.

| Code | Name | Usually means | What to do |
| --- | --- | --- | --- |
| 0 | success | — | — |
| -25200 | `kAXErrorFailure` | The app failed to answer | Retry after reading state again |
| -25201 | `kAXErrorIllegalArgument` | The server passed something the app rejected | Report it with the element's role |
| -25202 | `kAXErrorInvalidUIElement` | The element is gone | `get_app_state` again |
| -25203 | `kAXErrorInvalidUIElementObserver` | Observation handle is invalid | Does not apply to this server |
| -25204 | `kAXErrorCannotComplete` | App is busy, modal, or hung | Wait, dismiss the sheet, then read state. Reads are bounded at two seconds, so this arrives as a failed call rather than a partial tree |
| -25205 | `kAXErrorAttributeUnsupported` | The element has no such attribute | Use another attribute or a different tool |
| -25206 | `kAXErrorActionUnsupported` | The element has no such action | `perform_secondary_action` lists supported actions |
| -25207 | `kAXErrorNotificationUnsupported` | Notifications unsupported | Does not apply |
| -25208 | `kAXErrorNotImplemented` | The app does not implement this | Try the pointer path |
| -25209 | `kAXErrorNotificationAlreadyRegistered` | Notifications already registered | Does not apply |
| -25210 | `kAXErrorNotificationNotRegistered` | Notifications not registered | Does not apply |
| -25211 | `kAXErrorAPIDisabled` | Accessibility permission missing | `doctor`, then grant it |
| -25212 | `kAXErrorNoValue` | No value for that attribute | Element is stale or empty; read again |
| -25213 | `kAXErrorParameterizedAttributeUnsupported` | Range-based text reads unsupported | `select_text` cannot work on that element |
| -25214 | `kAXErrorNotEnoughPrecision` | Coordinate conversion lost precision | Use `include_frames` coordinates |

`kAXErrorCannotComplete` (`-25204`) and `kAXErrorAPIDisabled` (`-25211`) are the
two codes that fail a read: the first is the app missing the two-second bound,
the second is Accessibility going away mid-call. Every other code means this
element has nothing to give, and the tree is walked past it. A `stopped with an
internal error` message is a bug in this server rather than in your setup: the
call was answered as an error, the connection still works, and the line plus the
call that produced it is what a report needs.

## Screen capture

| Message | Cause | What to do |
| --- | --- | --- |
| ``screencapture produced an empty image`` | Screen Recording not granted | Grant it to the responsible app, then restart |
| ``screencapture failed (…)`` | Window in another Space, minimized, or gone | Capture the whole screen, or bring the window forward first |
| ``has no on-screen window to capture`` | No usable window for that app | Activate it with an action that takes `app` |
| A covered window returns the covering content | The platform capture composited the screen | Capture the specific window with `{"app": …}`, or bring it forward |
| `include_screenshot` says unavailable | Capture failed, tree still returned | Fix capture separately; the observation is not lost |

## Policy and approvals

| Symptom | Cause | What to do |
| --- | --- | --- |
| ``is on the deny list`` | `denied_apps` matched | `oxide-computer-use apps allow "X"` if intended; the deny list otherwise wins |
| ``is not on the allow list`` | An allow list exists and lacks the app | `oxide-computer-use apps allow "X"` |
| The policy changed but the server still refuses | The policy is read when the server starts | Restart the session or reconnect the client |
| Nothing prompts before an action | The client, not the server, owns approvals | Configure approval rules in the client; this server only marks read-only tools |

## Install

| Symptom | Cause | What to do |
| --- | --- | --- |
| `install.sh` says it found no release manifest | No release published yet, or `OXIDE_COMPUTER_USE_REPO` points somewhere without one | Build from the checkout ([docs/install.md](install.md)), or pass `OXIDE_COMPUTER_USE_VERSION=v<version>` for a release that exists |
| The release page has the notes but no archives yet | The draft was just published; `release.yml` attaches the assets once it has built them | Wait for the workflow to go green; the installer's manifest message is the symptom until then |
| Checksum verification failed | Truncated or replaced download | Rerun the installer; if it repeats, download the archive from the release page and verify it by hand |
| The downloaded binary did not run | Archive for the other architecture, or a quarantine flag on the file | `uname -m` to confirm the platform; `xattr -d com.apple.quarantine <binary>` if it was downloaded by a browser rather than by the installer |
| `oxide-computer-use: command not found` after installing | `~/.local/bin` is not on `PATH` | The installer warns about this; add the directory, then reopen the terminal the agent runs in |

## Clients

| Symptom | Cause | What to do |
| --- | --- | --- |
| Claude Code ignores the server | `computer-use` is a reserved server name there | Register `oxide-computer-use` (`install --client claude` does), or another `--name` |
| Codex shows two Computer Use tool sets | Codex ships its own Computer Use plugin; this server registers beside it | Expected. The plugin's tools are in Codex's `computer_use` namespace; this server is `oxide-computer-use` and only its calls obey this project's app policy |
| `install --client codex --name computer-use` is refused | Codex's bundled Computer Use plugin owns that name, as it does `computer_use`, `node_repl`, `cua_repl` and `codex_apps` | Use the default `oxide-computer-use`, or another name Codex does not already use |
| `install --client codex` warns that `mcp_servers.computer-use` is already there | An entry from an older install sits under the name Codex's plugin owns | Keep `oxide-computer-use` and delete that table: two entries, one of them shadowed, is what the warning is about |
| A client cannot parse the config after an edit | The file was invalid before `install` ran; it refuses to rewrite it | Fix the JSON/TOML, rerun `install` |
| The client never starts the server | Wrong absolute path, or the binary is not executable | `install --print` to see the exact command, then run it by hand |
| Tools appear but calls fail with "Connection closed" | The server exited: bad launch command, or a panic | Run the same command in a terminal and send one request; a clean server prints JSON and nothing else |
| Non-ASCII text looks mangled | The client's console encoding, not the transport | The server writes UTF-8 JSON; evidence in `tests/cli.rs` |

## Context and performance

- Large tree: `query` first, or lower `max_tree_nodes`; the reply states when it
  truncated.
- Long text values: leave `text_limit` at 200, raise it for one field only.
- Huge replies: state diffs and `max_output_chars` cap them; set
  `include_screenshot: false` for a text-only observation.
- Many steps: prefer `--calls` in the CLI, or one `get_app_state` followed by
  several actions in a session, over re-reading the tree each time.
- A single slow call: accessibility calls are synchronous and blocking; an app
  that is busy blocks the next request until it answers or times out.

## Collecting evidence for a report

```sh
oxide-computer-use doctor --json
oxide-computer-use tools
oxide-computer-use call list_apps
oxide-computer-use call get_app_state --args '{"app":"<app>","max_tree_nodes":400,"include_frames":true}'
```

Include the exact `element_index` or `window` used, and the reply verbatim: the
messages carry the role, the index, the app and the accessibility error code,
which is what makes a failure reproducible.

## Oxide Desktop previews

A screenshot needs Screen Recording; tree inspection and AX actions need
Accessibility. For direct stdio, grant the hosting Oxide.app in System Settings
(use + if it is absent). For the optional [native helper](oxide-helper.md), grant
Oxide Computer Use instead. Existing Oxide.app grants do not cover that helper;
reconnect its MCP server after changing grants. The observed
launching app in `permissions` is a diagnostic hint; macOS determines the grant
recipient. Never automate a grant or read the private TCC database to diagnose it.
The setup order, the two approval gates and the controls Oxide owns are in the
README's [Use it with Oxide Desktop](../README.md#use-it-with-oxide-desktop).

With an Oxide build that supports tool image events, `get_app_state` already
includes the app-window screenshot; `screenshot` requests one explicitly. The
tool card shows a clickable thumbnail that opens the captured image. Newer Desktop builds also retain the
last snapshot in the details dock and expose a mouse activity control/dialog,
including when the dock has no room. A later tree-only result keeps the previous
image and its original app label. A connected MCP server alone does not mean a
GUI action is running. No continuous preview stream is provided. A textual `[image content]` with no image on older clients can mean
the client discarded MCP image parts; updating only this server cannot fix that.


### Helper connects but control is unavailable

A successful MCP handshake and menu-bar cursor do not require Accessibility or
Screen Recording. Use **Check permissions** in the helper and `permissions` in
the MCP session; only the user can approve OS grants. Move the helper to its
stable location before granting access. Rebuilding an ad-hoc bundle can invalidate
grants. If startup reports an IPC/listen error, check whether the launching
sandbox forbids Unix sockets or LaunchServices before assuming a permission issue.
**Stop task** cancels the Oxide turn; the helper's stop menu disconnects its MCP
session. Neither reverses an action that already completed. This development
helper is not a signed/notarized installer or an input-isolated desktop.
