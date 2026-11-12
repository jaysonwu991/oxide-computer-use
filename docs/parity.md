# Codex Computer Use parity: the follow-ups, and where each one stands

Oxide Desktop [PR #292](https://github.com/jaysonwu991/oxide/pull/292) shipped a
native helper and its review listed what was still missing. The companion
documentation
[PR #3](https://github.com/jaysonwu991/computer-use/pull/3) repeated the same
list for the standalone server. This page takes that list — seven bullets — and
says for each one what this repository now does, what belongs to the client
instead, and what is still unverified or absent. Nothing here is a claim about
Oxide's window: the client half lives in the Oxide checkout.

The reference product is Codex/ChatGPT Computer Use as its
[public documentation](https://learn.chatgpt.com/docs/computer-use) describes it;
its private runtime is in neither repository. The README carries the short
version for someone deciding whether to use this server —
[what is different from Codex Computer Use](../README.md#what-is-different-from-codex-computer-use)
and [how to work with Oxide Desktop](../README.md#use-it-with-oxide-desktop).

## 1. Input and takeover

*Isolated background input, cursor takeover arbitration, a system-wide activity
overlay.*

- **Takeover arbitration: implemented here.** A listen-only `CGEventTap`
  (`src/macos/takeover.rs`) is installed before a paced drag or a multi-line
  `type_text`. Every event this runtime posts carries its own process id as its
  source and a person's hardware input does not, so the callback can tell them
  apart. The first foreign keystroke, pointer movement or scroll releases the
  mouse button at the last position and fails the call with `user takeover`
  instead of fighting the pointer. Without the Accessibility grant the tap cannot
  be installed; the gesture then runs unarbitrated, which the tool reference
  states.
- **Background keyboard input: implemented here.** `type_text` and `press_key`
  take `background`, which posts the same events to one process with
  `CGEventPostToPid` instead of the HID tap, so the app is not activated and the
  person keeps their focus. The reply names the delivery path and asks for
  verification, because an app that routes keys through its own window or focus
  state can ignore input delivered this way. Pointer input has no background
  path: a click is a click on the desktop.
- **Still absent:** a system-wide cursor overlay, which is a window and therefore
  the client's; and a verified answer to "which apps honour posted-to-pid keys",
  which needs a granted machine and a list of apps to test.

## 2. Preview

*Only tool-returned snapshots; continuous capture/streaming absent.*

- **Implemented here:** the `computer-use://screen` MCP resource. `resources/read`
  captures on demand and returns a PNG block, so a client can render a current
  view without spending a tool call, and `resources/subscribe` gets
  `notifications/resources/updated` after every call that could have changed the
  screen — any action tool, and reads that returned an image. A client that
  subscribes therefore knows when re-reading is worth it instead of guessing.
- **Absent:** frames pushed without a client request. A stdio server answers; it
  cannot start a conversation, and a timer that emits captures nobody asked for
  would be a stream the client never reads. The cadence of a "live" preview is
  the client's choice — read as often as the pane is visible.
- Tool-returned observations now match the current macOS window API shape:
  `get_app_state` attaches one app-window screenshot by default (and returns a
  diff after the first view), while the standalone `screenshot` tool remains
  available for an explicit full-screen or app-window capture.

## 3. Approvals and settings

*Codex-style per-app Allow / Always allow / revoke UI, app identity validation,
policy enforcement.*

- **Approvals: implemented here.** `src/policy.rs` now holds four decisions per
  app — allow, deny, allow-once, and the default for an app nobody decided about
  — instead of a single allow/deny pair:
  - `apps allow` approves from now on and records the running app's bundle
    identifier and bundle path, read with `proc_pidpath` and
    `CFBundleGetIdentifier`;
  - `apps grant-once` approves exactly one call, and the call that uses it
    removes it from the file;
  - `apps deny` refuses whatever the allow list says;
  - `apps revoke` withdraws every decision about one app, a deny included;
  - `apps policy allow|deny` sets the default for an app with no decision.
- **Identity validation: implemented here.** A recorded identifier must match the
  identity of the process that was resolved, or the call is refused naming both
  identifiers and both paths. That is what stops a different app from inheriting
  an approval by taking its name.
- **Enforcement: implemented here.** The check runs after resolution and before
  any accessibility or input call, and the file is re-read on every call, so a
  decision made in a terminal reaches a running `mcp` server. A policy file that
  does not parse refuses every app rather than falling back to "unlisted, so
  allowed".
- **Self-automation: implemented here.** The bundle that launched the server is
  refused by default, by bundle path, with `apps self-automation allow` in the
  refusal. That is narrower than Codex's documented restriction: shells stay
  reachable unless a shell is the host.
- **Client side:** the settings UI and Oxide's own tool approvals. A stdio server
  cannot prompt; its half is the refusal message that names the exact command,
  and it stays a separate gate from the client's. Everything above is reachable
  from a terminal (`apps list`, `apps allow|grant-once|deny|revoke|policy|
  self-automation`), which is the surface a settings pane would call.

## 4. Product flows

*Built-in browser integration, native @App selection/settings, automatic
unlock/display covering/relock, terminal and self-automation policy equivalence.*

- None of this is a runtime feature. The server drives a browser through
  accessibility like any other app, and `list_apps`/`get_app_state` are the whole
  surface.
- **Policy equivalence: implemented for the two rules Codex documents.** A
  terminal — by bundle identity, over nine known emulators — is refused before any
  accessibility call, and so is the hosting app, unless `apps terminals allow` or
  `apps self-automation allow` says otherwise. Both refusals name the bundle and
  the command. What is not equivalent is enforcement inside the client: Oxide's own
  tool approvals remain a separate gate.
- **Absent:** unlocking, covering the display and relocking — this runtime never
  asks for a password and never hides a screen — and the built-in browser and
  native `@App` picker, which are client surfaces.

## 5. Distribution

*Notarization, pinned runtime provenance/version, CI architecture coverage,
installer/update integration, stable signing/grants across updates.*

- **Provenance: implemented here.** `install.sh` writes `.oxide-computer-use-install`
  beside the binary with the release, version, platform and the installed
  binary's SHA-256, and `doctor --json` reports it as `provenance.install` along
  with the binary that is running, the target it was built for and whether it is
  a debug or release build. A client that bundles the runtime can report which
  build it started. See [development](development.md#provenance).
- **CI architecture coverage: implemented here.** `ci.yml` builds the release
  binary for `aarch64-apple-darwin` and `x86_64-apple-darwin` on every change,
  using the same commands the release workflow uses, instead of discovering a
  broken target at tag time.
- **Installer update check: implemented here.** `install.sh --check` compares the
  marker's version with the latest release's manifest and exits 0, 1 or 2 without
  downloading an archive, and `tests/install.sh` runs all four outcomes offline in
  CI. Applying an update is re-running the installer, which rewrites the marker
  from what it installed.
- **Absent:** notarization. This repository ships unsigned archives (the helper's
  app bundle is ad-hoc signed by its build script). Stable signing and surviving
  macOS grants across updates stay unfinished, and are the reason the installer
  only verifies a checksum.

## 6. Live verification

*System Settings permission entries and prompts, granted screenshots and input,
visual UI QA, helper-crash recovery, user takeover, other CPU and OS versions,
Windows runtime support.*

Everything written on this page that touches the GUI is compiled and unit-tested.
The rows below say which paths an ungranted machine could still prove — the
window server, a live bundle identity, and the read bound the system accepts —
and which need a grant, a second machine or a client:

| Behaviour | Status |
| --- | --- |
| Permission prompts and System Settings entries | Unverified; the developer machine has neither grant, so `doctor` reports both missing. |
| A live bundle identity from a running app | **Measured without a grant:** `verify` printed `bundle identity ok: com.tencent.tenvideo at /Applications/QQLive.app`, resolving a running pid's identifier and bundle path. The pure path/bundle logic and the missing-bundle path stay unit-tested. |
| A read from an app that stops answering | Half measured: `verify` printed `read bound ok: 2 seconds per message accepted for pid 5424`, so the system takes the bound this backend sets on every app element. That a wedged app then fails the call instead of returning an empty tree still needs the grant and a stopped app (recipe in [development](development.md#manual-smoke-checklist)). |
| The takeover tap's callback | The tap installs on an ungranted machine (measured), but whether it receives a person's events needs one; the tests cover the comparison and the mask, and `verify` reports the install. |
| Background delivery (`CGEventPostToPid`) | Unverified; the argument handling and the reply are tested, but whether a given app accepts posted-to-pid keys needs the grant and a list of apps. |
| The screen resource | The protocol path is tested end to end (list, read, subscribe, notification); the capture itself needs Screen Recording, so a read here returns the permission error. |
| Refusing the hosting app | Unverified end to end; the comparison is unit-tested, but a real hosting bundle needs a client that launches the server as a child of an `.app`. |
| Granted screenshots and synthetic input | Unverified; every path here fails with the permission error on an ungranted machine. |
| Visual UI QA of any reply | Not done. |
| Helper-crash recovery and user takeover in the helper | User takeover is implemented here (§1). Recovery is split between the two sides: a tool call that panics is answered as an `isError` result and the loop keeps serving (unit-tested), and a read from an app that stops answering fails after the messaging timeout rather than rendering an empty window. A server process that is *killed* is still the client's to restart — nothing in this repository supervises it. |
| Other CPU, macOS and display configurations | Only the Apple Silicon build on the development machine was run; CI compiles both architectures. |
| Windows runtime | Absent by design: another platform builds the same tool list over a stub that returns an explicit "not implemented here" error. |

The commands that close each gap are in
[development](development.md#manual-smoke-checklist).

## 7. Session reliability

*The 11:59 tool-discovery mismatch, and the literal backslash-n navigation
attempts.*

- **Backslash-n: addressed here.** `press_key` now reads `\n`, `\r`, `\t`,
  `newline`, `linefeed`/`lf`, `cr` and the real control characters as `Return`
  and `Tab`, and reports back the key it actually pressed. `type_text` presses a
  newline in its text as `Return` — an app that receives one inside a keyboard
  event usually inserts nothing — and counts the line breaks it sent. Text that
  contains a literal `\n` is typed as written and the reply says so, pointing at
  `press_key`. An unknown key now names the escape spellings in its error, so a
  retry has somewhere to go.
- **Tool discovery: partially addressed here.** The server cannot fix a client's
  search index, but it can match the words an agent searches for: every tool
  description now names the GUI surface it acts on (desktop app, browser window,
  element, click, keyboard, mouse, screenshot), `initialize` lists every tool
  name and the policy commands in its instructions, and `tools --json` prints the
  full surface for a person to inspect. Tests hold both — a description that
  loses its discovery vocabulary, or instructions that stop naming a tool, fail
  the build.
- **Still open:** *why* the 11:59 session's tool search returned unrelated MCP
  integrations is not established. Nothing in this repository can reproduce it,
  and a fix that assumes a cause would be a guess.

## 8. Current macOS window API

The original seven follow-ups did not cover the tool-level API that Codex's
installed Computer Use runtime now exposes. The macOS window surface is now
matched operation for operation:

- `list_apps` discovers installed applications as well as running bundles and
  returns `id`, `displayName`, `isRunning` and path. `app` accepts a bundle id,
  display/process name, application path, pid or client-style `@App` mention.
- `get_app_state` launches a discoverable app when necessary, waits for its
  window, returns an accessibility diff after the first view unless `disableDiff`
  is set, and attaches the app-window screenshot by default.
- `click` supports element or app-window coordinates, left/right/middle buttons,
  click count, held modifier chords and hold duration. Coordinates account for
  the window origin, Retina scale and PNG dimensions.
- `press_key` reads xdotool-style chords including `Super+c`, `Control_L+a` and
  `KP_0`, plus modifier, navigation, keypad and punctuation keysyms, and accepts
  a hold duration.
- `scroll` accepts all four directions and page distance; `drag` uses two
  app-window coordinates with paced HID events.
- `select_text` supports prefix/suffix disambiguation and `cursor_before` /
  `cursor_after`; `paste` supports text, Markdown and HTML and restores every
  previous clipboard data type.
- `type_text`, `set_value` and `perform_secondary_action` retain their existing
  verified-write and action support, including background keyboard delivery as
  an oxide-computer-use extension.
- Observations settle after actions: `get_app_state` waits roughly one second,
  then follows busy/progress indicators for at most five seconds.
- Selected common apps receive independently authored, once-per-session
  app-specific guidance; proprietary Codex instruction text is not copied.

The transport is still MCP rather than Codex's private JavaScript facade, and the
Windows runtime, locked-use flow, settings UI and optional audio recording remain
outside a macOS stdio server. Those are platform/product boundaries rather than
missing macOS window operations.

## What this work does not change

- The MCP revision negotiation is unchanged. Existing tool names are preserved;
  the current Computer Use parameter names were added and `paste` joined the
  surface.
- The existing allow/deny file keeps working: an allow list with entries still
  refuses everything it does not name, which is now spelled `app_policy: "deny"`.
- Nothing here approves a macOS permission, sends input at startup, or prompts
  the person. A refusal names the command that changes it and stops.
